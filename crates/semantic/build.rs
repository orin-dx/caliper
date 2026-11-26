//! Fingerprints the code that makes the checker's answer, so an answer kept on disk (`src/kept.rs`) is discarded
//! whenever it changes: this crate's sources and the locked dependency versions (the workspace's lockfile, else the one
//! a published crate ships), which pin the built-in checker (`tsrs`). `caliper-docgen-core`'s stored outcome carries that crate's own fingerprint.

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

/// The one rule for where the locked versions are read from, shared with the tests through `CALIPER_SEMANTIC_LOCKFILE`:
/// the lockfile Cargo resolves this package with. That is the workspace's when the workspace two levels up lists this
/// package as a member, else the package's own (`cargo package` ships one), else none; a `Cargo.lock` of some other
/// project in an ancestor directory is never taken. Paths come back joined onto `package`.
pub(crate) fn lockfile_in(package: &Path) -> Option<PathBuf> {
    let workspace = package.join("../..");
    let shared = workspace.join("Cargo.lock");
    if lists_member(&workspace.join("Cargo.toml"), package) && shared.is_file() {
        return Some(shared);
    }
    let own = package.join("Cargo.lock");
    own.is_file().then_some(own)
}

/// Whether the manifest has a `[workspace]` that lists `package` (a directory two levels below it) in `members`, by its
/// path or a `parent/*` glob, and does not `exclude` it. Reads only that table's arrays; unsure means no.
fn lists_member(manifest: &Path, package: &Path) -> bool {
    let (Ok(text), Ok(package)) = (std::fs::read_to_string(manifest), package.canonicalize()) else { return false };
    let (Some(name), Some(parent)) = (package.file_name(), package.parent().and_then(Path::file_name)) else {
        return false;
    };
    let (name, parent) = (name.to_string_lossy(), parent.to_string_lossy());
    let member = format!("{parent}/{name}");

    let mut table = String::new();
    let mut in_workspace = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_workspace = line == "[workspace]";
        } else if in_workspace {
            table.push_str(line);
            table.push('\n');
        }
    }
    let array = |key: &str| -> Vec<String> {
        let rest = table.lines().skip_while(|line| !is_key(line, key)).collect::<Vec<_>>().join("\n");
        let inside = rest.split_once('[').map_or("", |(_, inside)| inside.split(']').next().unwrap_or_default());
        inside.split('"').skip(1).step_by(2).map(str::to_string).collect()
    };
    let glob = format!("{parent}/*");
    array("members").iter().any(|m| *m == member || *m == glob) && !array("exclude").contains(&member)
}

fn is_key(line: &str, key: &str) -> bool {
    line.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
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
        // Relative to the manifest directory, so the build machine's path is not compiled in.
        println!("cargo:rustc-env=CALIPER_SEMANTIC_LOCKFILE={}", lock.display());
    }
    println!("cargo:rustc-env=CALIPER_SEMANTIC_FINGERPRINT={:016x}", hash.0);
}
