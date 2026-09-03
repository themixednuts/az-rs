use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const NO_SDK_EOS_BINDINGS_RS: &str = r#"
// Generated fallback bindings used when the EOS SDK headers are unavailable.
use ::std::os::raw::{c_char, c_void};
use ::std::ptr;

pub type EOS_Bool = i32;
pub type EOS_HPlatform = *mut c_void;
pub type EOS_HConnect = *mut c_void;
pub type EOS_HAntiCheatClient = *mut c_void;
pub type EOS_ProductUserId = *mut c_void;
pub type EOS_ContinuanceToken = *mut c_void;
pub type EOS_NotificationId = u64;

pub const EOS_INITIALIZE_API_LATEST: u32 = 1;
pub const EOS_PLATFORM_OPTIONS_API_LATEST: u32 = 1;
pub const EOS_CONNECT_CREDENTIALS_API_LATEST: u32 = 1;
pub const EOS_CONNECT_USERLOGININFO_API_LATEST: u32 = 1;
pub const EOS_CONNECT_LOGIN_API_LATEST: u32 = 1;
pub const EOS_CONNECT_CREATEUSER_API_LATEST: u32 = 1;
pub const EOS_ANTICHEATCLIENT_ADDNOTIFYMESSAGETOSERVER_API_LATEST: u32 = 1;
pub const EOS_ANTICHEATCLIENT_ADDNOTIFYCLIENTINTEGRITYVIOLATED_API_LATEST: u32 = 1;
pub const EOS_ANTICHEATCLIENT_BEGINSESSION_API_LATEST: u32 = 1;
pub const EOS_ANTICHEATCLIENT_RECEIVEMESSAGEFROMSERVER_API_LATEST: u32 = 1;
pub const EOS_ANTICHEATCLIENT_ENDSESSION_API_LATEST: u32 = 1;

pub mod EOS_EResult {
    pub type Type = i32;

    pub const EOS_Success: Type = 0;
    pub const EOS_AlreadyConfigured: Type = 1;
    pub const EOS_InvalidUser: Type = 3;
    pub const EOS_NotConfigured: Type = 17;
}

pub mod EOS_EExternalCredentialType {
    pub type Type = i32;

    pub const EOS_ECT_OPENID_ACCESS_TOKEN: Type = 9;
    pub const EOS_ECT_AMAZON_ACCESS_TOKEN: Type = 10;
    pub const EOS_ECT_APPLE_ID_TOKEN: Type = 11;
    pub const EOS_ECT_GOOGLE_ID_TOKEN: Type = 12;
    pub const EOS_ECT_NINTENDO_ID_TOKEN: Type = 13;
    pub const EOS_ECT_NINTENDO_NSA_ID_TOKEN: Type = 14;
    pub const EOS_ECT_OCULUS_USERID_NONCE: Type = 15;
    pub const EOS_ECT_DEVICEID_ACCESS_TOKEN: Type = 16;
}

pub mod EOS_EAntiCheatClientMode {
    pub type Type = i32;

    pub const EOS_ACCM_ClientServer: Type = 1;
}

pub mod EOS_EAntiCheatClientViolationType {
    pub type Type = i32;

    pub const EOS_ACCVT_Invalid: Type = 0;
}

pub mod EOS_ELogLevel {
    pub type Type = i32;

    pub const EOS_LOG_Off: Type = 0;
    pub const EOS_LOG_Fatal: Type = 100;
    pub const EOS_LOG_Error: Type = 200;
    pub const EOS_LOG_Warning: Type = 300;
    pub const EOS_LOG_Info: Type = 400;
    pub const EOS_LOG_Verbose: Type = 500;
}

pub mod EOS_ELogCategory {
    pub type Type = i32;

    pub const EOS_LC_ALL_CATEGORIES: Type = 0x7fff_ffff;
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_InitializeOptions {
    pub ApiVersion: i32,
    pub ProductName: *const c_char,
    pub ProductVersion: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Platform_ClientCredentials {
    pub ClientId: *const c_char,
    pub ClientSecret: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Platform_Options {
    pub ApiVersion: i32,
    pub ProductId: *const c_char,
    pub SandboxId: *const c_char,
    pub DeploymentId: *const c_char,
    pub ClientCredentials: EOS_Platform_ClientCredentials,
    pub bIsServer: EOS_Bool,
    pub Flags: u64,
    pub CacheDirectory: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_Credentials {
    pub ApiVersion: i32,
    pub Token: *const c_char,
    pub Type: EOS_EExternalCredentialType::Type,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_UserLoginInfo {
    pub ApiVersion: i32,
    pub DisplayName: *const c_char,
    pub NsaIdToken: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_LoginOptions {
    pub ApiVersion: i32,
    pub Credentials: *const EOS_Connect_Credentials,
    pub UserLoginInfo: *const EOS_Connect_UserLoginInfo,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_CreateUserOptions {
    pub ApiVersion: i32,
    pub ContinuanceToken: EOS_ContinuanceToken,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_LoginCallbackInfo {
    pub ResultCode: EOS_EResult::Type,
    pub ClientData: *mut c_void,
    pub LocalUserId: EOS_ProductUserId,
    pub ContinuanceToken: EOS_ContinuanceToken,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_Connect_CreateUserCallbackInfo {
    pub ResultCode: EOS_EResult::Type,
    pub ClientData: *mut c_void,
    pub LocalUserId: EOS_ProductUserId,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_AddNotifyMessageToServerOptions {
    pub ApiVersion: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_AddNotifyClientIntegrityViolatedOptions {
    pub ApiVersion: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_BeginSessionOptions {
    pub ApiVersion: i32,
    pub LocalUserId: EOS_ProductUserId,
    pub Mode: EOS_EAntiCheatClientMode::Type,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_ReceiveMessageFromServerOptions {
    pub ApiVersion: i32,
    pub DataLengthBytes: u32,
    pub Data: *const c_void,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_EndSessionOptions {
    pub ApiVersion: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_OnMessageToServerCallbackInfo {
    pub MessageData: *const c_void,
    pub MessageDataSizeBytes: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_AntiCheatClient_OnClientIntegrityViolatedCallbackInfo {
    pub ViolationType: EOS_EAntiCheatClientViolationType::Type,
    pub ViolationMessage: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EOS_LogMessage {
    pub Category: *const c_char,
    pub Message: *const c_char,
    pub Level: EOS_ELogLevel::Type,
}

pub unsafe fn EOS_Initialize(_options: *const EOS_InitializeOptions) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}

pub unsafe fn EOS_Shutdown() -> EOS_EResult::Type {
    EOS_EResult::EOS_Success
}

pub unsafe fn EOS_Platform_Create(_options: *const EOS_Platform_Options) -> EOS_HPlatform {
    ptr::null_mut()
}

pub unsafe fn EOS_Platform_Release(_platform: EOS_HPlatform) {}

pub unsafe fn EOS_Platform_Tick(_platform: EOS_HPlatform) {}

pub unsafe fn EOS_Platform_GetAntiCheatClientInterface(
    _platform: EOS_HPlatform,
) -> EOS_HAntiCheatClient {
    ptr::null_mut()
}

pub unsafe fn EOS_Platform_GetConnectInterface(_platform: EOS_HPlatform) -> EOS_HConnect {
    ptr::null_mut()
}

pub unsafe fn EOS_Logging_SetCallback(
    _callback: Option<unsafe extern "C" fn(*const EOS_LogMessage)>,
) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}

pub unsafe fn EOS_Logging_SetLogLevel(
    _category: EOS_ELogCategory::Type,
    _level: EOS_ELogLevel::Type,
) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}

pub unsafe fn EOS_Connect_Login(
    _handle: EOS_HConnect,
    _options: *const EOS_Connect_LoginOptions,
    _client_data: *mut c_void,
    _callback: Option<unsafe extern "C" fn(*const EOS_Connect_LoginCallbackInfo)>,
) {
}

pub unsafe fn EOS_Connect_CreateUser(
    _handle: EOS_HConnect,
    _options: *const EOS_Connect_CreateUserOptions,
    _client_data: *mut c_void,
    _callback: Option<unsafe extern "C" fn(*const EOS_Connect_CreateUserCallbackInfo)>,
) {
}

pub unsafe fn EOS_AntiCheatClient_AddNotifyMessageToServer(
    _handle: EOS_HAntiCheatClient,
    _options: *const EOS_AntiCheatClient_AddNotifyMessageToServerOptions,
    _client_data: *mut c_void,
    _notification_fn: Option<
        unsafe extern "C" fn(*const EOS_AntiCheatClient_OnMessageToServerCallbackInfo),
    >,
) -> EOS_NotificationId {
    0
}

pub unsafe fn EOS_AntiCheatClient_AddNotifyClientIntegrityViolated(
    _handle: EOS_HAntiCheatClient,
    _options: *const EOS_AntiCheatClient_AddNotifyClientIntegrityViolatedOptions,
    _client_data: *mut c_void,
    _notification_fn: Option<
        unsafe extern "C" fn(*const EOS_AntiCheatClient_OnClientIntegrityViolatedCallbackInfo),
    >,
) -> EOS_NotificationId {
    0
}

pub unsafe fn EOS_AntiCheatClient_RemoveNotifyMessageToServer(
    _handle: EOS_HAntiCheatClient,
    _notification_id: EOS_NotificationId,
) {
}

pub unsafe fn EOS_AntiCheatClient_RemoveNotifyClientIntegrityViolated(
    _handle: EOS_HAntiCheatClient,
    _notification_id: EOS_NotificationId,
) {
}

pub unsafe fn EOS_AntiCheatClient_BeginSession(
    _handle: EOS_HAntiCheatClient,
    _options: *const EOS_AntiCheatClient_BeginSessionOptions,
) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}

pub unsafe fn EOS_AntiCheatClient_ReceiveMessageFromServer(
    _handle: EOS_HAntiCheatClient,
    _options: *const EOS_AntiCheatClient_ReceiveMessageFromServerOptions,
) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}

pub unsafe fn EOS_AntiCheatClient_EndSession(
    _handle: EOS_HAntiCheatClient,
    _options: *const EOS_AntiCheatClient_EndSessionOptions,
) -> EOS_EResult::Type {
    EOS_EResult::EOS_NotConfigured
}
"#;

fn write_generated_file(output_path: &Path, contents: &str, label: &str) {
    fs::File::create(output_path)
        .and_then(|mut f| f.write_all(contents.as_bytes()))
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to write {label}: {e}");
        });
}

fn workspace_root() -> PathBuf {
    if let Ok(path) = env::var("CARGO_WORKSPACE_DIR") {
        return PathBuf::from(path);
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    manifest_dir
        .ancestors()
        .find(|path| path.join("Cargo.toml").is_file() && path.join("Cargo.lock").is_file())
        .map(Path::to_path_buf)
        .unwrap_or(manifest_dir)
}

/// The SDK headers `main` cares about, resolved under one `Include` directory.
///
/// `sdk` and `auth` are required; the rest are probed because older SDK drops
/// do not ship them.
struct SdkHeaders {
    include_dir: PathBuf,
    sdk: PathBuf,
    auth: PathBuf,
    logging: PathBuf,
    anticheat_client: PathBuf,
    connect: PathBuf,
}

impl SdkHeaders {
    fn locate(include_dir: PathBuf) -> Self {
        Self {
            sdk: include_dir.join("eos_sdk.h"),
            auth: include_dir.join("eos_auth.h"),
            logging: include_dir.join("eos_logging.h"),
            anticheat_client: include_dir.join("eos_anticheatclient.h"),
            connect: include_dir.join("eos_connect.h"),
            include_dir,
        }
    }

    fn emit_rerun_directives(&self) {
        println!("cargo:rerun-if-changed={}", self.sdk.to_string_lossy());
        println!("cargo:rerun-if-changed={}", self.auth.to_string_lossy());
        for optional in [&self.logging, &self.anticheat_client, &self.connect] {
            if optional.exists() {
                println!("cargo:rerun-if-changed={}", optional.to_string_lossy());
            }
        }
    }
}

/// Emits the link-search and DLL-copy directives for a located SDK.
fn emit_link_directives(lib_dir: &Path, bin_dir: &Path) {
    if lib_dir.exists() {
        println!(
            "cargo:rustc-link-search=native={}",
            lib_dir.to_string_lossy()
        );
        println!("cargo:rustc-link-lib=EOSSDK-Win64-Shipping");
    }

    copy_runtime_dll(bin_dir);
}

fn main() {
    let workspace_root = workspace_root();

    println!("cargo:rerun-if-env-changed=EOS_SDK_DIR");

    let eos_root_buf = env::var_os("EOS_SDK_DIR").map(PathBuf::from);
    let has_explicit_sdk = eos_root_buf.is_some();
    let bundled = find_bundled_eos_sdk(&workspace_root);
    let eos_root: &Path = eos_root_buf.as_deref().unwrap_or(&bundled);

    let lib_dir = eos_root.join("Lib");
    let bin_dir = eos_root.join("Bin");
    let headers = SdkHeaders::locate(eos_root.join("Include"));
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    if !headers.sdk.exists() {
        assert!(
            !has_explicit_sdk,
            "EOS_SDK_DIR does not contain Include/eos_sdk.h: {}",
            eos_root.display()
        );
        write_generated_file(
            &out_path.join("eos_bindings.rs"),
            NO_SDK_EOS_BINDINGS_RS,
            "EOS fallback bindings",
        );
        return;
    }

    headers.emit_rerun_directives();
    generate_bindings(&headers, &out_path);
    emit_link_directives(&lib_dir, &bin_dir);
}

/// Runs bindgen over the located headers and writes `eos_bindings.rs`.
fn generate_bindings(headers: &SdkHeaders, out_path: &Path) {
    let mut builder = bindgen::Builder::default()
        .rust_edition(bindgen::RustEdition::Edition2024)
        .header(headers.sdk.to_string_lossy())
        .header(headers.auth.to_string_lossy())
        .header(headers.logging.to_string_lossy())
        .clang_arg(format!("-I{}", headers.include_dir.to_string_lossy()))
        .allowlist_function("EOS_Auth_.*")
        .allowlist_function("EOS_Platform_.*")
        .allowlist_function("EOS_Logging_.*")
        .allowlist_function("EOS_AntiCheatClient_.*")
        .allowlist_function("EOS_AntiCheatCommon_.*")
        .allowlist_function("EOS_Connect_.*")
        .allowlist_function("EOS_ProductUserId_.*")
        .allowlist_function("EOS_ContinuanceToken_.*")
        .allowlist_function("EOS_Initialize")
        .allowlist_function("EOS_Shutdown")
        .allowlist_type("EOS_Auth_.*")
        .allowlist_type("EOS_Platform_.*")
        .allowlist_type("EOS_ELog.*")
        .allowlist_type("EOS_Log.*")
        .allowlist_type("EOS.*")
        .allowlist_var("EOS_.*_API_LATEST")
        .allowlist_var("EOS_AUTH_.*")
        .allowlist_var("EOS_PLATFORM_.*")
        .allowlist_var("EOS_LOG.*")
        .allowlist_var("EOS_LC_.*")
        .allowlist_var("EOS_ANTICHEAT.*")
        .layout_tests(false)
        .derive_default(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .constified_enum_module("EOS_EResult")
        .constified_enum_module("EOS_ELoginCredentialType")
        .constified_enum_module("EOS_EExternalCredentialType")
        .constified_enum_module("EOS_ELogLevel")
        .constified_enum_module("EOS_ELogCategory")
        .constified_enum_module("EOS_EAntiCheatClientMode")
        .constified_enum_module("EOS_EAntiCheatClientViolationType")
        .constified_enum_module("EOS_EAntiCheatCommonClientType")
        .constified_enum_module("EOS_EAntiCheatCommonClientPlatform")
        .constified_enum_module("EOS_EAntiCheatCommonClientAction")
        .constified_enum_module("EOS_EAntiCheatCommonClientActionReason")
        .constified_enum_module("EOS_EAntiCheatCommonClientAuthStatus");

    if headers.anticheat_client.exists() {
        builder = builder.header(headers.anticheat_client.to_string_lossy());
    }
    if headers.connect.exists() {
        builder = builder.header(headers.connect.to_string_lossy());
    }

    let bindings = builder.generate().expect("Unable to generate EOS bindings");
    bindings
        .write_to_file(out_path.join("eos_bindings.rs"))
        .expect("Couldn't write EOS bindings!");
}

/// Copies the shipping DLL next to the built artifacts so tests and examples
/// can load it without a manual step.
fn copy_runtime_dll(bin_dir: &Path) {
    let dll = bin_dir.join("EOSSDK-Win64-Shipping.dll");
    if dll.exists() {
        println!("cargo:rerun-if-changed={}", dll.to_string_lossy());

        let profile = env::var("PROFILE").unwrap();
        let out_dirs = [
            format!("target/{profile}"),
            format!("target/{profile}/deps"),
        ];
        let src_meta = std::fs::metadata(&dll).ok();

        for dest_dir in out_dirs {
            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                eprintln!("Warning: Failed to create {dest_dir}: {e}");
                continue;
            }
            let dest_path = Path::new(&dest_dir).join("EOSSDK-Win64-Shipping.dll");
            let should_copy = match (src_meta.as_ref(), std::fs::metadata(&dest_path).ok()) {
                (Some(src), Some(dst)) => {
                    src.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        > dst.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                }
                (Some(_), None) => true,
                _ => false,
            };

            if should_copy && let Err(e) = std::fs::copy(&dll, &dest_path) {
                eprintln!(
                    "Warning: Failed to copy {} to {}: {}",
                    dll.to_string_lossy(),
                    dest_path.to_string_lossy(),
                    e
                );
            }
        }
    } else {
        println!(
            "cargo:warning=EOS DLL not found at {}",
            dll.to_string_lossy()
        );
    }
}

fn find_bundled_eos_sdk(workspace_root: &Path) -> PathBuf {
    let resources_dir = workspace_root.join("resources");

    let mut candidates = fs::read_dir(&resources_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path().join("SDK"))
        .filter(|sdk| {
            sdk.join("Include").exists() || sdk.join("Lib").exists() || sdk.join("Bin").exists()
        })
        .collect::<Vec<_>>();

    candidates.sort();

    for sdk in &candidates {
        println!("cargo:rerun-if-changed={}", sdk.display());
    }

    let fallback = resources_dir.join("EOS-SDK").join("SDK");
    if fallback.exists() {
        println!("cargo:rerun-if-changed={}", fallback.display());
    }

    candidates.into_iter().next().unwrap_or(fallback)
}
