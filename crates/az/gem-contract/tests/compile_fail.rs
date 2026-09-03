#[test]
fn ui() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
    // Cases whose expected diagnostic only exists when the Bevy edge is
    // compiled in. Without `app` the same source fails as E0599 (no such
    // method) instead of the E0277 capability refusal it is written to pin,
    // so the whole crate stays testable in both feature states.
    #[cfg(feature = "app")]
    cases.compile_fail("tests/ui/app/*.rs");
}
