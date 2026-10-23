//! DTS cache — persists parsed `SourceData` across runs to avoid re-parsing unchanged .d.ts files.
//!
//! Cache validity is based on a content hash of the file's actual bytes, not
//! mtime/size (see `CacheKey`'s doc comment for why that was replaced).
//! The payload starts with a `CacheHeader`; a header that doesn't match the
//! running build (schema, crate version, source fingerprint) discards the whole cache.
//!
//! The cache never panics — load failures degrade gracefully to an empty
//! state, and save failures are reported via a `Diagnostic` (see
//! `save_to_disk`) rather than swallowed.

use camino::{Utf8Path, Utf8PathBuf};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::hash::Hasher;

use crate::types::{Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceData};

/// Bump this whenever `SourceData`'s field list or field order changes.
/// Serialization is via plain `rmp_serde::to_vec`/`from_slice` — MessagePack's
/// *positional* (not named-map) struct encoding — so inserting a field
/// anywhere but the very end shifts every subsequent field's decode position
/// for any cache entry written before the change. A same-shaped field
/// addition (e.g. another `FxHashMap<String, Vec<X>>`) wouldn't even fail
/// loudly; it would decode as plausible-looking but wrong data. `const_arrays`
/// (added mid-struct, not appended) is exactly this case — this bump covers it.
/// Also bumped for the mtime+size -> content-hash key change (P0-2 fix):
/// `SerializableCacheKey`'s own shape changed, so pre-existing on-disk cache
/// files must be discarded rather than misread under the new field layout.
/// Extractor changes that keep the shape are covered by `CacheHeader::fingerprint` instead.
/// 16: `LexedExport::LocalDeclaration` gained `declared_as`.
/// 17: component mappings record whether they are exported.
/// 18: generic functions' result types, and the callee of a wrapper-built component.
/// 19: a mapped type records its key variable and `?` modifier.
const CACHE_SCHEMA_VERSION: u32 = 19;

const DATA_FILE: &str = "dts.msgpack";

/// Written first in the payload, so a cache written by a different build is recognized before its entries are decoded.
/// The manifest repeats it for inspection only.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct CacheHeader {
    schema: u32,
    version: String,
    /// A hash of this crate's sources and locked dependencies (see `build.rs`): any change to the code that produces
    /// `SourceData` invalidates the cache, bumped or not.
    fingerprint: String,
}

impl CacheHeader {
    fn current() -> CacheHeader {
        CacheHeader {
            schema: CACHE_SCHEMA_VERSION,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            fingerprint: env!("CALIPER_SOURCE_FINGERPRINT").to_owned(),
        }
    }
}

// ─── Internal key type ────────────────────────────────────────────────────────

/// Keyed by a hash of the file's actual byte content, not `(size, mtime)`.
///
/// The prior mtime+size scheme had a documented staleness gap (P0-2): on
/// filesystems/environments with coarse mtime resolution (network FS,
/// container overlay FS, or simply two edits landing in the same clock
/// tick), an edit completing within the same tick and producing a
/// same-length file was indistinguishable from the original — a stale
/// `SourceData` was served with no signal anything was wrong. A content hash
/// closes this gap exactly: two different byte sequences essentially never
/// hash to the same 64-bit `FxHasher` value (birthday-bound collision risk
/// is negligible at this cache's `MAX_CACHE_ENTRIES` scale), and identical
/// bytes always hash identically regardless of mtime/clock granularity.
/// This trades away the old stat-only check's near-zero I/O cost — computing
/// the hash requires the file's content, not just its metadata — but every
/// call site already reads the full file on a miss to parse it anyway; `get`
/// and `insert` both take that same content as a parameter instead of
/// re-deriving a (now-unreliable) proxy for it, so no call site pays for an
/// extra read this fix didn't already need.
#[derive(Hash, Eq, PartialEq, Clone)]
struct CacheKey {
    path: Utf8PathBuf,
    content_hash: u64,
}

fn hash_content(content: &str) -> u64 {
    let mut hasher = rustc_hash::FxHasher::default();
    hasher.write(content.as_bytes());
    hasher.finish()
}

// ─── Serializable surrogate ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct SerializableCacheKey {
    path: String,
    content_hash: u64,
}

impl From<&CacheKey> for SerializableCacheKey {
    fn from(k: &CacheKey) -> Self {
        Self { path: k.path.to_string(), content_hash: k.content_hash }
    }
}

impl From<SerializableCacheKey> for CacheKey {
    fn from(k: SerializableCacheKey) -> Self {
        Self { path: Utf8PathBuf::from(k.path), content_hash: k.content_hash }
    }
}

// ─── DtsCache ────────────────────────────────────────────────────────────────

/// Maximum number of cached `.d.ts` entries allowed in memory/on disk.
const MAX_CACHE_ENTRIES: usize = 5000;

/// The largest cache file loaded. A cache holding TypeScript's lib files, the largest entries, measures about 3 MB.
const MAX_CACHE_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Thread-safe DTS parse-result cache.
///
/// Keyed by `(path, content_hash)` — if the file's content hasn't changed,
/// the cached `SourceData` is returned directly without re-parsing.
pub struct DtsCache {
    store: DashMap<CacheKey, SourceData>,
    cache_dir: Utf8PathBuf,
    dirty: std::sync::atomic::AtomicBool,
    lookups: std::sync::atomic::AtomicU32,
    hits: std::sync::atomic::AtomicU32,
    load_diagnostic: Option<Diagnostic>,
}

/// Why a cache on disk was not used.
enum LoadFailure {
    /// No cache yet, or one written by a different build: expected, so not reported.
    Absent,
    /// A cache from this build that could not be read.
    Unreadable(String),
}

impl DtsCache {
    /// Load cache from disk.
    ///
    /// On any error (missing dir, a different build's cache, corrupt data) the cache starts empty and `insert` calls
    /// repopulate it. A cache from this build that can't be decoded is reported by `load_diagnostic`.
    ///
    /// `cache_dir` defaults to `node_modules/.cache/caliper-docgen/` relative
    /// to the current working directory.
    pub fn load_from_disk(cache_dir: Option<&Utf8Path>) -> Self {
        let dir =
            cache_dir.map(|p| p.to_owned()).unwrap_or_else(|| Utf8PathBuf::from("node_modules/.cache/caliper-docgen"));

        let (store, load_diagnostic) = match Self::try_load(&dir) {
            Ok(store) => (store, None),
            Err(LoadFailure::Absent) => (DashMap::default(), None),
            Err(LoadFailure::Unreadable(reason)) => (
                DashMap::default(),
                Some(
                    Diagnostic::new(
                        DiagnosticSeverity::Info,
                        DiagnosticCode::IoError,
                        format!("Discarded the unreadable DTS cache at '{dir}': {reason}"),
                    )
                    .with_help("This run parsed every file and rewrites the cache; extraction is unaffected."),
                ),
            ),
        };
        Self {
            store,
            cache_dir: dir,
            dirty: std::sync::atomic::AtomicBool::new(false),
            lookups: std::sync::atomic::AtomicU32::new(0),
            hits: std::sync::atomic::AtomicU32::new(0),
            load_diagnostic,
        }
    }

    fn try_load(dir: &Utf8Path) -> Result<DashMap<CacheKey, SourceData>, LoadFailure> {
        use serde::Deserialize as _;

        // Checked before reading, so a cache file that isn't ours (or a corrupt one) can't make a run allocate it all.
        let path = dir.join(DATA_FILE);
        if let Ok(meta) = std::fs::metadata(path.as_std_path()) {
            if meta.len() > MAX_CACHE_FILE_BYTES {
                return Err(LoadFailure::Unreadable(format!(
                    "it is {} bytes, over the {MAX_CACHE_FILE_BYTES}-byte limit",
                    meta.len()
                )));
            }
        }
        let bytes = match std::fs::read(path.as_std_path()) {
            Ok(bytes) => bytes,
            Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                return Err(LoadFailure::Absent)
            }
            Err(error) => return Err(LoadFailure::Unreadable(error.to_string())),
        };
        let mut decoder = rmp_serde::Deserializer::new(bytes.as_slice());
        // A header that doesn't decode is a payload from an older layout; that is a different build too.
        match CacheHeader::deserialize(&mut decoder) {
            Ok(header) if header == CacheHeader::current() => {}
            _ => return Err(LoadFailure::Absent),
        }
        let entries = Vec::<(SerializableCacheKey, SourceData)>::deserialize(&mut decoder)
            .map_err(|error| LoadFailure::Unreadable(error.to_string()))?;

        let map = DashMap::with_capacity(entries.len());
        for (k, v) in entries {
            map.insert(CacheKey::from(k), v);
        }
        Ok(map)
    }

    /// Why the cache on disk could not be used, when that is worth reporting.
    pub fn load_diagnostic(&self) -> Option<&Diagnostic> {
        self.load_diagnostic.as_ref()
    }

    /// `(lookups, hits)` since load: how many `.d.ts` files went through the cache and how many it answered.
    pub fn counts(&self) -> (u32, u32) {
        (self.lookups.load(std::sync::atomic::Ordering::Relaxed), self.hits.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Look up cached `SourceData` for `path`, given its current content.
    ///
    /// Returns `None` if the file is not cached or its content hash has
    /// changed. `content` is a parameter rather than read internally because
    /// every call site already reads the file to parse it on a miss — taking
    /// content here means neither a hit nor a miss ever reads the file twice.
    pub fn get(&self, path: &Utf8Path, content: &str) -> Option<SourceData> {
        let key = self.key_for(path, content);
        self.lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let hit = self.store.get(&key).map(|v| v.clone());
        if hit.is_some() {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        hit
    }

    /// Insert `data` into the cache, keyed by `path` + a hash of `content`.
    pub fn insert(&self, path: &Utf8Path, content: &str, data: SourceData) {
        let key = self.key_for(path, content);
        // Evict extra entries if cache size exceeds MAX_CACHE_ENTRIES
        if self.store.len() >= MAX_CACHE_ENTRIES {
            let keys_to_evict: Vec<CacheKey> = self.store.iter().take(100).map(|r| r.key().clone()).collect();
            for k in keys_to_evict {
                self.store.remove(&k);
            }
        }
        self.store.insert(key, data);
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Persist the cache to disk atomically (temp file + rename).
    ///
    /// Skips writing if the cache is clean (no inserts since load). Never
    /// panics — on failure, returns a `Diagnostic` describing what went
    /// wrong (cache persistence is best-effort and must not fail the
    /// extraction run) instead of failing silently. Severity is `Info`, not
    /// `Warning`: a `Warning` here would flip `docgen check --strict`'s exit
    /// code to 1 purely because caching failed, even though extraction
    /// output itself is entirely correct — the exact class of spurious
    /// exit-code flip this project has already fixed once (see the
    /// resolver's discarded-return-type-resolution fix).
    pub fn save_to_disk(&self) -> Option<Diagnostic> {
        if !self.dirty.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        self.try_save().err().map(|reason| Diagnostic {
            severity: DiagnosticSeverity::Info,
            message: format!("Failed to persist the DTS cache to '{}': {reason}", self.cache_dir),
            file: None,
            line: None,
            column: None,
            help: Some("Check that the cache directory is writable. Extraction still succeeded; only caching for the next run was affected.".into()),
            code: DiagnosticCode::IoError,
            affects: Vec::new(),
        })
    }

    fn try_save(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.cache_dir.as_std_path()).map_err(|e| e.to_string())?;

        // Collect into a serializable form.
        let entries: Vec<(SerializableCacheKey, SourceData)> =
            self.store.iter().map(|r| (SerializableCacheKey::from(r.key()), r.value().clone())).collect();

        let header = CacheHeader::current();
        let mut bytes = rmp_serde::to_vec(&header).map_err(|e| e.to_string())?;
        bytes.extend(rmp_serde::to_vec(&entries).map_err(|e| e.to_string())?);

        write_atomic(&self.cache_dir, DATA_FILE, &bytes)?;

        // For inspection only; loading trusts the payload's own header.
        let manifest = serde_json::json!({
            "schema": header.schema,
            "version": header.version,
            "fingerprint": header.fingerprint,
            "entry_count": entries.len(),
        });
        let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        write_atomic(&self.cache_dir, "manifest.json", manifest_json.as_bytes())?;

        self.dirty.store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Builds the cache key from the file's path and a hash of its content —
    /// see `CacheKey`'s doc comment for why this replaced mtime+size.
    fn key_for(&self, path: &Utf8Path, content: &str) -> CacheKey {
        CacheKey { path: path.to_owned(), content_hash: hash_content(content) }
    }
}

/// Write via temp file + rename so readers never see a partial file. The temp name is unique per process and call:
/// processes sharing a cache dir (CLI + Vite plugin, parallel tests) would otherwise clobber each other's temp file.
fn write_atomic(dir: &Utf8Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp_path = dir.join(format!("{name}.{}.{seq}.tmp", std::process::id()));
    let final_path = dir.join(name);

    let result = std::fs::write(tmp_path.as_std_path(), bytes)
        .and_then(|()| std::fs::rename(tmp_path.as_std_path(), final_path.as_std_path()));
    if result.is_err() {
        std::fs::remove_file(tmp_path.as_std_path()).ok();
    }
    result.map_err(|e| e.to_string())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SourceData;
    use camino::Utf8Path;
    use std::io::Write;

    /// Create a unique temp directory under std::env::temp_dir().
    fn temp_dir(suffix: &str) -> Utf8PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let base = std::env::temp_dir();
        let dir = base.join(format!("caliper-docgen-cache-test-{}-{}", suffix, ts));
        std::fs::create_dir_all(&dir).unwrap();
        Utf8PathBuf::from_path_buf(dir).unwrap()
    }

    fn make_temp_file(dir: &Utf8PathBuf, name: &str, content: &[u8]) -> Utf8PathBuf {
        let path = dir.as_std_path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    #[test]
    fn test_get_miss_on_empty_cache() {
        let tmp = temp_dir("miss");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        let file_path = make_temp_file(&tmp, "foo.d.ts", b"export {}");
        assert!(cache.get(Utf8Path::new(file_path.as_str()), "export {}").is_none());
    }

    #[test]
    fn test_insert_and_get() {
        let tmp = temp_dir("insert");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        let content = "export type Foo = string;";
        let file_path = make_temp_file(&tmp, "bar.d.ts", content.as_bytes());
        let data = SourceData::default();

        cache.insert(&file_path, content, data.clone());
        let retrieved = cache.get(&file_path, content);
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_save_and_reload() {
        let tmp = temp_dir("reload");
        let cache_dir = tmp.join("cache");

        let content = "export type X = number;";
        let file_path = make_temp_file(&tmp, "baz.d.ts", content.as_bytes());

        {
            let cache = DtsCache::load_from_disk(Some(&cache_dir));
            cache.insert(&file_path, content, SourceData::default());
            cache.save_to_disk();
        }

        // Check manifest was written.
        let manifest_path = cache_dir.join("manifest.json");
        let manifest_content = std::fs::read_to_string(manifest_path.as_std_path()).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_content).unwrap();
        assert_eq!(manifest["schema"].as_u64().unwrap(), CACHE_SCHEMA_VERSION as u64);
        assert_eq!(manifest["entry_count"].as_u64().unwrap(), 1);

        // Reload and verify entry is present.
        let cache2 = DtsCache::load_from_disk(Some(&cache_dir));
        assert!(cache2.get(&file_path, content).is_some());
    }

    /// A cache with unsaved changes, so `save_to_disk` tries to write.
    fn dirty_cache_at(dir: &Utf8Path) -> DtsCache {
        let cache = DtsCache::load_from_disk(Some(dir));
        cache.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        cache
    }

    /// A saved cache holding one entry, and the file it caches.
    fn saved_cache(suffix: &str) -> (Utf8PathBuf, Utf8PathBuf, &'static str) {
        let tmp = temp_dir(suffix);
        let cache_dir = tmp.join("cache");
        let content = "export type X = number;";
        let file = make_temp_file(&tmp, "x.d.ts", content.as_bytes());
        let cache = DtsCache::load_from_disk(Some(&cache_dir));
        cache.insert(&file, content, SourceData::default());
        assert_eq!(cache.save_to_disk(), None);
        (cache_dir, file, content)
    }

    fn rewrite_payload(cache_dir: &Utf8Path, header: &CacheHeader, entries: &[u8]) {
        let mut bytes = rmp_serde::to_vec(header).unwrap();
        bytes.extend_from_slice(entries);
        std::fs::write(cache_dir.join(DATA_FILE).as_std_path(), bytes).unwrap();
    }

    #[test]
    fn a_cache_from_this_build_is_used() {
        let (cache_dir, file, content) = saved_cache("same-build");

        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        assert!(cache.get(&file, content).is_some());
        assert_eq!(cache.load_diagnostic(), None);
    }

    #[test]
    fn a_cache_from_a_different_build_is_discarded_silently() {
        for header in [
            CacheHeader { schema: CACHE_SCHEMA_VERSION + 1, ..CacheHeader::current() },
            CacheHeader { version: "0.0.0".into(), ..CacheHeader::current() },
            CacheHeader { fingerprint: "0000000000000000".into(), ..CacheHeader::current() },
        ] {
            let (cache_dir, file, content) = saved_cache("other-build");
            let entries = rmp_serde::to_vec(&vec![(
                SerializableCacheKey { path: file.to_string(), content_hash: hash_content(content) },
                SourceData::default(),
            )])
            .unwrap();
            rewrite_payload(&cache_dir, &header, &entries);

            let cache = DtsCache::load_from_disk(Some(&cache_dir));

            assert!(cache.get(&file, content).is_none(), "{header:?}");
            assert_eq!(cache.load_diagnostic(), None, "{header:?}");
        }
    }

    #[test]
    fn a_payload_without_a_header_is_discarded_silently() {
        let (cache_dir, file, content) = saved_cache("headerless");
        std::fs::write(cache_dir.join(DATA_FILE).as_std_path(), [0xc1]).unwrap();

        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        assert!(cache.get(&file, content).is_none());
        assert_eq!(cache.load_diagnostic(), None);
    }

    #[test]
    fn a_cache_file_over_the_size_limit_is_reported_without_being_read() {
        let (cache_dir, file, content) = saved_cache("oversized");
        // Sparse: the length is set without writing the bytes.
        std::fs::OpenOptions::new()
            .write(true)
            .open(cache_dir.join(DATA_FILE).as_std_path())
            .unwrap()
            .set_len(MAX_CACHE_FILE_BYTES + 1)
            .unwrap();

        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        assert!(cache.get(&file, content).is_none());
        let diagnostic = cache.load_diagnostic().expect("a diagnostic");
        assert!(
            diagnostic.message.ends_with(&format!("over the {MAX_CACHE_FILE_BYTES}-byte limit")),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn corrupt_entries_under_this_builds_header_are_reported_and_discarded() {
        let (cache_dir, file, content) = saved_cache("corrupt");
        rewrite_payload(&cache_dir, &CacheHeader::current(), &[0x92, 0xc1]);

        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        assert!(cache.get(&file, content).is_none());
        let diagnostic = cache.load_diagnostic().expect("a diagnostic");
        assert_eq!((&diagnostic.severity, &diagnostic.code), (&DiagnosticSeverity::Info, &DiagnosticCode::IoError));
        assert!(diagnostic.message.starts_with(&format!("Discarded the unreadable DTS cache at '{cache_dir}': ")));
    }

    #[test]
    fn counts_report_every_lookup_and_every_hit() {
        let (cache_dir, file, content) = saved_cache("counts");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        let _ = cache.get(&file, content);
        let _ = cache.get(&file, "export type X = string;");
        let _ = cache.get(&file, content);

        assert_eq!(cache.counts(), (3, 2));
    }

    #[test]
    fn test_stale_key_after_content_change() {
        let tmp = temp_dir("stale");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        let file_path = make_temp_file(&tmp, "changing.d.ts", b"v1 with more content here");
        cache.insert(&file_path, "v1 with more content here", SourceData::default());

        // Overwrite with DIFFERENT content of the SAME LENGTH — under the old
        // mtime+size key this was indistinguishable from the original on a
        // filesystem with coarse mtime resolution (P0-2, the exact bug this
        // cache was rewritten to fix). The content hash catches it regardless.
        let new_content = "v2 with more content here";
        assert_eq!(
            new_content.len(),
            "v1 with more content here".len(),
            "test fixture must be same-length to prove this"
        );
        std::fs::write(file_path.as_std_path(), new_content).unwrap();

        assert!(
            cache.get(&file_path, new_content).is_none(),
            "a same-length content change must miss the cache — this is P0-2's exact regression"
        );
    }

    #[test]
    fn test_identical_content_still_hits_even_with_a_different_mtime() {
        // The other half of the same fix: content hashing must not become
        // MORE conservative than the old scheme for the common case — an
        // untouched-content rewrite (e.g. a build tool that touches mtime
        // without changing bytes) should still hit.
        let tmp = temp_dir("identical");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        let content = "export type Stable = string;";
        let file_path = make_temp_file(&tmp, "stable.d.ts", content.as_bytes());
        cache.insert(&file_path, content, SourceData::default());

        // Rewrite with byte-identical content — mtime changes, bytes don't.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(file_path.as_std_path(), content).unwrap();

        assert!(cache.get(&file_path, content).is_some(), "identical content must still hit regardless of mtime");
    }

    #[test]
    fn test_load_from_disk_no_dir_returns_empty() {
        // A nonexistent cache dir should just give an empty cache without panicking.
        let cache = DtsCache::load_from_disk(Some(Utf8Path::new("/nonexistent/cache/dir")));
        assert!(cache.store.is_empty());
    }

    #[test]
    fn test_save_to_disk_never_panics_on_bad_dir() {
        // Writing to a path that can't be created should not panic.
        let cache = dirty_cache_at(Utf8Path::new("/dev/null/impossible/cache"));
        cache.save_to_disk(); // must not panic
    }

    #[test]
    fn test_save_to_disk_surfaces_a_diagnostic_instead_of_failing_silently() {
        // Regression test for: a cache-persistence failure (unwritable dir,
        // full disk, sandboxed CI, etc.) was silently swallowed via `let _ =
        // self.try_save()`, violating "always emit a Diagnostic when
        // degrading" — every subsequent run would re-parse every .d.ts file
        // with zero user-visible signal.
        let cache = dirty_cache_at(Utf8Path::new("/dev/null/impossible/cache"));
        let diagnostic = cache.save_to_disk();
        assert!(diagnostic.is_some(), "expected a diagnostic when cache persistence fails, got None");
        let diagnostic = diagnostic.unwrap();
        // Info, not Warning: a cache-persistence failure must not flip `check
        // --strict`'s exit code — extraction output itself is unaffected.
        assert_eq!(diagnostic.severity, crate::types::DiagnosticSeverity::Info);
        assert!(
            diagnostic.message.to_lowercase().contains("cache"),
            "expected the diagnostic to mention the cache, got: {}",
            diagnostic.message
        );
    }

    #[test]
    fn test_save_to_disk_returns_none_on_success() {
        let tmp = temp_dir("save-success");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));
        cache.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(cache.save_to_disk().is_none(), "expected no diagnostic on a successful save");
    }

    #[test]
    fn test_dirty_flag_prevents_unnecessary_write() {
        let tmp = temp_dir("dirty");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        assert!(!cache.dirty.load(std::sync::atomic::Ordering::Relaxed));
        cache.save_to_disk();
        // manifest should NOT exist because cache wasn't dirty
        assert!(!cache_dir.join("manifest.json").exists());

        let file_path = make_temp_file(&tmp, "dirty.d.ts", b"export type D = boolean;");
        cache.insert(&file_path, "export type D = boolean;", SourceData::default());
        assert!(cache.dirty.load(std::sync::atomic::Ordering::Relaxed));

        cache.save_to_disk();
        assert!(!cache.dirty.load(std::sync::atomic::Ordering::Relaxed));
        assert!(cache_dir.join("manifest.json").exists());
    }

    #[test]
    fn test_eviction_when_exceeding_max_entries() {
        let tmp = temp_dir("evict");
        let cache_dir = tmp.join("cache");
        let cache = DtsCache::load_from_disk(Some(&cache_dir));

        for i in 0..(MAX_CACHE_ENTRIES + 10) {
            let key = CacheKey { path: Utf8PathBuf::from(format!("file_{i}.d.ts")), content_hash: i as u64 };
            cache.store.insert(key, SourceData::default());
        }

        let file_path = make_temp_file(&tmp, "overflow.d.ts", b"export type O = string;");
        cache.insert(&file_path, "export type O = string;", SourceData::default());
        assert!(cache.store.len() <= MAX_CACHE_ENTRIES);
    }

    #[test]
    fn concurrent_saves_to_one_cache_dir_all_succeed_and_leave_no_temp_files() {
        // One DtsCache per thread, as separate processes would have.
        let tmp = temp_dir("concurrent");
        let cache_dir = tmp.join("cache");
        let writers = 16;

        let failures: Vec<Diagnostic> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..writers)
                .map(|i| {
                    let cache_dir = &cache_dir;
                    scope.spawn(move || {
                        let cache = DtsCache::load_from_disk(Some(cache_dir));
                        let content = format!("export type T{i} = string;");
                        cache.insert(Utf8Path::new(&format!("/virtual/t{i}.d.ts")), &content, SourceData::default());
                        cache.save_to_disk()
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(failures, vec![], "every concurrent save must succeed");

        let mut leftovers: Vec<String> = std::fs::read_dir(cache_dir.as_std_path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        leftovers.sort();
        assert_eq!(leftovers, vec![DATA_FILE.to_owned(), "manifest.json".to_owned()]);

        // Last rename wins. It may hold several writers' entries: a later writer's load starts from an earlier save.
        let reloaded = DtsCache::load_from_disk(Some(&cache_dir));
        let written: std::collections::BTreeSet<String> = (0..writers).map(|i| format!("/virtual/t{i}.d.ts")).collect();
        let loaded: std::collections::BTreeSet<String> =
            reloaded.store.iter().map(|r| r.key().path.to_string()).collect();
        assert!(!loaded.is_empty(), "the surviving cache file must decode to at least one entry");
        assert!(
            loaded.is_subset(&written),
            "unexpected entries: {:?}",
            loaded.difference(&written).collect::<Vec<_>>()
        );
    }
}
