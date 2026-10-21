#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
// SPEC-TYPES-001 AC-001: `ParsedProp`'s private `_seal` field must make a
// bare struct literal fail to compile anywhere outside
// crates/core/src/types/output.rs.
#[test]
fn parsed_prop_bare_struct_literal_fails_to_compile_outside_output_rs() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/parsed_prop_seal.rs");
}
