//! Owner-only named-pipe security for Windows RPC listeners.
//!
//! `tokio::net::windows::named_pipe::ServerOptions::create` builds the pipe
//! with a null security descriptor, which makes Windows apply the *default*
//! DACL — on most systems that admits any authenticated local user, not just
//! the user that created the pipe. Azoth's local-trust transport model (ADR
//! 0031 Correction 5) requires the opposite: only the creating user may open
//! the pipe. Every pipe-creating service should route pipe creation through
//! [`create_owner_only_named_pipe`] instead of calling `ServerOptions::create`
//! (or `create_with_security_attributes_raw`) directly.

use std::ffi::{OsStr, c_void};
use std::io;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{FALSE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::core::HSTRING;

/// SDDL for a DACL that grants generic-all access (`GA`) to the pipe's
/// creating owner (`OW`) only, and is protected (`P`) from inherited ACEs
/// (so no ambient parent ACL can widen access). No other principal —
/// including other local users — can open the pipe.
const OWNER_ONLY_PIPE_SDDL: &str = "D:P(A;;GA;;;OW)";

/// Owns a `SECURITY_DESCRIPTOR` allocated by
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` plus the
/// `SECURITY_ATTRIBUTES` wrapper `CreateNamedPipeW` expects. The descriptor
/// is freed on drop.
struct OwnerOnlyPipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl OwnerOnlyPipeSecurity {
    fn new() -> io::Result<Self> {
        let sddl = HSTRING::from(OWNER_ONLY_PIPE_SDDL);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // Safety: `sddl` is a valid, NUL-terminated wide string for the
        // duration of the call, and `descriptor` is a valid out-pointer.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &raw mut descriptor,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                .expect("SECURITY_ATTRIBUTES size fits in u32"),
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: FALSE,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    const fn as_raw(&mut self) -> *mut c_void {
        std::ptr::from_mut(&mut self.attributes).cast()
    }
}

impl Drop for OwnerOnlyPipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_invalid() {
            // Safety: `descriptor` was allocated by
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW`, which
            // documents `LocalFree` as the correct release for it.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.descriptor.0)));
            }
        }
    }
}

/// Creates a named-pipe server instance whose security descriptor grants
/// access only to the user that created it (SDDL `D:P(A;;GA;;;OW)`).
///
/// Use this everywhere a service would otherwise call
/// [`ServerOptions::create`] — both for the first pipe instance and for every
/// reconnect instance in a listener's accept loop, since each
/// `CreateNamedPipeW` call establishes its own pipe-instance security
/// descriptor.
///
/// # Errors
///
/// Returns an [`io::Error`] if the SDDL cannot be parsed into a security
/// descriptor, or if the underlying `CreateNamedPipeW` call fails.
pub fn create_owner_only_named_pipe(
    options: &ServerOptions,
    addr: impl AsRef<OsStr>,
) -> io::Result<NamedPipeServer> {
    let mut security = OwnerOnlyPipeSecurity::new()?;
    // Safety: `security` outlives this synchronous FFI call. `CreateNamedPipeW`
    // only reads the security descriptor while creating the pipe instance; it
    // does not retain the pointer afterward, so it is sound to free `security`
    // once this call returns.
    unsafe { options.create_with_security_attributes_raw(addr, security.as_raw()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_only_pipe_security_builds_and_frees_a_descriptor() {
        let mut security = OwnerOnlyPipeSecurity::new().expect("SDDL parses");
        assert!(!security.descriptor.is_invalid());
        assert!(!security.as_raw().is_null());
    }

    #[test]
    fn create_owner_only_named_pipe_serves_a_pipe_name() {
        let name = format!(
            r"\\.\pipe\az-rpc-pipe-security-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        // `NamedPipeServer` registers with the Tokio IO driver on creation, so
        // this needs a live reactor even though the test itself is synchronous.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let mut options = ServerOptions::new();
        options.first_pipe_instance(true);
        let server =
            create_owner_only_named_pipe(&options, &name).expect("owner-only pipe is created");
        drop(server);
    }
}
