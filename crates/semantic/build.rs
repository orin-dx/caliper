//! Fingerprints the code that makes the checker's answer, so an answer kept on disk (`src/kept.rs`) is discarded
//! whenever it changes: this crate's sources and the locked dependency versions (the workspace's lockfile, else the one
//! a published crate ships), which pin the built-in checker (`tsrs`). `caliper-docgen-core`'s stored outcome carries
//! that crate's own fingerprint.

use std::path::{Path, PathBuf};
use std::process::Command;

/// FNV-1a: stable across Rust versions and platforms, unlike `std`'s `DefaultHasher`.
struct Fnv(u64);

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The one rule for where the locked versions are read from, shared with the tests through `CALIPER_SEMANTIC_LOCKFILE`:
/// the lockfile Cargo resolves this package with. Cargo itself says where the workspace root is, which covers members
/// listed by path or glob and crates it takes in as path dependencies of a member; a package that no workspace claims
/// (Cargo then reports an error) reads its own, as a published crate ships one. Without either there is none, and the
/// tests say they skipped. A `Cargo.lock` of some other project in an ancestor directory is never taken.
pub(crate) fn lockfile_in(package: &Path) -> Option<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let root = Command::new(cargo)
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .current_dir(package)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|manifest| Path::new(manifest.trim()).parent().map(|root| root.join("Cargo.lock")));
    let own = package.join("Cargo.lock");
    root.filter(|lock| lock.is_file()).or_else(|| own.is_file().then_some(own))
}

fn lockfile() -> Option<PathBuf> {
    lockfile_in(Path::new("."))
}

fn main() {
    let mut files = Vec::new();
    source_files(Path::new("src"), &mut files);
    files.sort();

    let mut hash = Fnv(0xcbf2_9ce4_8422_2325);
    for file in &files {
        hash.write(file.to_string_lossy().as_bytes());
        if let Ok(bytes) = std::fs::read(file) {
            hash.write(&bytes);
        }
    }
    let lock = lockfile();
    if let Some(bytes) = lock.as_ref().and_then(|lock| std::fs::read(lock).ok()) {
        hash.write(&bytes);
    }

    println!("cargo:rerun-if-changed=src");
    if let Some(lock) = &lock {
        println!("cargo:rerun-if-changed={}", lock.display());
        println!("cargo:rustc-env=CALIPER_SEMANTIC_LOCKFILE={}", lock.display());
    }
    println!("cargo:rustc-env=CALIPER_SEMANTIC_FINGERPRINT={:016x}", hash.0);
}
