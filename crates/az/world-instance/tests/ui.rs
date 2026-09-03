//! Compile-fail proof that the refined lifecycle identities are nominal.
//!
//! ADR 0052 requires that instance, placement, process, route, admission,
//! presence, transfer, checkpoint, and operation identities cannot be
//! interchanged accidentally. A runtime assertion cannot prove that, because
//! the mistake it guards against never reaches runtime: these cases fail to
//! compile, and the recorded stderr names the exact expected type.

#[test]
fn refined_lifecycle_identities_cannot_be_interchanged() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/pass/refined_identities.rs");
    cases.compile_fail("tests/ui/fail/*.rs");
}
