//! `Listed` is in the crate's enumeration; `Forgotten` declares the same
//! registration and is not. Under the inventory this compiled and produced an
//! entry no host ever received.

use az_core::rtti::AzTypeRegistration;
use az_derive::AzRtti;

#[derive(AzRtti)]
#[az_rtti("44444444-4444-4444-4444-444444444444", register)]
struct Listed;

#[derive(AzRtti)]
#[az_rtti("66666666-6666-6666-6666-666666666666", register)]
struct Forgotten;

fn types() -> [AzTypeRegistration; 1] {
    [Listed::REGISTRATION]
}
fn main() {
    // The type itself is in use — only its registration was left out.
    let _ = Forgotten;
    assert_eq!(types().len(), 1);
}
