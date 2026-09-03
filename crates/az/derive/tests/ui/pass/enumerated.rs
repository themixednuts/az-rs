//! A registered type listed by its own crate: the shape every contributing
//! crate writes.

use az_core::component::ComponentLoweringRegistration;
use az_core::rtti::AzTypeRegistration;
use az_derive::{AzComponent, AzRtti};

#[derive(AzRtti)]
#[az_rtti("44444444-4444-4444-4444-444444444444", register)]
struct Marker;

#[derive(AzComponent)]
#[az_component("55555555-5555-5555-5555-555555555555")]
struct Widget;

fn types() -> [AzTypeRegistration; 1] {
    [Marker::REGISTRATION]
}

fn lowerings() -> [ComponentLoweringRegistration; 1] {
    [Widget::REGISTRATION]
}

fn main() {
    assert_eq!(types().len(), 1);
    assert_eq!(lowerings().len(), 1);
}
