#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use
)]
/// Drops SGR colour sequences so assertions read plain text.
pub fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            chars.by_ref().find(|&c| c == 'm');
        } else {
            plain.push(c);
        }
    }
    plain
}

/// An empty `typescript` install under `root`, so no missing-package warning joins the output.
pub fn install_stub_typescript(root: &std::path::Path) {
    let lib = root.join("node_modules/typescript/lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(root.join("node_modules/typescript/package.json"), r#"{"name":"typescript","version":"5.9.3"}"#)
        .unwrap();
    for file in ["lib.es5.d.ts", "lib.dom.d.ts"] {
        std::fs::write(lib.join(file), "").unwrap();
    }
}
