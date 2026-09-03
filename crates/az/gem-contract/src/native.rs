//! Native contribution handshake vocabulary.
//!
//! This module deliberately contains no platform loading operation. A host
//! verifies this fixed header before invoking the wide, same-toolchain Rust
//! compose entry from a contribution image.

use crate::capability::Refusal;
use crate::composer::Composer;
use crate::descriptor::ProductActivation;

/// Native contribution contract revision.
pub const NATIVE_CONTRIBUTION_CONTRACT_VERSION: u32 = 1;
/// Exported C handshake symbol every generated native contribution shim owns.
pub const NATIVE_CONTRIBUTION_HEADER_SYMBOL: &[u8] = b"azoth_native_contribution_header\0";
/// Exported wide-Rust compose symbol every generated native contribution shim owns.
pub const NATIVE_CONTRIBUTION_COMPOSE_SYMBOL: &[u8] = b"azoth_native_contribution_compose\0";

/// Fixed-size identity used by the native handshake.
///
/// The bytes are a domain-separated digest chosen by the engine build and are
/// intentionally opaque to gem code. Comparing them before a Rust ABI call is
/// the native-tier compatibility rule; a mismatch is a refusal, never a best
/// effort load.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeBuildIdentity(pub [u8; 32]);

/// The small C-compatible header returned before any wide Rust call.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeContributionHeader {
    pub contract_version: u32,
    pub engine: NativeBuildIdentity,
    pub rustc: NativeBuildIdentity,
    pub descriptor: NativeBuildIdentity,
}

impl NativeContributionHeader {
    #[must_use]
    pub const fn new(
        engine: NativeBuildIdentity,
        rustc: NativeBuildIdentity,
        descriptor: NativeBuildIdentity,
    ) -> Self {
        Self {
            contract_version: NATIVE_CONTRIBUTION_CONTRACT_VERSION,
            engine,
            rustc,
            descriptor,
        }
    }
}

/// The expected native handshake values for one prepared contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeContributionExpectation {
    pub engine: NativeBuildIdentity,
    pub rustc: NativeBuildIdentity,
    pub descriptor: NativeBuildIdentity,
}

impl NativeContributionExpectation {
    #[must_use]
    pub fn matches(self, header: NativeContributionHeader) -> bool {
        header.contract_version == NATIVE_CONTRIBUTION_CONTRACT_VERSION
            && header.engine == self.engine
            && header.rustc == self.rustc
            && header.descriptor == self.descriptor
    }
}

/// C handshake entry from a generated shim.
pub type NativeContributionHeaderEntry = unsafe extern "C" fn() -> NativeContributionHeader;

/// Wide Rust entry from a generated shim.
///
/// The shim owns the concrete contribution type and calls
/// [`Composer::add`], preserving the sole typed registration path. The loader
/// calls this only after a successful [`NativeContributionHeaderEntry`]
/// comparison and retains the image for the resulting composition's life.
pub type NativeContributionComposeEntry =
    unsafe extern "Rust" fn(&mut Composer, ProductActivation) -> Result<(), Refusal>;
