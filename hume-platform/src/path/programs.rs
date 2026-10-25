//! The programs on a search path that a shell runs by name.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::name_starts_with;
use crate::snapshots::{Snapshots, spawn_snapshots};
use crate::wake::WakeCallback;

/// Program names gathered from search path directories in path order,
/// each listed once.
///
/// On Unix a program is a file (a symlink is followed) with any execute
/// bit set, and `extensions` is unused. On Windows it is a file whose name
/// ends with one of `extensions` (`.exe`, as `sh` adds to a bare name, or
/// `PATHEXT`'s list), named by its stem. Names match a prefix as the file
/// system compares them, without regard to ASCII case on Windows, where
/// names differing only in ASCII case are listed once. The spelling listed
/// is the first one scanned.
#[derive(Default)]
struct ProgramNames {
    /// Keyed by the name as the file system compares it, which on Unix is
    /// the listed name itself.
    by_key: BTreeMap<Arc<str>, Arc<str>>,
}

impl ProgramNames {
    /// Adds the programs in `dir` whose names start with `prefix` and are
    /// not listed yet. `true` when it added any. A directory that can't be
    /// read adds nothing. Stops before the next entry once `live` is
    /// `false`.
    fn scan_dir(
        &mut self,
        dir: &Path,
        prefix: &str,
        extensions: &[impl AsRef<str>],
        live: impl Fn() -> bool,
    ) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut added = false;
        for entry in entries.flatten() {
            if !live() {
                break;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            // On Unix a program is keyed by its file name and the first one
            // on the path wins, so a name already listed skips the `stat`.
            if cfg!(unix) && self.by_key.contains_key(name) {
                continue;
            }
            let Some(program) = program_name(&entry, name, prefix, extensions) else {
                continue;
            };
            let program: Arc<str> = program.into();
            let key = if cfg!(windows) {
                program.to_ascii_lowercase().into()
            } else {
                Arc::clone(&program)
            };
            if let Entry::Vacant(slot) = self.by_key.entry(key) {
                slot.insert(program);
                added = true;
            }
        }
        added
    }

    /// Every name listed, sorted.
    fn sorted(&self) -> Arc<[Arc<str>]> {
        self.by_key.values().cloned().collect()
    }
}

/// Lists the programs on `search_path` (a `PATH` value) whose names start
/// with `prefix`, on a background thread. A relative entry joins onto `cwd`,
/// as it does when the program is run or looked up, an empty one is skipped
/// on Windows, and one that cannot be made absolute is skipped. The scan reports the sorted names so far after each
/// directory that adds one, and once the returned [`Snapshots`] is dropped it
/// reads no further directory.
pub fn spawn_program_scan(
    search_path: Option<OsString>,
    cwd: PathBuf,
    prefix: String,
    extensions: Vec<String>,
    wake: WakeCallback,
) -> io::Result<Snapshots<Arc<[Arc<str>]>>> {
    spawn_snapshots("hume-program-scan", wake, move |reporter| {
        let mut names = ProgramNames::default();
        for entry in search_path.iter().flat_map(std::env::split_paths) {
            if !reporter.is_live() {
                return;
            }
            if cfg!(windows) && entry.as_os_str().is_empty() {
                continue;
            }
            let Ok(dir) = super::absolute(&entry, &cwd) else {
                continue;
            };
            if names.scan_dir(&dir, &prefix, &extensions, || reporter.is_live()) {
                reporter.send(names.sorted());
            }
        }
    })
}

/// The name a shell runs directory entry `entry` (named `name`) by, when
/// it is a program whose name starts with `prefix`.
#[cfg(unix)]
fn program_name<'a>(
    entry: &std::fs::DirEntry,
    name: &'a str,
    prefix: &str,
    _extensions: &[impl AsRef<str>],
) -> Option<&'a str> {
    use std::os::unix::fs::PermissionsExt;
    if !name_starts_with(name, prefix) {
        return None;
    }
    // The entry's own type is free from the listing on most filesystems,
    // and only a file or a symlink can be a program, so a directory or a
    // device never costs a `stat`. A file is read through its directory
    // entry, and only a symlink is followed by path.
    let file_type = entry.file_type().ok()?;
    let meta = if file_type.is_file() {
        entry.metadata()
    } else if file_type.is_symlink() {
        std::fs::metadata(entry.path())
    } else {
        return None;
    }
    .ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(name)
}

/// The name a shell runs directory entry `entry` (named `name`) by, when
/// it is a program whose name starts with `prefix`.
#[cfg(windows)]
fn program_name<'a>(
    entry: &std::fs::DirEntry,
    name: &'a str,
    prefix: &str,
    extensions: &[impl AsRef<str>],
) -> Option<&'a str> {
    if !name_starts_with(name, prefix) {
        return None;
    }
    let stem = extensions.iter().find_map(|ext| {
        let ext = ext.as_ref();
        let split = name
            .len()
            .checked_sub(ext.len())
            .filter(|&split| split > 0)?;
        let tail = name.get(split..)?;
        tail.eq_ignore_ascii_case(ext).then(|| &name[..split])
    })?;
    // The entry's type comes with the listing, so this costs no file open.
    (name_starts_with(stem, prefix) && !entry.file_type().ok()?.is_dir()).then_some(stem)
}

#[cfg(test)]
mod tests;
