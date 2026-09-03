//! EOS service: long-running platform handle plus Connect interface.
//!
//! The anti-cheat path obtains an `EOS_ProductUserId` by passing the configured
//! `OpenID` token to EOS Connect.

use crate::{error::EosError, settings::EosSettings};
use eos_ffi::{
    EOS_ELogCategory, EOS_ELogLevel, EOS_EResult, EOS_HPlatform, EOS_INITIALIZE_API_LATEST,
    EOS_Initialize, EOS_InitializeOptions, EOS_LogMessage, EOS_Logging_SetCallback,
    EOS_Logging_SetLogLevel, EOS_PLATFORM_OPTIONS_API_LATEST, EOS_Platform_Create,
    EOS_Platform_Options, EOS_Platform_Release, EOS_Platform_Tick, EOS_Shutdown,
};
// The Connect and AntiCheat entry points below are referenced only from the
// feature-gated runtime paths in this file, and had drifted out of the list
// above because nothing ever compiled them: `--all-targets` on default
// features never enters these cfgs, so a missing name is never reported.
// Building with `--all-features` (which clippy now does) is what keeps these
// two lists honest. `anti-cheat-client` implies `client-runtime`, so the
// Connect names — including `EOS_ProductUserId` and `EOS_Connect_Login`, which
// both feature paths use — need only the narrower gate to cover both.
#[cfg(feature = "client-runtime")]
use eos_ffi::{
    EOS_CONNECT_CREATEUSER_API_LATEST, EOS_CONNECT_CREDENTIALS_API_LATEST,
    EOS_CONNECT_LOGIN_API_LATEST, EOS_CONNECT_USERLOGININFO_API_LATEST, EOS_Connect_CreateUser,
    EOS_Connect_CreateUserCallbackInfo, EOS_Connect_CreateUserOptions, EOS_Connect_Credentials,
    EOS_Connect_Login, EOS_Connect_LoginCallbackInfo, EOS_Connect_LoginOptions,
    EOS_Connect_UserLoginInfo, EOS_ContinuanceToken, EOS_EExternalCredentialType, EOS_HConnect,
    EOS_Platform_GetConnectInterface, EOS_ProductUserId,
};
#[cfg(feature = "anti-cheat-client")]
use eos_ffi::{EOS_HAntiCheatClient, EOS_Platform_GetAntiCheatClientInterface};
use std::ffi::{CStr, CString};
#[cfg(feature = "client-runtime")]
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;
use std::sync::Mutex;
#[cfg(feature = "client-runtime")]
use std::sync::mpsc;
use std::thread::{self, ThreadId};
use tracing::{debug, error, info, warn};

const PRODUCT_NAME: &str = "az-rs";
const PRODUCT_VERSION: &str = "0.1.0";

/// EOS Service resource: holds the long-running platform handle and the
/// `EOS_ProductUserId` from Connect login (when available).
///
/// EOS handles are owned by the thread that creates the platform. `EosPlugin`
/// stores this as a Bevy non-send resource, and SDK-touching methods also
/// verify they are running on the owner thread before calling into EOS.
pub struct EosService {
    inner: Rc<EosInner>,
}

pub(crate) struct EosInner {
    platform: Mutex<EOS_HPlatform>,
    /// Set after a successful `connect_login` (or after CreateUser+retry).
    #[cfg(feature = "client-runtime")]
    local_user_id: Mutex<Option<EOS_ProductUserId>>,
    owner_thread: ThreadId,
}

impl EosService {
    /// Initialize the EOS client SDK and create a single long-running platform.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new_with_role`] returns:
    /// [`EosError::InvalidConfiguration`] if the product name/version or any of
    /// the product, sandbox, or deployment ids contain an interior NUL,
    /// [`EosError::InitializationFailed`] if `EOS_Initialize` reports anything
    /// other than success or already-configured, and
    /// [`EosError::PlatformCreationFailed`] if `EOS_Platform_Create` returns a
    /// null handle.
    pub fn new(settings: &EosSettings) -> Result<Self, EosError> {
        Self::new_with_role(settings, false)
    }

    /// Initialize the EOS server SDK and create a single long-running platform.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::new`]; the server role only changes
    /// whether missing client credentials are warned about, not which failures
    /// are reported.
    pub fn new_server(settings: &EosSettings) -> Result<Self, EosError> {
        Self::new_with_role(settings, true)
    }

    fn new_with_role(settings: &EosSettings, is_server: bool) -> Result<Self, EosError> {
        info!("Initializing EOS SDK...");

        let product_name = CString::new(PRODUCT_NAME)
            .map_err(|_| EosError::InvalidConfiguration("Invalid product name".into()))?;
        let product_version = CString::new(PRODUCT_VERSION)
            .map_err(|_| EosError::InvalidConfiguration("Invalid product version".into()))?;

        // `..Default::default()` below is required when build.rs finds a real
        // SDK: bindgen then emits the full EOS_InitializeOptions (allocator
        // hooks, reserved, thread affinity), and only the no-SDK fallback
        // struct is exhaustively covered by the fields named here.
        #[allow(clippy::needless_update)]
        let init_opts = EOS_InitializeOptions {
            // The bindings publish API-version constants as `u32` but the
            // options structs take `i32`. Every published value is a small
            // positive version number, so reinterpreting the bits is exact.
            ApiVersion: EOS_INITIALIZE_API_LATEST.cast_signed(),
            ProductName: product_name.as_ptr(),
            ProductVersion: product_version.as_ptr(),
            ..Default::default()
        };

        let result = unsafe { EOS_Initialize(&raw const init_opts) };
        if result != EOS_EResult::EOS_Success && result != EOS_EResult::EOS_AlreadyConfigured {
            return Err(EosError::InitializationFailed(format!("{result:?}")));
        }

        info!("EOS SDK initialized (status: {:?})", result);

        install_log_callback();

        let product_id = CString::new(settings.product_id.as_str()).map_err(|e| {
            EosError::InvalidConfiguration(format!("Invalid EOS_PRODUCT_ID: {e:?}"))
        })?;
        let sandbox_id = CString::new(settings.sandbox_id.as_str()).map_err(|e| {
            EosError::InvalidConfiguration(format!("Invalid EOS_SANDBOX_ID: {e:?}"))
        })?;
        let deployment_id = CString::new(settings.deployment_id.as_str()).map_err(|e| {
            EosError::InvalidConfiguration(format!("Invalid EOS_DEPLOYMENT_ID: {e:?}"))
        })?;

        let client_id_cstr = settings
            .client_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| CString::new(s).ok());
        let client_secret_cstr = settings
            .client_secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| CString::new(s).ok());

        if !is_server && (client_id_cstr.is_none() || client_secret_cstr.is_none()) {
            warn!(
                "EOS_CLIENT_ID/EOS_CLIENT_SECRET not detected — Connect_Login will likely fail. \
                 Supply them through EosSettings or the launch environment."
            );
        }

        let cache_dir = std::env::temp_dir().join("eos-cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let cache_dir_cstr = CString::new(cache_dir.to_string_lossy().as_ref()).ok();

        let mut platform_opts: EOS_Platform_Options = unsafe { std::mem::zeroed() };
        // Same u32-constant / i32-field mismatch as in `init_opts` above.
        platform_opts.ApiVersion = EOS_PLATFORM_OPTIONS_API_LATEST.cast_signed();
        platform_opts.ProductId = product_id.as_ptr();
        platform_opts.SandboxId = sandbox_id.as_ptr();
        platform_opts.DeploymentId = deployment_id.as_ptr();
        platform_opts.ClientCredentials.ClientId =
            client_id_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        platform_opts.ClientCredentials.ClientSecret = client_secret_cstr
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        platform_opts.bIsServer = i32::from(is_server);
        platform_opts.Flags = 0;
        platform_opts.CacheDirectory = cache_dir_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());

        let platform = unsafe { EOS_Platform_Create(&raw const platform_opts) };
        if platform.is_null() {
            unsafe { EOS_Shutdown() };
            return Err(EosError::PlatformCreationFailed);
        }

        info!("EOS platform created");

        Ok(Self {
            inner: Rc::new(EosInner {
                platform: Mutex::new(platform),
                #[cfg(feature = "client-runtime")]
                local_user_id: Mutex::new(None),
                owner_thread: thread::current().id(),
            }),
        })
    }

    fn is_owner_thread(&self) -> bool {
        thread::current().id() == self.inner.owner_thread
    }

    fn warn_if_wrong_thread(&self, operation: &str) -> bool {
        if self.is_owner_thread() {
            return false;
        }
        error!("EOS {operation} called off the owning thread; SDK call skipped");
        true
    }

    /// Tick the platform — must run from the main thread every frame.
    ///
    /// # Panics
    ///
    /// Panics if the platform-handle mutex is poisoned, which happens only if a
    /// previous SDK call panicked while holding it.
    pub fn tick(&self) {
        if self.warn_if_wrong_thread("Platform_Tick") {
            return;
        }
        let platform = *self.inner.platform.lock().unwrap();
        unsafe { EOS_Platform_Tick(platform) };
    }

    /// Raw platform handle. Only safe to use from the main thread.
    ///
    /// # Panics
    ///
    /// Panics if the platform-handle mutex is poisoned, which happens only if a
    /// previous SDK call panicked while holding it.
    #[must_use]
    pub fn platform(&self) -> EOS_HPlatform {
        if self.warn_if_wrong_thread("platform handle access") {
            return ptr::null_mut();
        }
        *self.inner.platform.lock().unwrap()
    }

    /// Anti-cheat client interface. NULL until the platform finishes
    /// initialization or if the deployment doesn't have `AntiCheat` enabled.
    #[cfg(feature = "anti-cheat-client")]
    #[must_use]
    pub fn anti_cheat_client(&self) -> EOS_HAntiCheatClient {
        unsafe { EOS_Platform_GetAntiCheatClientInterface(self.platform()) }
    }

    /// Connect interface, used for `EOS_Connect_Login` to obtain a
    /// `EOS_ProductUserId`.
    #[cfg(feature = "client-runtime")]
    #[must_use]
    pub fn connect(&self) -> EOS_HConnect {
        unsafe { EOS_Platform_GetConnectInterface(self.platform()) }
    }

    /// Local product user id from a successful `connect_login` (or post-
    /// `CreateUser` retry).
    ///
    /// # Panics
    ///
    /// Panics if the local-user-id mutex is poisoned, which happens only if a
    /// previous call panicked while holding it.
    #[cfg(feature = "client-runtime")]
    #[must_use]
    pub fn local_user_id(&self) -> Option<EOS_ProductUserId> {
        *self.inner.local_user_id.lock().unwrap()
    }

    /// Begin an `EOS_Connect_Login` flow on the **main thread**.
    ///
    /// The main thread is the only one that may touch the platform. Returns a
    /// receiver that the caller polls each frame; the SDK callback fires from
    /// `EOS_Platform_Tick`, which is also driven on the main thread.
    ///
    /// `external_type` selects the identity provider; the appropriate
    /// `token` format depends on the provider (see `eos_common.h`
    /// `EOS_EExternalCredentialType` doc comments). Most relevant here:
    /// - `EOS_ECT_OPENID_ACCESS_TOKEN` → AGS tokenservice JWT
    ///
    /// Why no worker thread: `EOS_Platform_Tick` is not safe to call
    /// concurrently from multiple threads — doing so silently aborts the
    /// process inside the SDK. A previous implementation drove tick from
    /// a polling loop in an `IoTaskPool` task and crashed within ~17 ms
    /// of the first `EOS_Connect_Login` call, before the SDK could even
    /// emit a log line.
    ///
    /// The returned `ConnectLoginPending` keeps the heap-allocated
    /// `CString`s alive — the SDK retains the `Token`/`DisplayName`
    /// pointers across `Tick` calls until the callback fires.
    ///
    /// # Errors
    ///
    /// Returns [`EosError::AuthInterfaceUnavailable`] if the platform has not
    /// yet published a Connect interface, or
    /// [`EosError::InvalidSteamTicket`] if `token` contains an interior NUL and
    /// so cannot be handed to the SDK as a C string.
    ///
    /// # Panics
    ///
    /// Panics if `PRODUCT_NAME` ever gains an interior NUL byte; it is a
    /// NUL-free literal, so the `DisplayName` conversion cannot fail at
    /// runtime.
    #[cfg(feature = "client-runtime")]
    pub fn begin_connect_login(
        &self,
        external_type: EOS_EExternalCredentialType::Type,
        token: &str,
    ) -> Result<ConnectLoginPending, EosError> {
        let connect = self.connect();
        if connect.is_null() {
            return Err(EosError::AuthInterfaceUnavailable);
        }
        let token = CString::new(token)
            .map_err(|e| EosError::InvalidSteamTicket(format!("Token contains NUL: {e:?}")))?;
        // `EOS_Connect_UserLoginInfo` is required for several external
        // providers (Amazon/Apple/Google/Nintendo/Oculus/DeviceID); for
        // others (Steam, Epic) it can be NULL. We pass NULL whenever we
        // can to avoid surfacing a placeholder DisplayName to EOS.
        let needs_user_info = external_type
            == EOS_EExternalCredentialType::EOS_ECT_AMAZON_ACCESS_TOKEN
            || external_type == EOS_EExternalCredentialType::EOS_ECT_APPLE_ID_TOKEN
            || external_type == EOS_EExternalCredentialType::EOS_ECT_GOOGLE_ID_TOKEN
            || external_type == EOS_EExternalCredentialType::EOS_ECT_NINTENDO_ID_TOKEN
            || external_type == EOS_EExternalCredentialType::EOS_ECT_NINTENDO_NSA_ID_TOKEN
            || external_type == EOS_EExternalCredentialType::EOS_ECT_OCULUS_USERID_NONCE
            || external_type == EOS_EExternalCredentialType::EOS_ECT_DEVICEID_ACCESS_TOKEN;
        let display = needs_user_info.then(|| CString::new(PRODUCT_NAME).unwrap());
        let (tx, rx) = mpsc::channel();
        let ctx_box = Box::into_raw(Box::new(tx)).cast::<c_void>();

        // Same u32-constant / i32-field mismatch as in `new_with_role` above:
        // every published API version is a small positive number, so
        // reinterpreting the bits is exact.
        let credentials = EOS_Connect_Credentials {
            ApiVersion: EOS_CONNECT_CREDENTIALS_API_LATEST.cast_signed(),
            Token: token.as_ptr(),
            Type: external_type,
        };
        let user_info = display.as_ref().map(|d| EOS_Connect_UserLoginInfo {
            ApiVersion: EOS_CONNECT_USERLOGININFO_API_LATEST.cast_signed(),
            DisplayName: d.as_ptr(),
            NsaIdToken: ptr::null(),
        });
        let login_opts = EOS_Connect_LoginOptions {
            ApiVersion: EOS_CONNECT_LOGIN_API_LATEST.cast_signed(),
            Credentials: &raw const credentials,
            UserLoginInfo: user_info.as_ref().map_or(ptr::null(), ptr::from_ref),
        };
        unsafe {
            EOS_Connect_Login(
                connect,
                &raw const login_opts,
                ctx_box,
                Some(connect_login_cb),
            );
        }
        debug!(
            "EOS_Connect_Login dispatched (type={:?}, token_len={})",
            external_type,
            token.as_bytes().len()
        );
        Ok(ConnectLoginPending {
            rx,
            ctx_box,
            _token: token,
            _display: display,
        })
    }

    /// Same lifetime contract as [`begin_connect_login`]: must run
    /// on the main thread; the returned receiver is polled each frame.
    ///
    /// # Errors
    ///
    /// Returns [`EosError::AuthInterfaceUnavailable`] if the platform has not
    /// yet published a Connect interface. Dispatch itself cannot fail: any
    /// rejection of `continuance` is reported through the returned receiver.
    #[cfg(feature = "client-runtime")]
    pub fn begin_connect_create_user(
        &self,
        continuance: EOS_ContinuanceToken,
    ) -> Result<ConnectCreateUserPending, EosError> {
        let connect = self.connect();
        if connect.is_null() {
            return Err(EosError::AuthInterfaceUnavailable);
        }
        // u32 constant into an i32 field; see `begin_connect_login` above.
        let opts = EOS_Connect_CreateUserOptions {
            ApiVersion: EOS_CONNECT_CREATEUSER_API_LATEST.cast_signed(),
            ContinuanceToken: continuance,
        };
        let (tx, rx) = mpsc::channel();
        let ctx_box = Box::into_raw(Box::new(tx)).cast::<c_void>();
        unsafe {
            EOS_Connect_CreateUser(
                connect,
                &raw const opts,
                ctx_box,
                Some(connect_create_user_cb),
            );
        }
        debug!("EOS_Connect_CreateUser dispatched");
        Ok(ConnectCreateUserPending { rx, ctx_box })
    }

    /// Record a successful login result on the service.
    #[cfg(feature = "client-runtime")]
    pub(crate) fn set_local_user_id(&self, puid: EOS_ProductUserId) {
        *self.inner.local_user_id.lock().unwrap() = Some(puid);
    }
}

/// In-flight `EOS_Connect_Login` request. Drop while the callback is
/// still pending leaks the channel sender and any held `CString`s.
#[cfg(feature = "client-runtime")]
pub struct ConnectLoginPending {
    pub(crate) rx: mpsc::Receiver<Result<ConnectLoginOutcome, EosError>>,
    ctx_box: *mut c_void,
    _token: CString,
    _display: Option<CString>,
}

#[cfg(feature = "client-runtime")]
impl Drop for ConnectLoginPending {
    fn drop(&mut self) {
        // If the callback already fired the sender was already taken;
        // try_recv would have returned. Leak the box if unfired — the
        // SDK still holds the pointer and may dispatch later.
        let _ = self.ctx_box;
    }
}

/// In-flight `EOS_Connect_CreateUser` request.
#[cfg(feature = "client-runtime")]
pub struct ConnectCreateUserPending {
    pub(crate) rx: mpsc::Receiver<Result<EOS_ProductUserId, EosError>>,
    ctx_box: *mut c_void,
}

#[cfg(feature = "client-runtime")]
impl Drop for ConnectCreateUserPending {
    fn drop(&mut self) {
        let _ = self.ctx_box;
    }
}

#[cfg(feature = "client-runtime")]
#[derive(Debug)]
pub enum ConnectLoginOutcome {
    Success(EOS_ProductUserId),
    NeedsCreateUser(EOS_ContinuanceToken),
}

#[cfg(feature = "client-runtime")]
unsafe extern "C" fn connect_login_cb(data: *const EOS_Connect_LoginCallbackInfo) {
    if data.is_null() {
        return;
    }
    let info = unsafe { &*data };
    let tx_ptr = info
        .ClientData
        .cast::<mpsc::Sender<Result<ConnectLoginOutcome, EosError>>>();
    if tx_ptr.is_null() {
        return;
    }
    let tx = unsafe { Box::from_raw(tx_ptr) };
    let outcome = match info.ResultCode {
        EOS_EResult::EOS_Success => Ok(ConnectLoginOutcome::Success(info.LocalUserId)),
        EOS_EResult::EOS_InvalidUser => {
            Ok(ConnectLoginOutcome::NeedsCreateUser(info.ContinuanceToken))
        }
        code => Err(EosError::LoginFailed(format!("Connect_Login: {code:?}"))),
    };
    let _ = tx.send(outcome);
}

#[cfg(feature = "client-runtime")]
unsafe extern "C" fn connect_create_user_cb(data: *const EOS_Connect_CreateUserCallbackInfo) {
    if data.is_null() {
        return;
    }
    let info = unsafe { &*data };
    let tx_ptr = info
        .ClientData
        .cast::<mpsc::Sender<Result<EOS_ProductUserId, EosError>>>();
    if tx_ptr.is_null() {
        return;
    }
    let tx = unsafe { Box::from_raw(tx_ptr) };
    let result = if info.ResultCode == EOS_EResult::EOS_Success {
        Ok(info.LocalUserId)
    } else {
        Err(EosError::LoginFailed(format!(
            "Connect_CreateUser: {:?}",
            info.ResultCode
        )))
    };
    let _ = tx.send(result);
}

fn install_log_callback() {
    // Targets default to the calling module path (`eos_plugin::service`)
    // so the registered-module filter picks them up without a custom
    // entry. Override via `RUST_LOG=eos_plugin=trace` to see SDK logs.
    unsafe extern "C" fn cb(msg: *const EOS_LogMessage) {
        if msg.is_null() {
            return;
        }
        let m = unsafe { &*msg };
        let category = unsafe { CStr::from_ptr(m.Category).to_string_lossy() };
        let text = unsafe { CStr::from_ptr(m.Message).to_string_lossy() };
        match m.Level {
            EOS_ELogLevel::EOS_LOG_Fatal | EOS_ELogLevel::EOS_LOG_Error => {
                error!("[eos:{}] {}", category, text);
            }
            EOS_ELogLevel::EOS_LOG_Warning => {
                warn!("[eos:{}] {}", category, text);
            }
            EOS_ELogLevel::EOS_LOG_Info => {
                info!("[eos:{}] {}", category, text);
            }
            _ => {
                debug!("[eos:{}] {}", category, text);
            }
        }
    }
    unsafe {
        let _ = EOS_Logging_SetCallback(Some(cb));
        let _ = EOS_Logging_SetLogLevel(
            EOS_ELogCategory::EOS_LC_ALL_CATEGORIES,
            EOS_ELogLevel::EOS_LOG_Verbose,
        );
    }
}

impl Drop for EosService {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) > 1 {
            return;
        }
        info!("Shutting down EOS platform...");
        let platform = *self.inner.platform.lock().unwrap();
        unsafe {
            EOS_Platform_Release(platform);
            EOS_Shutdown();
        }
    }
}
