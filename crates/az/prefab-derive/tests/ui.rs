#[test]
fn derive_compile_pass_and_fail_matrix() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/pass/direct_component.rs");
    cases.pass("tests/ui/pass/source_only_component.rs");
    cases.pass("tests/ui/pass/template_construction.rs");
    cases.compile_fail("tests/ui/fail/*.rs");
}
