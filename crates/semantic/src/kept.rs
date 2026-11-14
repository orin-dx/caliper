//! The whole-project answer kept on disk between `extract` runs ([`crate::resolve_cached`]), and what it depends on.
//!
//! The linked checker reads the disk only through the file system it is given, so a run hands it one that notes every
//! question it asks and a digest of each answer: whether a path is a file or a directory, a file's text, a directory's
//! entries, a path's metadata and its real path, failed lookups included (the paths module resolution tried and did not
//! find). That is everything outside this build that the answer can depend on, the compiler options included (the
//! tsconfig and what it extends are read the same way); the build itself, the pinned checker with it, is in the header
//! ([`Header`]), and the request (the components, the project root, the tsconfig's path) is in the key. A later run
//! asks each question again of the disk, and only when every answer is the same is the kept answer used: any change
//! anywhere the checker looked, a new file where it found none included, discards it whole. Nothing is tracked per
//! component (ADR 0026: a component's dependency closure is not known), so this is conservative by construction.
//!
//! A question answered two ways in one run (a file edited while the checker read it) keeps nothing, and so does a run
//! that reports a diagnostic (a checker that did not start, a request that timed out).

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use caliper_docgen_core::types::semantic::SemanticOutcome;
use camino::Utf8Path;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use tsrs_vfs::{Entries, FS, FileInfo, osvfs};
use xxhash_rust::xxh3::Xxh3;

use crate::SemanticRequest;

/// The file the answer is kept in, in the cache directory the parse cache uses.
pub(crate) const FILE: &str = "checker-answers.msgpack";

/// Bump when the file's layout changes in a way the fingerprint would not show.
const FORMAT: u32 = 1;

/// The largest kept answer read back: `gate-large`'s is about 10 MB.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// A question the checker asked of the file system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Question {
    FileExists(String),
    ReadFile(String),
    DirectoryExists(String),
    Entries(String),
    Stat(String),
    Realpath(String),
}

/// The questions a run's checker asked, each with a digest of its answer.
#[derive(Default)]
pub(crate) struct Reads {
    asked: Mutex<FxHashMap<Question, u128>>,
    /// A question was answered two ways, or the checker wrote: what it read is not one state of the disk.
    unsettled: AtomicBool,
}

impl Reads {
    fn note(&self, question: Question, answer: u128) {
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        match asked.get(&question) {
            Some(known) if *known != answer => self.unsettled.store(true, Ordering::Relaxed),
            Some(_) => {}
            None => {
                asked.insert(question, answer);
            }
        }
    }

    /// Keeps the run's answer from being kept: the checker was refused a file it read (see `checker::TsrsSession`), so
    /// its answer is not the one the disk's files make.
    pub(crate) fn spoil(&self) {
        self.unsettled.store(true, Ordering::Relaxed);
    }

    /// Every question and answer digest, sorted, when the run read one state of the disk.
    fn settled(&self) -> Option<Vec<(Question, u128)>> {
        if self.unsettled.load(Ordering::Relaxed) {
            return None;
        }
        let asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        let mut reads: Vec<(Question, u128)> =
            asked.iter().map(|(question, answer)| (question.clone(), *answer)).collect();
        reads.sort_by(|a, b| a.0.cmp_key().cmp(&b.0.cmp_key()));
        Some(reads)
    }
}

impl Question {
    fn cmp_key(&self) -> (u8, &str) {
        match self {
            Question::FileExists(path) => (0, path),
            Question::ReadFile(path) => (1, path),
            Question::DirectoryExists(path) => (2, path),
            Question::Entries(path) => (3, path),
            Question::Stat(path) => (4, path),
            Question::Realpath(path) => (5, path),
        }
    }

    /// The digest of what `fs` answers to this question now.
    fn answer(&self, fs: &dyn FS) -> u128 {
        match self {
            Question::FileExists(path) => flag(fs.file_exists(path)),
            Question::ReadFile(path) => text(fs.read_file(path).as_deref()),
            Question::DirectoryExists(path) => flag(fs.directory_exists(path)),
            Question::Entries(path) => entries(&fs.get_accessible_entries(path)),
            Question::Stat(path) => stat(fs.stat(path).as_ref()),
            Question::Realpath(path) => text(Some(&fs.realpath(path))),
        }
    }
}

fn flag(answer: bool) -> u128 {
    u128::from(answer)
}

fn text(answer: Option<&str>) -> u128 {
    let mut hasher = Xxh3::new();
    match answer {
        Some(text) => {
            hasher.update(&[1]);
            hasher.update(text.as_bytes());
        }
        None => hasher.update(&[0]),
    }
    hasher.digest128()
}

/// The entries in the order the file system gave them, each list closed by its length, symlinks included.
fn entries(answer: &Entries) -> u128 {
    let mut hasher = Xxh3::new();
    let mut list = |names: &mut dyn Iterator<Item = &String>| {
        let mut count = 0u64;
        for name in names {
            hasher.update(name.as_bytes());
            hasher.update(&[0]);
            count += 1;
        }
        hasher.update(&count.to_le_bytes());
    };
    list(&mut answer.files.iter());
    list(&mut answer.directories.iter());
    match &answer.symlinks {
        Some(symlinks) => {
            let mut sorted: Vec<&String> = symlinks.iter().collect();
            sorted.sort();
            list(&mut sorted.into_iter());
        }
        None => list(&mut std::iter::once(&String::from("\u{0}none"))),
    }
    hasher.digest128()
}

/// Every field of the metadata, the modification time included: a file touched since is a change, though its text may
/// not be.
fn stat(answer: Option<&FileInfo>) -> u128 {
    let mut hasher = Xxh3::new();
    match answer {
        Some(info) => {
            hasher.update(&[1]);
            hasher.update(info.name.as_bytes());
            hasher.update(&[0]);
            hasher.update(&info.size.to_le_bytes());
            hasher.update(&info.mode.bits().to_le_bytes());
            let modified = info.mod_time.and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok());
            hasher.update(&modified.map_or(u128::MAX, |since| since.as_nanos()).to_le_bytes());
        }
        None => hasher.update(&[0]),
    }
    hasher.digest128()
}

/// The file system the linked checker reads through, noting each question it asks of `inner` and the answer's digest
/// in `reads`.
pub(crate) struct Recording<T> {
    pub(crate) inner: T,
    pub(crate) reads: Arc<Reads>,
}

impl<T: FS> FS for Recording<T> {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.inner.use_case_sensitive_file_names()
    }

    fn file_exists(&self, path: &str) -> bool {
        let answer = self.inner.file_exists(path);
        self.reads.note(Question::FileExists(path.to_string()), flag(answer));
        answer
    }

    fn read_file(&self, path: &str) -> Option<String> {
        let answer = self.inner.read_file(path);
        self.reads.note(Question::ReadFile(path.to_string()), text(answer.as_deref()));
        answer
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.reads.unsettled.store(true, Ordering::Relaxed);
        self.inner.write_file(path, data)
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.reads.unsettled.store(true, Ordering::Relaxed);
        self.inner.append_file(path, data)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        self.reads.unsettled.store(true, Ordering::Relaxed);
        self.inner.remove(path)
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.reads.unsettled.store(true, Ordering::Relaxed);
        self.inner.chtimes(path, a_time, m_time)
    }

    fn directory_exists(&self, path: &str) -> bool {
        let answer = self.inner.directory_exists(path);
        self.reads.note(Question::DirectoryExists(path.to_string()), flag(answer));
        answer
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        let answer = self.inner.get_accessible_entries(path);
        self.reads.note(Question::Entries(path.to_string()), entries(&answer));
        answer
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        let answer = self.inner.stat(path);
        self.reads.note(Question::Stat(path.to_string()), stat(answer.as_ref()));
        answer
    }

    fn realpath(&self, path: &str) -> String {
        let answer = self.inner.realpath(path);
        self.reads.note(Question::Realpath(path.to_string()), text(Some(&answer)));
        answer
    }
}

/// What a kept answer was made by: this build (its sources, the locked dependencies and with them the pinned checker),
/// and the request.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Header {
    format: u32,
    version: String,
    fingerprint: String,
    request: u128,
}

impl Header {
    fn of(request: u128) -> Header {
        Header {
            format: FORMAT,
            version: env!("CARGO_PKG_VERSION").to_string(),
            fingerprint: env!("CALIPER_SEMANTIC_FINGERPRINT").to_string(),
            request,
        }
    }
}

/// A digest of everything in the request that the answer depends on: the components with all the structural pass
/// says of them, the project root (the checker's working directory), the tsconfig's path and the query order.
pub(crate) fn request_key(request: &SemanticRequest) -> u128 {
    fn sorted(names: &rustc_hash::FxHashSet<String>) -> Vec<&String> {
        let mut names: Vec<&String> = names.iter().collect();
        names.sort();
        names
    }
    let targets: Vec<_> = request
        .targets
        .iter()
        .map(|target| {
            let wanted = target.react_wanted.as_ref().map(sorted);
            (&target.key, &target.file_path, &target.display_name, &target.exports, sorted(&target.described), wanted)
        })
        .collect();
    let described = format!("{:?}", (&request.project_root, &request.tsconfig, targets, request.reverse_queries));
    let mut hasher = Xxh3::new();
    hasher.update(described.as_bytes());
    hasher.digest128()
}

/// The answer kept at `file` for the request `key` names, when every question its checker asked of the disk has the
/// same answer now; `None` otherwise, or when there is none, or it cannot be read.
pub(crate) fn load(file: &Utf8Path, key: u128) -> Option<SemanticOutcome> {
    let size = std::fs::metadata(file).ok()?.len();
    if size > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    let mut decoder = rmp_serde::Deserializer::new(Cursor::new(bytes.as_slice()));
    if Header::deserialize(&mut decoder).ok()? != Header::of(key) {
        return None;
    }
    let reads = Vec::<(Question, u128)>::deserialize(&mut decoder).ok()?;
    let at = usize::try_from(decoder.get_ref().position()).ok()?;
    if !still_holds(&reads) {
        return None;
    }
    SemanticOutcome::from_bytes(bytes.get(at..)?).ok()
}

/// Whether the disk answers every question as it did, asked on as many threads as the machine has.
fn still_holds(reads: &[(Question, u128)]) -> bool {
    let fs = osvfs::fs();
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get()).min(16);
    let chunk = reads.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let checks: Vec<_> = reads
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().all(|(question, answer)| question.answer(fs) == *answer)))
            .collect();
        checks.into_iter().all(|check| check.join().unwrap_or(false))
    })
}

/// Keeps `outcome` at `file` for the request `key` names, with the questions `reads` noted. Nothing is kept for an
/// outcome with a diagnostic or a run that read more than one state of the disk; a failure to write is logged and
/// otherwise ignored (the next run asks the checker again).
pub(crate) fn save(file: &Utf8Path, key: u128, reads: &Reads, outcome: &SemanticOutcome) {
    if !outcome.diagnostics.is_empty() {
        return;
    }
    let Some(reads) = reads.settled() else {
        tracing::debug!("the checker read a file that changed during the run; its answer is not kept");
        return;
    };
    let written = (|| -> Result<(), String> {
        let mut bytes = rmp_serde::to_vec(&Header::of(key)).map_err(|error| error.to_string())?;
        bytes.extend(rmp_serde::to_vec(&reads).map_err(|error| error.to_string())?);
        bytes.extend(outcome.to_bytes()?);
        let dir = file.parent().ok_or_else(|| "no directory".to_string())?;
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        // A name of its own per process, so two runs sharing the directory do not write one temporary file.
        let temporary = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        let result = std::fs::write(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, file));
        if result.is_err() {
            std::fs::remove_file(&temporary).ok();
        }
        result.map_err(|error| error.to_string())
    })();
    if let Err(error) = written {
        tracing::debug!("could not keep the checker's answer at {file}: {error}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use caliper_docgen_core::pipeline::{PipelineOptions, extract};
    use camino::Utf8PathBuf;

    use super::*;
    use crate::{resolve, resolve_cached, targets_of};

    /// A project of two components, one of which imports a module that does not exist yet, and a tsconfig; and a cache
    /// directory beside it (one inside it would be a new entry of a directory the checker lists).
    fn project() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
        let dir = tempfile::Builder::new().prefix("Kept").tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap()).unwrap();
        let root = base.join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("tsconfig.json"), r#"{ "compilerOptions": { "strict": true, "jsx": "preserve" } }"#)
            .unwrap();
        std::fs::write(root.join("types.ts"), "export type Size = 'sm' | 'lg';\n").unwrap();
        std::fs::write(
            root.join("Button.tsx"),
            "import type { Size } from './types';\n\
             export interface ButtonProps { size?: Size; label: string }\n\
             export function Button(props: ButtonProps) { return null; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Card.tsx"),
            "import type { Tone } from './tone';\n\
             export function Card(props: { tone?: Tone; title: string }) { return null; }\n",
        )
        .unwrap();
        (dir, root, base.join("cache"))
    }

    fn request(root: &Utf8Path) -> SemanticRequest {
        let output = extract(&PipelineOptions { src_dirs: vec![root.to_owned()], ..PipelineOptions::default() });
        let mut request = SemanticRequest::new(root.to_owned(), targets_of(&output));
        request.tsconfig = Some(root.join("tsconfig.json"));
        request
    }

    /// The kept answer, if it still holds for `root`'s request now.
    fn kept(root: &Utf8Path, cache: &Utf8Path) -> Option<SemanticOutcome> {
        load(&cache.join(FILE), request_key(&request(root)))
    }

    #[test]
    fn a_rerun_over_the_same_disk_is_given_the_answer_a_fresh_run_gives() {
        let (_dir, root, cache) = project();

        let first = resolve_cached(request(&root), &cache);
        let again = kept(&root, &cache).expect("the answer was kept and still holds");
        let fresh = resolve(request(&root));

        assert!(first.diagnostics.is_empty() && first.components.len() == 2, "{first:?}");
        assert_eq!(again, fresh);
        assert_eq!(resolve_cached(request(&root), &cache), fresh);
    }

    #[test]
    fn an_edit_to_a_file_the_checker_read_discards_the_answer() {
        let (_dir, root, cache) = project();
        resolve_cached(request(&root), &cache);
        assert!(kept(&root, &cache).is_some());

        std::fs::write(root.join("types.ts"), "export type Size = 'sm' | 'md' | 'lg';\n").unwrap();

        assert!(kept(&root, &cache).is_none());
        let rerun = resolve_cached(request(&root), &cache);
        assert_eq!(rerun, resolve(request(&root)));
        let size = rerun.components.iter().flat_map(|c| &c.props).find(|p| p.name == "size").unwrap();
        assert!(format!("{:?}", size.prop_type).contains("md"), "{size:?}");
    }

    /// A file where the checker looked and found none (`./tone` did not resolve) changes the answer when it appears,
    /// though no file the checker read changed.
    #[test]
    fn a_new_file_where_the_checker_found_none_discards_the_answer() {
        let (_dir, root, cache) = project();
        resolve_cached(request(&root), &cache);
        assert!(kept(&root, &cache).is_some());

        std::fs::write(root.join("tone.ts"), "export type Tone = 'calm' | 'loud';\n").unwrap();

        assert!(kept(&root, &cache).is_none());
    }

    /// The root files handed out while the structural pass runs are the ones the finished output gives, in the same
    /// order: the program the early start opens is the one the run would open, files that share a component name
    /// (suffixed with their file) and a file that exports nothing included.
    #[test]
    fn the_root_files_handed_out_early_are_those_the_finished_output_gives() {
        let (_dir, root, _cache) = project();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/Button.tsx"), "export function Button(props: { x?: number }) { return null; }\n")
            .unwrap();
        std::fs::write(
            root.join("Alpha.tsx"),
            "function Hidden(props: { y: string }) { return null; }\nexport const A = 1;\n",
        )
        .unwrap();
        std::fs::write(root.join("Zed.tsx"), "export const Zed = (props: { z: string }) => null;\n").unwrap();
        let options = PipelineOptions { src_dirs: vec![root.clone()], ..PipelineOptions::default() };
        let mut early = None;

        let output = caliper_docgen_core::pipeline::extract_with_early_roots(&options, |roots| early = Some(roots));

        let mut request = SemanticRequest::new(root.clone(), targets_of(&output));
        request.tsconfig = Some(root.join("tsconfig.json"));
        let later: Vec<String> = crate::root_files(&request);
        let early: Vec<String> = early.expect("handed out").iter().map(ToString::to_string).collect();
        assert!(later.len() >= 4, "{later:?}");
        assert_eq!(early, later);
    }

    /// A program opened early gives the answer a run that opens it itself gives, is kept the same way, and is left
    /// unused (not failed on) when its root files are not the request's.
    #[test]
    fn an_early_start_gives_the_answer_a_run_gives_and_a_wrong_one_is_dropped() {
        let (_dir, root, cache) = project();
        let timeout = std::time::Duration::from_secs(60);
        let roots: Vec<Utf8PathBuf> = crate::root_files(&request(&root)).into_iter().map(Utf8PathBuf::from).collect();
        let fresh = resolve(request(&root));

        let early = crate::start_early(root.clone(), Some(root.join("tsconfig.json")), timeout, &roots, true);
        assert_eq!(crate::resolve_cached_early(request(&root), &cache, early), fresh);
        assert!(kept(&root, &cache).is_some(), "the answer from an early start is kept");

        std::fs::remove_file(cache.join(FILE)).unwrap();
        let wrong = crate::start_early(root.clone(), None, timeout, &roots[..1], true);
        assert_eq!(crate::resolve_cached_early(request(&root), &cache, wrong), fresh);
    }

    #[test]
    fn a_change_to_the_compiler_options_discards_the_answer() {
        let (_dir, root, cache) = project();
        resolve_cached(request(&root), &cache);
        assert!(kept(&root, &cache).is_some());

        std::fs::write(root.join("tsconfig.json"), r#"{ "compilerOptions": { "strict": false, "jsx": "preserve" } }"#)
            .unwrap();

        assert!(kept(&root, &cache).is_none());
    }

    /// Another build of Caliper or of the pinned checker (its fingerprint covers the locked `tsrs` version) does not
    /// read an answer this one kept.
    #[test]
    fn an_answer_another_build_kept_is_not_read() {
        let (_dir, root, cache) = project();
        resolve_cached(request(&root), &cache);
        assert!(kept(&root, &cache).is_some());
        let file = cache.join(FILE);
        let bytes = std::fs::read(&file).unwrap();
        let mut decoder = rmp_serde::Deserializer::new(Cursor::new(bytes.as_slice()));
        let mut header = Header::deserialize(&mut decoder).unwrap();
        let rest = bytes.get(usize::try_from(decoder.get_ref().position()).unwrap()..).unwrap().to_vec();

        header.fingerprint = "another-checker-pin".to_string();
        let mut rewritten = rmp_serde::to_vec(&header).unwrap();
        rewritten.extend(rest);
        std::fs::write(&file, rewritten).unwrap();

        assert!(kept(&root, &cache).is_none());
    }

    #[test]
    fn another_request_does_not_read_the_answer() {
        let (_dir, root, cache) = project();
        resolve_cached(request(&root), &cache);
        let mut other = request(&root);
        other.targets.retain(|target| target.key == "Button");

        assert!(load(&cache.join(FILE), request_key(&other)).is_none());
        assert!(kept(&root, &cache).is_some());
    }

    /// A file the checker is refused for its nesting (`checker::MAX_NESTING`) makes an answer that is not the disk's
    /// files' answer, so none is kept, and a run after it asks the checker again.
    #[test]
    fn a_run_that_was_refused_a_file_keeps_no_answer() {
        let (_dir, root, cache) = project();
        let depth = crate::checker::MAX_NESTING + 1;
        std::fs::write(
            root.join("tone.ts"),
            format!("export type Tone = {}string{};\n", "{ a: ".repeat(depth), " }".repeat(depth)),
        )
        .unwrap();

        let refused = resolve_cached(request(&root), &cache);

        assert!(refused.components.is_empty() || !refused.fallbacks.is_empty() || !refused.diagnostics.is_empty());
        assert!(!cache.join(FILE).exists());
    }

    /// A run of two programs (40 files, past `MIN_FILES_PER_PROGRAM` twice): the second program's snapshot is derived
    /// from the first's and reads its files from that cache, not from the disk, so the questions noted must still
    /// cover a file only one component imports, whichever program answers it. Editing it discards the kept answer.
    #[test]
    fn an_edit_to_a_file_one_component_of_a_two_program_run_imports_discards_the_answer() {
        let (_dir, root, cache) = project();
        for index in 0..38 {
            std::fs::write(
                root.join(format!("C{index}.tsx")),
                format!("export function C{index}(props: {{ label: string }}) {{ return null; }}\n"),
            )
            .unwrap();
        }
        std::fs::write(root.join("last.ts"), "export type Last = 'a' | 'b';\n").unwrap();
        std::fs::write(
            root.join("Z.tsx"),
            "import type { Last } from './last';\nexport function Z(props: { last: Last }) { return null; }\n",
        )
        .unwrap();
        let first = resolve_cached(request(&root), &cache);
        assert_eq!(first.components.len(), 41, "{:?}", first.fallbacks);
        assert!(kept(&root, &cache).is_some());

        std::fs::write(root.join("last.ts"), "export type Last = 'a' | 'b' | 'c';\n").unwrap();

        assert!(kept(&root, &cache).is_none());
        let rerun = resolve_cached(request(&root), &cache);
        let last = rerun.components.iter().find(|c| c.key == "Z").unwrap();
        assert!(format!("{:?}", last.props[0].prop_type).contains("\"c\""), "{last:?}");
    }
}
