//! `#[derive(AzComponent)]` registers unconditionally, so every component type
//! owes its crate's lowering enumeration an entry.

use az_derive::AzComponent;

#[derive(AzComponent)]
#[az_component("55555555-5555-5555-5555-555555555555")]
struct Widget;

fn main() {
    let _ = Widget;
}
