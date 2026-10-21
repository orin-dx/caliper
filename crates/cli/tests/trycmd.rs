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
    std::env::set_var("COLUMNS", "80");
    let t = trycmd::TestCases::new();
    t.case("tests/cmd/*.trycmd");
}
