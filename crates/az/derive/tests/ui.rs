//! The registration item's omission detector, exercised end to end.
//!
//! A derive cannot append to a per-crate list without a link section, so the
//! crate that owns a type has to name it. These cases are what keeps that list
//! honest: declaring a registration and then leaving the type out of the
//! enumeration does not compile, and the error names the type.

#[test]
fn an_unenumerated_registration_does_not_compile() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/pass/enumerated.rs");
    cases.compile_fail("tests/ui/fail/*.rs");
}
