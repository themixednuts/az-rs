//! Native contribution image loading and composition.
//!
//! A [`NativeComposition`] owns both the typed [`Composer`] and every image
//! that supplied an admitted contribution. The field order is intentional:
//! contribution lifecycle values are destroyed before their image leases, so a
//! normal shutdown never invokes gem code after its library is released.
//!
//! Non-empty composition is currently fail-closed. A real Windows cdylib
//! proof showed that two separately linked copies of the Rust contract cannot
//! safely mutate one `Composer`, even under the same toolchain and handshake.
//! The loader retains the boundary vocabulary while the shared-contract or
//! narrow-ABI replacement is designed.

use std::path::{Path, PathBuf};

use az_gem_contract::{
    ComposeError, ComposeReport, Composer, ProductActivation, Refusal, Registries,
    native::{
        NATIVE_CONTRIBUTION_COMPOSE_SYMBOL, NATIVE_CONTRIBUTION_CONTRACT_VERSION,
        NATIVE_CONTRIBUTION_HEADER_SYMBOL, NativeContributionComposeEntry,
        NativeContributionExpectation, NativeContributionHeader,
    },
};
use az_project::PreparedRoleDelivery;
use libloading::Library;
use thiserror::Error;

/// One completed native composition.
///
/// The composer is declared before image leases so it drops first. The loader
/// never offers an image-unload operation: reload owns a distinct replacement
/// composition and old images remain mapped until process exit.
pub struct NativeComposition {
    composer: Composer,
    images: Vec<NativeImage>,
}

impl NativeComposition {
    /// Load the already prepared role closure and compose it in manifest order.
    ///
    /// The role is sealed inside [`PreparedRoleDelivery`]; callers cannot
    /// select an arbitrary role or pass an externally-owned composer.
    ///
    /// # Errors
    ///
    /// Returns [`NativeCompositionError::WideRustBoundaryUnproven`] for every
    /// non-empty delivery. Empty compositions remain useful for host bootstrap
    /// and ownership tests while the replacement boundary is designed.
    pub fn compose(delivery: PreparedRoleDelivery) -> Result<Self, NativeCompositionError> {
        let role = delivery.role();
        let contributions = delivery.into_contributions();
        if !contributions.is_empty() {
            return Err(NativeCompositionError::WideRustBoundaryUnproven {
                images: contributions.len(),
            });
        }
        let mut images = Vec::with_capacity(contributions.len());
        let mut composer = Composer::new(role);

        for contribution in contributions {
            let image = NativeImage::open(contribution.path(), contribution.expectation())?;
            // SAFETY: `NativeImage::open` resolved this entry from `image` and
            // retained the library. The matching C handshake was checked before
            // this wide Rust ABI call.
            unsafe { (image.compose)(&mut composer, ProductActivation::default()) }
                .map_err(NativeCompositionError::Refused)?;
            images.push(image);
        }

        composer
            .finalize()
            .map_err(NativeCompositionError::Finalize)?;
        Ok(Self { composer, images })
    }

    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.composer.registries()
    }

    /// Re-runs finalization to report what the loaded images composed.
    ///
    /// # Errors
    ///
    /// Returns any [`ComposeError`] `Composer::finalize` returns when the
    /// accumulated contributions do not form a valid composition — for example
    /// a registration that conflicts with one an earlier image made.
    pub fn compose_report(&self) -> Result<ComposeReport, ComposeError> {
        self.composer.finalize()
    }

    #[must_use]
    pub const fn image_count(&self) -> usize {
        self.images.len()
    }
}

struct NativeImage {
    // Keep the library after the function pointer so the pointer cannot be
    // called after an unload. `Library` is intentionally never exposed.
    compose: NativeContributionComposeEntry,
    _library: Library,
}

impl NativeImage {
    fn open(
        path: &Path,
        expectation: NativeContributionExpectation,
    ) -> Result<Self, NativeCompositionError> {
        // SAFETY: loading an arbitrary image is isolated to this platform
        // boundary. The narrow C handshake below runs before the wide Rust ABI
        // compose entry; platform image initialization may already have run.
        let library =
            unsafe { Library::new(path) }.map_err(|source| NativeCompositionError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        // SAFETY: symbol names include their terminating NUL and the generated
        // shim contract fixes both signatures.
        let header = unsafe {
            library.get::<az_gem_contract::native::NativeContributionHeaderEntry>(
                NATIVE_CONTRIBUTION_HEADER_SYMBOL,
            )
        }
        .map_err(|source| NativeCompositionError::MissingHeader {
            path: path.to_path_buf(),
            source,
        })?;
        // SAFETY: the C header is a Copy `repr(C)` value returned by the exact
        // symbol just resolved. No Rust-owned data crosses this call.
        let header = unsafe { header() };
        verify_header(path, expectation, header)?;
        // SAFETY: symbol names include their terminating NUL and the header
        // already established this image's exact contract compatibility.
        let compose = unsafe {
            library.get::<NativeContributionComposeEntry>(NATIVE_CONTRIBUTION_COMPOSE_SYMBOL)
        }
        .map_err(|source| NativeCompositionError::MissingCompose {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            compose: *compose,
            _library: library,
        })
    }
}

fn verify_header(
    path: &Path,
    expectation: NativeContributionExpectation,
    actual: NativeContributionHeader,
) -> Result<(), NativeCompositionError> {
    if actual.contract_version != NATIVE_CONTRIBUTION_CONTRACT_VERSION {
        return Err(NativeCompositionError::ContractVersion {
            path: path.to_path_buf(),
            expected: NATIVE_CONTRIBUTION_CONTRACT_VERSION,
            actual: actual.contract_version,
        });
    }
    if !expectation.matches(actual) {
        return Err(NativeCompositionError::IdentityMismatch {
            path: path.to_path_buf(),
            expected: Box::new(expectation),
            actual: Box::new(actual),
        });
    }
    Ok(())
}

/// Native image admission or composition failure.
#[derive(Debug, Error)]
pub enum NativeCompositionError {
    #[error(
        "native composition of {images} image(s) is disabled: separately linked Rust contract state is not interchangeable across the image boundary"
    )]
    WideRustBoundaryUnproven { images: usize },
    #[error("open native contribution `{path}`")]
    Open {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    #[error("native contribution `{path}` has no handshake header")]
    MissingHeader {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    #[error("native contribution `{path}` has contract version {actual}, expected {expected}")]
    ContractVersion {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    #[error("native contribution `{path}` does not match its prepared identity")]
    IdentityMismatch {
        path: PathBuf,
        // Boxed: inline these two made `IdentityMismatch` 228 bytes and set the
        // size of every `Result` in this module.
        expected: Box<NativeContributionExpectation>,
        actual: Box<NativeContributionHeader>,
    },
    #[error("native contribution `{path}` has no typed compose entry")]
    MissingCompose {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Finalize(#[from] ComposeError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::native::{NativeBuildIdentity, NativeContributionHeader};
    use az_project::{PreparedContributionArtifact, PreparedRoleDelivery};

    const ENGINE: NativeBuildIdentity = NativeBuildIdentity([1; 32]);
    const RUSTC: NativeBuildIdentity = NativeBuildIdentity([2; 32]);
    const DESCRIPTOR: NativeBuildIdentity = NativeBuildIdentity([3; 32]);

    fn expectation() -> NativeContributionExpectation {
        NativeContributionExpectation {
            engine: ENGINE,
            rustc: RUSTC,
            descriptor: DESCRIPTOR,
        }
    }

    #[test]
    fn a_matching_handshake_is_admitted_before_the_rust_entry() {
        verify_header(
            Path::new("example.dll"),
            expectation(),
            NativeContributionHeader::new(ENGINE, RUSTC, DESCRIPTOR),
        )
        .unwrap();
    }

    #[test]
    fn an_identity_mismatch_refuses_before_registration() {
        let error = verify_header(
            Path::new("example.dll"),
            expectation(),
            NativeContributionHeader::new(ENGINE, RUSTC, NativeBuildIdentity([4; 32])),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            NativeCompositionError::IdentityMismatch { .. }
        ));
    }

    #[test]
    fn non_empty_composition_fails_closed_before_opening_an_image() {
        let delivery = PreparedRoleDelivery::for_test(
            az_gem_contract::GemTargetRole::RuntimeHost,
            vec![PreparedContributionArtifact::for_test(
                "does-not-exist.dll",
                expectation(),
            )],
        );

        assert!(matches!(
            NativeComposition::compose(delivery),
            Err(NativeCompositionError::WideRustBoundaryUnproven { images: 1 })
        ));
    }
}
