//! EOS Anti-Cheat client wrapper.
//!
//! Wraps `EOS_AntiCheatClient_BeginSession`, `_AddNotifyMessageToServer`,
//! `_ReceiveMessageFromServer`, and `_EndSession`. The SDK fires its
//! `OnMessageToServer` callback from `EOS_Platform_Tick` (per
//! `eos_anticheatclient_types.h`), so the queue is drained on the main thread.
//!
//! Wire bridge:
//! - server → us: `EosAntiCheatClientTraitUpdateMsg` (type 6484) →
//!   `receive_from_server(bytes)` → SDK validates internally
//! - us → server: SDK callback fires → `take_outbound()` drains →
//!   the host's drain system emits its anti-cheat transport message (type 6483)

use crate::error::EosError;
use crate::service::EosService;
// Spelled out rather than globbed: this whole module lives behind
// `anti-cheat-client`, so a stale name here is invisible to any build that
// does not enable the feature.
use eos_ffi::{
    EOS_ANTICHEATCLIENT_ADDNOTIFYCLIENTINTEGRITYVIOLATED_API_LATEST,
    EOS_ANTICHEATCLIENT_ADDNOTIFYMESSAGETOSERVER_API_LATEST,
    EOS_ANTICHEATCLIENT_BEGINSESSION_API_LATEST, EOS_ANTICHEATCLIENT_ENDSESSION_API_LATEST,
    EOS_ANTICHEATCLIENT_RECEIVEMESSAGEFROMSERVER_API_LATEST,
    EOS_AntiCheatClient_AddNotifyClientIntegrityViolated,
    EOS_AntiCheatClient_AddNotifyClientIntegrityViolatedOptions,
    EOS_AntiCheatClient_AddNotifyMessageToServer,
    EOS_AntiCheatClient_AddNotifyMessageToServerOptions, EOS_AntiCheatClient_BeginSession,
    EOS_AntiCheatClient_BeginSessionOptions, EOS_AntiCheatClient_EndSession,
    EOS_AntiCheatClient_EndSessionOptions,
    EOS_AntiCheatClient_OnClientIntegrityViolatedCallbackInfo,
    EOS_AntiCheatClient_OnMessageToServerCallbackInfo,
    EOS_AntiCheatClient_ReceiveMessageFromServer,
    EOS_AntiCheatClient_ReceiveMessageFromServerOptions,
    EOS_AntiCheatClient_RemoveNotifyClientIntegrityViolated,
    EOS_AntiCheatClient_RemoveNotifyMessageToServer, EOS_EAntiCheatClientMode, EOS_EResult,
    EOS_HAntiCheatClient, EOS_NotificationId, EOS_ProductUserId,
};
use std::collections::VecDeque;
use std::os::raw::c_void;
use std::ptr;
use std::sync::{Mutex, OnceLock};
use std::thread::{self, ThreadId};
use tracing::{debug, error, info, warn};

/// Per-process outbound queue. The SDK callback writes here; the Bevy
/// drain system pops. Static because there is at most one anti-cheat
/// session per process, and the SDK callback signature has no closure.
fn outbound_queue() -> &'static Mutex<VecDeque<Vec<u8>>> {
    static Q: OnceLock<Mutex<VecDeque<Vec<u8>>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Non-send Bevy resource that owns the current anti-cheat session, if one is
/// active.
///
/// Keeping the optional slot alive lets other plugins reset the session without
/// dynamically inserting non-send resources through `Commands`.
#[derive(Default)]
pub struct AntiCheatClientState {
    service: Option<AntiCheatService>,
}

impl AntiCheatClientState {
    pub fn is_active(&self) -> bool {
        self.service
            .as_ref()
            .is_some_and(AntiCheatService::is_active)
    }

    /// Replace any current session with a fresh one for `local_user_id`.
    ///
    /// # Errors
    ///
    /// Returns [`EosError::AuthInterfaceUnavailable`] if the platform does not
    /// expose the anti-cheat client interface, plus any error
    /// [`AntiCheatService::start`] returns.
    pub fn start(
        &mut self,
        eos: &EosService,
        local_user_id: EOS_ProductUserId,
    ) -> Result<(), EosError> {
        if self.is_active() {
            return Ok(());
        }
        self.end();

        let Some(mut service) = AntiCheatService::new(eos) else {
            return Err(EosError::AuthInterfaceUnavailable);
        };
        service.start(local_user_id)?;
        self.service = Some(service);
        Ok(())
    }

    pub fn receive_from_server(&self, bytes: &[u8]) {
        if let Some(service) = self.service.as_ref().filter(|service| service.is_active()) {
            service.receive_from_server(bytes);
        } else {
            debug!(
                "anti-cheat receive buffered/ignored by caller: session not active ({} bytes)",
                bytes.len()
            );
        }
    }

    pub fn take_outbound(&self) -> Vec<Vec<u8>> {
        self.service
            .as_ref()
            .filter(|service| service.is_active())
            .map(AntiCheatService::take_outbound)
            .unwrap_or_default()
    }

    pub fn end(&mut self) {
        if let Some(service) = self.service.as_mut() {
            service.end();
        }
        self.service = None;
        AntiCheatService::clear_outbound_queue();
    }
}

impl Drop for AntiCheatClientState {
    fn drop(&mut self) {
        self.end();
    }
}

/// `AntiCheat` client session. Lifecycle: `start(local_user_id)` once after
/// `EOS_Connect_Login` succeeds; receive/send while gridmate session is
/// open; `end()` (or Drop) when leaving the world.
pub struct AntiCheatService {
    handle: EOS_HAntiCheatClient,
    notification_id: EOS_NotificationId,
    integrity_notification_id: EOS_NotificationId,
    session_active: bool,
    owner_thread: ThreadId,
}

impl AntiCheatService {
    /// Acquire the anti-cheat client handle from the platform. Returns
    /// `None` if the platform doesn't expose the interface (e.g. the
    /// EOS deployment doesn't have `AntiCheat` enabled).
    pub fn new(eos: &EosService) -> Option<Self> {
        let handle = eos.anti_cheat_client();
        if handle.is_null() {
            debug!("EOS anti-cheat client interface unavailable");
            return None;
        }
        Some(Self {
            handle,
            notification_id: 0,
            integrity_notification_id: 0,
            session_active: false,
            owner_thread: thread::current().id(),
        })
    }

    fn is_owner_thread(&self) -> bool {
        thread::current().id() == self.owner_thread
    }

    fn warn_if_wrong_thread(&self, operation: &str) -> bool {
        if self.is_owner_thread() {
            return false;
        }
        error!("EOS anti-cheat {operation} called off the owning thread; SDK call skipped");
        true
    }

    /// Drop every message still waiting to go out to the server.
    ///
    /// # Panics
    ///
    /// Panics if the outbound-queue mutex is poisoned, which happens only if a
    /// previous queue operation panicked while holding it.
    pub fn clear_outbound_queue() {
        outbound_queue().lock().unwrap().clear();
    }

    /// Register the outbound-message callback and start a `ClientServer`
    /// session for the given product user.
    ///
    /// Must be called after `EOS_Connect_Login` returns the
    /// `EOS_ProductUserId`. Idempotent: re-calling does nothing if a
    /// session is already active.
    ///
    /// # Errors
    ///
    /// Returns [`EosError::LoginFailed`] if called off the thread that acquired
    /// the handle, or if `EOS_AntiCheatClient_BeginSession` reports anything
    /// other than success; [`EosError::AuthInterfaceUnavailable`] if
    /// `EOS_AntiCheatClient_AddNotifyMessageToServer` hands back the invalid
    /// notification id, which means the SDK refused the subscription.
    pub fn start(&mut self, local_user_id: EOS_ProductUserId) -> Result<(), EosError> {
        if self.warn_if_wrong_thread("BeginSession") {
            return Err(EosError::LoginFailed(
                "AntiCheatClient_BeginSession called off owning thread".into(),
            ));
        }
        if self.session_active {
            return Ok(());
        }
        Self::clear_outbound_queue();

        // Subscribe to outbound messages first so we don't drop early
        // SDK output between BeginSession and the callback registration.
        // The bindings publish API-version constants as `u32` but the options
        // structs take `i32`. Every published value is a small positive version
        // number, so reinterpreting the bits is exact. (Same throughout.)
        let notify_opts = EOS_AntiCheatClient_AddNotifyMessageToServerOptions {
            ApiVersion: EOS_ANTICHEATCLIENT_ADDNOTIFYMESSAGETOSERVER_API_LATEST.cast_signed(),
        };
        self.notification_id = unsafe {
            EOS_AntiCheatClient_AddNotifyMessageToServer(
                self.handle,
                &raw const notify_opts,
                ptr::null_mut(),
                Some(on_message_to_server),
            )
        };
        // EOS_INVALID_NOTIFICATIONID is a `((EOS_NotificationId)0)` macro
        // that bindgen does not emit; sentinel value is `0`.
        if self.notification_id == 0 {
            return Err(EosError::AuthInterfaceUnavailable);
        }

        // Optional integrity-violated callback — useful for diagnostics.
        let integ_opts = EOS_AntiCheatClient_AddNotifyClientIntegrityViolatedOptions {
            ApiVersion: EOS_ANTICHEATCLIENT_ADDNOTIFYCLIENTINTEGRITYVIOLATED_API_LATEST
                .cast_signed(),
        };
        self.integrity_notification_id = unsafe {
            EOS_AntiCheatClient_AddNotifyClientIntegrityViolated(
                self.handle,
                &raw const integ_opts,
                ptr::null_mut(),
                Some(on_integrity_violated),
            )
        };

        let begin_opts = EOS_AntiCheatClient_BeginSessionOptions {
            ApiVersion: EOS_ANTICHEATCLIENT_BEGINSESSION_API_LATEST.cast_signed(),
            LocalUserId: local_user_id,
            Mode: EOS_EAntiCheatClientMode::EOS_ACCM_ClientServer,
        };
        let result =
            unsafe { EOS_AntiCheatClient_BeginSession(self.handle, &raw const begin_opts) };
        if result != EOS_EResult::EOS_Success {
            unsafe {
                EOS_AntiCheatClient_RemoveNotifyMessageToServer(self.handle, self.notification_id);
                EOS_AntiCheatClient_RemoveNotifyClientIntegrityViolated(
                    self.handle,
                    self.integrity_notification_id,
                );
            }
            self.notification_id = 0;
            self.integrity_notification_id = 0;
            return Err(EosError::LoginFailed(format!(
                "AntiCheatClient_BeginSession: {result:?}"
            )));
        }
        self.session_active = true;
        info!("EOS anti-cheat session started (ClientServer)");
        Ok(())
    }

    /// Forward bytes received as `EosAntiCheatClientTraitUpdateMsg`
    /// (type 6484) into the SDK for validation.
    pub fn receive_from_server(&self, bytes: &[u8]) {
        if self.warn_if_wrong_thread("ReceiveMessageFromServer") {
            return;
        }
        if !self.session_active {
            debug!(
                "anti-cheat receive ignored: session not active ({} bytes)",
                bytes.len()
            );
            return;
        }
        // `DataLengthBytes` is a u32; the SDK has no way to accept anything
        // longer, so an oversized frame is dropped rather than truncated into a
        // length that would make the SDK read past the end of `bytes`.
        let Ok(length) = u32::try_from(bytes.len()) else {
            warn!(
                "anti-cheat receive dropped: message exceeds the SDK's u32 length field ({} bytes)",
                bytes.len()
            );
            return;
        };
        let opts = EOS_AntiCheatClient_ReceiveMessageFromServerOptions {
            ApiVersion: EOS_ANTICHEATCLIENT_RECEIVEMESSAGEFROMSERVER_API_LATEST.cast_signed(),
            DataLengthBytes: length,
            Data: bytes.as_ptr().cast::<c_void>(),
        };
        let result =
            unsafe { EOS_AntiCheatClient_ReceiveMessageFromServer(self.handle, &raw const opts) };
        if result != EOS_EResult::EOS_Success {
            warn!(
                "EOS_AntiCheatClient_ReceiveMessageFromServer: {:?} (size={})",
                result,
                bytes.len()
            );
        }
    }

    /// Drain the outbound queue. Each entry is a single message that
    /// must be wrapped in an `EosAntiCheatTraitUpdateMsg` (type 6483)
    /// and shipped to the server.
    ///
    /// # Panics
    ///
    /// Panics if the outbound-queue mutex is poisoned, which happens only if a
    /// previous queue operation panicked while holding it.
    #[must_use]
    pub fn take_outbound(&self) -> Vec<Vec<u8>> {
        let mut q = outbound_queue().lock().unwrap();
        if q.is_empty() {
            return Vec::new();
        }
        q.drain(..).collect()
    }

    /// Tear the session down. Idempotent.
    pub fn end(&mut self) {
        if self.warn_if_wrong_thread("EndSession") {
            return;
        }
        if !self.session_active {
            Self::clear_outbound_queue();
            return;
        }
        let opts = EOS_AntiCheatClient_EndSessionOptions {
            ApiVersion: EOS_ANTICHEATCLIENT_ENDSESSION_API_LATEST.cast_signed(),
        };
        unsafe {
            let _ = EOS_AntiCheatClient_EndSession(self.handle, &raw const opts);
            if self.notification_id != 0 {
                EOS_AntiCheatClient_RemoveNotifyMessageToServer(self.handle, self.notification_id);
            }
            if self.integrity_notification_id != 0 {
                EOS_AntiCheatClient_RemoveNotifyClientIntegrityViolated(
                    self.handle,
                    self.integrity_notification_id,
                );
            }
        }
        self.notification_id = 0;
        self.integrity_notification_id = 0;
        self.session_active = false;
        Self::clear_outbound_queue();
        info!("EOS anti-cheat session ended");
    }

    /// Whether `EOS_AntiCheatClient_BeginSession` has succeeded and the session
    /// has not been torn down since.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.session_active
    }
}

impl Drop for AntiCheatService {
    fn drop(&mut self) {
        self.end();
    }
}

unsafe extern "C" fn on_message_to_server(
    data: *const EOS_AntiCheatClient_OnMessageToServerCallbackInfo,
) {
    if data.is_null() {
        return;
    }
    let info = unsafe { &*data };
    if info.MessageData.is_null() || info.MessageDataSizeBytes == 0 {
        return;
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            info.MessageData.cast::<u8>(),
            info.MessageDataSizeBytes as usize,
        )
    };
    // Release the queue lock before logging: `debug!` can format and write for
    // a good while, and this callback runs inside `EOS_Platform_Tick`.
    let depth = {
        let mut q = outbound_queue().lock().unwrap();
        q.push_back(bytes.to_vec());
        q.len()
    };
    debug!(
        "anti-cheat outbound queued ({} bytes, queue depth {})",
        bytes.len(),
        depth
    );
}

unsafe extern "C" fn on_integrity_violated(
    data: *const EOS_AntiCheatClient_OnClientIntegrityViolatedCallbackInfo,
) {
    if data.is_null() {
        return;
    }
    let info = unsafe { &*data };
    let msg = if info.ViolationMessage.is_null() {
        std::borrow::Cow::Borrowed("(no message)")
    } else {
        unsafe { std::ffi::CStr::from_ptr(info.ViolationMessage) }.to_string_lossy()
    };
    error!(
        "EOS anti-cheat INTEGRITY VIOLATION: type={:?} msg={}",
        info.ViolationType, msg
    );
}
