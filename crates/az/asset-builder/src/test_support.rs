//! Test-only product-format registration.
//!
//! The SDK registers no product formats in production processes (ADR 0010 end
//! state) — every real builder owns its narrower byte contract. Test fixtures
//! that only need *a* registered format (worker manifest validation, catalog
//! and package round-trips) opt into this crate feature and use
//! [`TEST_RAW_PRODUCT_FORMAT_ID`] instead of inventing per-test formats.

use az_gem_contract::GemContext;

use crate::value::{ProductFormatId, ProductFormatRegistration};

/// Registered raw byte format reserved for test fixtures.
pub const TEST_RAW_PRODUCT_FORMAT_ID: ProductFormatId =
    ProductFormatId::__from_static("az.test.raw");

/// Current version of the test fixture format.
pub const TEST_RAW_PRODUCT_FORMAT_VERSION: u32 = 1;

/// The test fixture format, for a harness contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::from_dynamic_parts(
        TEST_RAW_PRODUCT_FORMAT_ID,
        TEST_RAW_PRODUCT_FORMAT_VERSION,
    )]
}

/// Register the test fixture format into a composing host.
pub fn register<D>(ctx: &mut GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
}
