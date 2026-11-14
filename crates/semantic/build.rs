//! Fingerprints the code that makes the checker's answer, so an answer kept on disk (`src/kept.rs`) is discarded
//! whenever it changes: this crate's sources and, in a workspace build, the locked dependency versions, which pin the
//! built-in checker (`tsrs`). `caliper-docgen-core`'s stored outcome carries that crate's own fingerprint.

use std::path::{Path, PathBuf};

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
    // Absent when built from the published crate, whose version then identifies the code and its pinned checker.
    let lock = Path::new("../../Cargo.lock");
    if let Ok(bytes) = std::fs::read(lock) {
        hash.write(&bytes);
    }

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rustc-env=CALIPER_SEMANTIC_FINGERPRINT={:016x}", hash.0);
}
