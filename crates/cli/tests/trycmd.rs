#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
#[test]
fn cli_trycmd_tests() {
    // `set_var` is unsafe in edition 2024; the cases are the only readers, so pass it to them directly.
    let t = trycmd::TestCases::new();
    t.env("COLUMNS", "80");
    t.case("tests/cmd/*.trycmd");
}
