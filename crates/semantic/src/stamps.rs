//! What the files a warm checker's snapshot read looked like on disk, so that an edit it was not told about is found
//! before it answers from them (ADR 0026 S6).
//!
//! The checker keeps every file a snapshot read and reads one again only when it is told the file changed: an edit it
//! is not told about is answered from the old text, with no sign. A stamp is a file's modification time and length as
//! the snapshot read it, taken when the snapshot was built; before the next answer every stamp is taken again and
//! compared. A file modified within [`MTIME_GRANULARITY`] of the build cannot be told apart from a later edit by its
//! stamp, so it is unsure and is read again at every update until it settles.

use std::time::{Duration, SystemTime};

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};

/// Filesystems that keep modification times to the second, or to a kernel clock tick, give an edit made just after a
/// read the same modification time.
const MTIME_GRANULARITY: Duration = Duration::from_secs(2);

/// Threads that take stamps at once: a Chakra program reads 1,855 files, 4 ms one at a time on macOS, 1.5 ms on four.
const STAMP_THREADS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stamp {
    Missing,
    Present {
        modified: SystemTime,
        len: u64,
    },
    /// Modified too close to the read, or unreadable: the snapshot may hold either version, so it reads it again.
    Unsure,
}

fn stamp(path: &Utf8Path) -> Stamp {
    match std::fs::metadata(path) {
        Ok(meta) => match meta.modified() {
            Ok(modified) => Stamp::Present { modified, len: meta.len() },
            Err(_) => Stamp::Unsure,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Stamp::Missing,
        Err(_) => Stamp::Unsure,
    }
}

/// `stamp` of every path, on [`STAMP_THREADS`] threads.
fn stamp_all<T: Send>(paths: Vec<Utf8PathBuf>, stamp: impl Fn(&Utf8Path) -> T + Sync) -> Vec<(Utf8PathBuf, T)> {
    let chunk = paths.len().div_ceil(STAMP_THREADS).max(1);
    std::thread::scope(|scope| {
        let stamp = &stamp;
        let workers: Vec<_> = paths
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(|path| (path.clone(), stamp(path))).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
            .collect()
    })
}

/// The stamps of the files a snapshot read, for a snapshot that started reading them at `read_from`. A path that is not
/// on the disk (a checker's built-in file) has no stamp.
pub(crate) fn record(files: Vec<Utf8PathBuf>, read_from: SystemTime) -> FxHashMap<Utf8PathBuf, Stamp> {
    let files = files.into_iter().filter(|file| file.is_absolute()).collect();
    stamp_all(files, |path| match stamp(path) {
        Stamp::Present { modified, .. } if modified + MTIME_GRANULARITY >= read_from => Stamp::Unsure,
        // Deleted after the snapshot read it, or never there: either way the checker is told it is gone.
        Stamp::Missing => Stamp::Unsure,
        stamp => stamp,
    })
    .into_iter()
    .collect()
}

/// How the files a snapshot read differ from the disk now.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Drift {
    /// Changed since the snapshot read them, and the checker was told: it reads them again.
    pub(crate) told: Vec<Utf8PathBuf>,
    /// Unsure when the snapshot read them: read again, whatever their stamp says now.
    pub(crate) unsure: Vec<Utf8PathBuf>,
    /// Changed since the snapshot read them, and nobody told the checker: the snapshot cannot be trusted.
    pub(crate) untold: Vec<Utf8PathBuf>,
}

/// The files of `stamps` whose stamp changed, split by whether `told` names them, and the unsure ones; each list sorted.
pub(crate) fn drift(stamps: &FxHashMap<Utf8PathBuf, Stamp>, told: &FxHashSet<Utf8PathBuf>) -> Drift {
    let mut drift = Drift::default();
    let (unsure, settled): (Vec<_>, Vec<_>) = stamps.iter().partition(|(_, stamp)| **stamp == Stamp::Unsure);
    drift.unsure = unsure.into_iter().map(|(path, _)| path.clone()).collect();
    let settled = settled.into_iter().map(|(path, _)| path.clone()).collect();
    for (path, now) in stamp_all(settled, stamp) {
        if stamps.get(&path) == Some(&now) {
            continue;
        }
        match told.contains(&path) {
            true => drift.told.push(path),
            false => drift.untold.push(path),
        }
    }
    for list in [&mut drift.told, &mut drift.unsure, &mut drift.untold] {
        list.sort();
    }
    drift
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        (dir, root)
    }

    /// Recorded as if the snapshot had read the files an hour after they were written, so their stamps are settled.
    fn settled(files: &[&Utf8PathBuf]) -> FxHashMap<Utf8PathBuf, Stamp> {
        record(files.iter().map(|file| (*file).clone()).collect(), SystemTime::now() + Duration::from_secs(3600))
    }

    #[test]
    fn an_edit_nobody_announced_is_untold_and_an_announced_one_is_told() {
        let (_dir, root) = dir();
        let (a, b, c) = (root.join("a.ts"), root.join("b.ts"), root.join("c.ts"));
        for file in [&a, &b, &c] {
            std::fs::write(file, "export {}").unwrap();
        }
        let stamps = settled(&[&a, &b, &c]);

        std::fs::write(&a, "export const a = 1").unwrap();
        std::fs::write(&b, "export const b = 1").unwrap();
        let drift = drift(&stamps, &FxHashSet::from_iter([a.clone()]));

        assert_eq!(drift, Drift { told: vec![a], unsure: vec![], untold: vec![b] });
    }

    #[test]
    fn a_deleted_file_is_a_change() {
        let (_dir, root) = dir();
        let a = root.join("a.ts");
        std::fs::write(&a, "export {}").unwrap();
        let stamps = settled(&[&a]);

        std::fs::remove_file(&a).unwrap();

        assert_eq!(drift(&stamps, &FxHashSet::default()).untold, [a]);
    }

    #[test]
    fn a_file_modified_as_the_snapshot_read_it_is_unsure_and_never_untold() {
        let (_dir, root) = dir();
        let a = root.join("a.ts");
        std::fs::write(&a, "export {}").unwrap();
        let stamps = record(vec![a.clone(), Utf8PathBuf::from("bundled:///libs/lib.d.ts")], SystemTime::now());

        std::fs::write(&a, "export const a = 1").unwrap();

        assert_eq!(stamps.len(), 1, "a path that is not on the disk has no stamp");
        assert_eq!(drift(&stamps, &FxHashSet::default()), Drift { unsure: vec![a], ..Drift::default() });
    }
}
