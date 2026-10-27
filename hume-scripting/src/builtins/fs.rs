//! Editor-integration filesystem builtins for HUME's Steel scripting engine.
//!
//! Full-trust plugin model (see `user-manual/docs/writing-plugins.md`'s
//! "Filesystem and processes"): general filesystem access goes through Steel's own
//! `steel/filesystem`/`steel/ports` stdlib, not a HUME builtin. What remains
//! here is editor-integration info Steel can't derive on its own,
//! `path-join`'s Windows UNC-prefix stripping (Steel's own
//! `canonicalize-path` leaks `\\?\`, which external tools reject), and what
//! the session has to own: a tree delete the budget does not count, a
//! directory swap the budget cannot split, file locks released when the
//! session ends, and a program lookup on the editor's search path.
//!
//! The path arguments of `set-cwd!`, `replace-dir!`, `remove-dir!`,
//! the lock pair and `path-entries` are
//! [`ScriptPath`](super::args::ScriptPath)s.
//!
//! | Steel name      | Signature                      | Notes                                        |
//! |-----------------|--------------------------------|----------------------------------------------|
//! | `data-dir`      | `() → string \| #f`            | HUME data directory (XDG), or `#f` if unset  |
//! | `runtime-dir`   | `() → string \| #f`            | Runtime dir, or `#f` if absent               |
//! | `cwd`           | `() → string`                  | The editor's working directory               |
//! | `set-cwd!`      | `string → void`                | `:cd`: moves the editor's working directory  |
//! | `replace-dir!`  | `string string → void`         | Moves a directory into another's place, all or nothing |
//! | `remove-dir!`   | `string → void`                | Deletes a directory tree, budget stopped     |
//! | `try-lock-file!` | `string → bool`               | Exclusive OS lock, held until `unlock-file!` or the session ends |
//! | `unlock-file!`  | `string → void`                | Releases a lock this session took            |
//! | `path-join`     | `string… → string`             | OS-native join; no sandbox, no filesystem access |
//! | `path->display` | `string → string`               | UNC-strip + `~`-collapse; no filesystem access |
//! | `safe-path-segment?` | `any → bool`              | `#t` for a string safe as one path segment   |
//! | `path-entries`  | `string string → list`         | What a typed path completes to, as `:e` lists it |
//! | `path-programs` | `string list proc → job`       | Lists the search path's programs by name prefix in the background |
//! | `find-program`  | `string → string \| #f`        | Where a program runs from, on the search path |
//! | `home-dir`      | `() → string \| #f`            | The user's home directory                    |
//!
//! `path-separator` is *not* registered here: steel-core's `steel/meta`
//! module already provides it as a bare global (`Engine::new()` baseline),
//! and HUME registering its own would silently shadow it.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use hume_platform::path::AbsolutePath;

use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;
use hume_core::Severity;

use super::SteelResult;
use super::args::{list_to_strings, string_list};
use super::errors::{generic_err, require_cap};

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// Shared body for the builtins returning a path: `dir`'s display-form path
/// as a string, or `#f` if `dir` is `None`.
fn dir_builtin(dir: Option<&Path>) -> SteelResult {
    match dir {
        Some(p) => p
            .to_string_lossy()
            .as_ref()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(data-dir)`: returns the HUME data directory as a string, or `#f` if
/// HOME/APPDATA is unset.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows) so Scheme plugins can safely join segments with `(path-join …)`
/// or, if necessary, plain string concatenation.
pub(crate) fn data_dir(ctx: &mut SteelCtx) -> SteelResult {
    dir_builtin(ctx.dirs.data_dir_display.as_deref())
}

/// `(runtime-dir)`: returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows).
pub(crate) fn runtime_dir(ctx: &mut SteelCtx) -> SteelResult {
    dir_builtin(ctx.dirs.runtime_dir_display.as_deref())
}

/// `(cwd)`: the editor's working directory as a string: the base `:e` and
/// every spawned child resolve against. Display form, like `(data-dir)`.
pub(crate) fn cwd(ctx: &mut SteelCtx) -> SteelResult {
    dir_builtin(Some(ctx.host.buffers().cwd().as_path()))
}

/// `(set-cwd! path)`: moves the editor's working directory and the process
/// one with it, like `:cd path`: a relative `path` joins onto the current
/// one, and the result is canonicalized. Raises when `path` is not a
/// directory.
pub(crate) fn set_cwd(ctx: &mut SteelCtx, path: AbsolutePath) -> SteelResult {
    ctx.host.buffers().set_cwd(&path).map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(replace-dir! src dest)`: moves the directory `src` to `dest` (relative
/// paths join onto `(cwd)`), replacing whatever directory `dest` held and
/// creating `dest`'s parent when absent. Either `src` ends up at `dest`, or
/// the call raises with `dest` as it was. One builtin call runs to the end
/// even when the budget expires during it, so a stopped script cannot leave
/// `dest` deleted and `src` unmoved. `dest`'s old contents move aside to a
/// sibling `<name>.replaced` and are deleted last, with the budget stopped;
/// a backup a crash left there is cleared first. A backup that cannot be
/// deleted is logged as a warning and the call still succeeds, since `src`
/// is in place by then.
pub(crate) fn replace_dir(
    ctx: &mut SteelCtx,
    src: AbsolutePath,
    dest: AbsolutePath,
) -> SteelResult {
    let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
        return Err(generic_err(format!(
            "{} names no directory to replace",
            dest.display()
        )));
    };
    let mut backup_name = name.to_os_string();
    backup_name.push(".replaced");
    let backup = parent.join(backup_name);
    ctx.while_waiting(|| {
        std::fs::create_dir_all(parent)?;
        swap_dir_in(&src, &dest, &backup)
    })
    .map_err(|e| generic_err(format!("{} -> {}: {e}", src.display(), dest.display())))?;
    if let Err(e) = ctx.while_waiting(|| remove_dir_tree_if_present(&backup)) {
        ctx.log(
            Severity::Warning,
            format!(
                "replace-dir!: {} was replaced, but its old contents in {} were not deleted: {e}",
                dest.display(),
                backup.display()
            ),
        );
    }
    Ok(SteelVal::Void)
}

/// Moves `src` to `dest`, parking what `dest` held at `backup`, which stays
/// there for the caller to delete. A `backup` already present is deleted
/// first. When `src` cannot move, `dest` is put back; if that fails too, the
/// error names both failures and where the old contents are.
fn swap_dir_in(src: &Path, dest: &Path, backup: &Path) -> std::io::Result<()> {
    remove_dir_tree_if_present(backup)?;
    let had_dest = match std::fs::rename(dest, backup) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    let Err(e) = std::fs::rename(src, dest) else {
        return Ok(());
    };
    if had_dest && let Err(restore) = std::fs::rename(backup, dest) {
        return Err(std::io::Error::new(
            e.kind(),
            format!(
                "{e}; putting {} back from {} failed too: {restore}",
                dest.display(),
                backup.display()
            ),
        ));
    }
    Err(e)
}

/// `(remove-dir! dir)`: deletes the directory `dir` and everything in it
/// (a missing `dir` raises), with the session's budget stopped while the OS
/// works. A relative `dir` joins onto `(cwd)`.
pub(crate) fn remove_dir(ctx: &mut SteelCtx, dir: AbsolutePath) -> SteelResult {
    ctx.while_waiting(|| std::fs::remove_dir_all(&dir))?;
    Ok(SteelVal::Void)
}

fn remove_dir_tree_if_present(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// `(try-lock-file! path)`: takes the exclusive OS lock on the file `path`
/// (a relative one joins onto `(cwd)`), creating the file when absent:
/// `#t` when taken, `#f` when another holder has it. The lock lasts until
/// `unlock-file!` or the end of the session, whichever comes first, and the
/// OS drops it when the process exits, so no lock is ever left over. Taking
/// a lock this session already holds raises.
pub(crate) fn try_lock_file(ctx: &mut SteelCtx, path: AbsolutePath) -> SteelResult {
    if ctx.file_locks.contains_key(path.as_path()) {
        return Err(generic_err(format!(
            "this session already holds {}",
            path.display()
        )));
    }
    let lock_err = |e: std::io::Error| generic_err(format!("{}: {e}", path.display()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(lock_err)?;
    match file.try_lock() {
        Ok(()) => {
            ctx.file_locks.insert(path.to_path_buf(), HeldLock(file));
            Ok(SteelVal::BoolV(true))
        }
        Err(TryLockError::WouldBlock) => Ok(SteelVal::BoolV(false)),
        Err(TryLockError::Error(e)) => Err(lock_err(e)),
    }
}

/// A file whose OS lock this process holds, released when dropped.
///
/// The lock belongs to the open file, which a child forked from any thread
/// shares until it starts its program, so closing this handle alone could
/// leave the lock held that long. Unlocking first releases it at once.
pub(crate) struct HeldLock(File);

impl Drop for HeldLock {
    fn drop(&mut self) {
        // Holding the lock means the unlock cannot fail, and the close that
        // follows releases it anyway.
        let _ = self.0.unlock();
    }
}

/// `(unlock-file! path)`: releases the lock `try-lock-file!` took on `path`
/// this session. Raises when this session does not hold it.
pub(crate) fn unlock_file(ctx: &mut SteelCtx, path: AbsolutePath) -> SteelResult {
    ctx.file_locks
        .remove(path.as_path())
        .map(|_| SteelVal::Void)
        .ok_or_else(|| generic_err(format!("this session does not hold {}", path.display())))
}

// ── Completion listings ──────────────────────────────────────────────────────

/// `(path-entries dir prefix)`: the names in `dir` (relative to `(cwd)`)
/// that a typed path ending in `prefix` completes to, as `:e` lists them:
/// [`hume_platform::path::completion_entries`]'s rule.
pub(crate) fn path_entries(_ctx: &mut SteelCtx, dir: AbsolutePath, prefix: String) -> SteelResult {
    Ok(string_list(hume_platform::path::completion_entries(
        &dir, &prefix, false,
    )))
}

/// `(path-programs prefix extensions on-names)`: lists the programs on the
/// editor's search path whose names start with `prefix` in the background,
/// calling `(on-names names done?)` as they are found
/// ([`AsyncProcessHost::scan_programs`](crate::host::AsyncProcessHost::scan_programs)),
/// and returns the job `cancel-async!` stops it by. `extensions` (`".exe"`,
/// …) are the file extensions that name a program on Windows.
pub(crate) fn path_programs(
    ctx: &mut SteelCtx,
    prefix: String,
    extensions: SteelVal,
    on_names: crate::Callback,
) -> SteelResult {
    let extensions = list_to_strings(extensions, "extensions")?;
    let job = require_cap(ctx.host.async_process())?
        .scan_programs(prefix, extensions, on_names)
        .map_err(generic_err)?;
    Ok(job.into_steel_val())
}

/// `(find-program name)`: the path of the program `name` runs as, looked up
/// on the editor's search path (the `PATH` HUME started with, which every
/// program HUME starts gets), a relative entry taken from `(cwd)`. `#f` when
/// there is none, or no search path.
pub(crate) fn find_program(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let buffers = ctx.host.buffers();
    let found = buffers
        .search_path()
        .and_then(|path| ::which::which_in(&name, Some(path), buffers.cwd()).ok());
    dir_builtin(found.as_deref())
}

/// `(home-dir)`: the user's home directory, or `#f` when the environment
/// names none.
pub(crate) fn home_dir() -> SteelResult {
    dir_builtin(hume_platform::dirs::home_dir().as_deref())
}

// ── path-join ─────────────────────────────────────────────────────────────────

/// `(path-join seg1 seg2 …)`: join path segments using the OS-native
/// separator and return the result as a string.
///
/// Uses `PathBuf::push` semantics: if any segment is an absolute path it
/// replaces everything to the left (the same rule as `Path::join`).  This
/// lets plugins build paths portably without hard-coding `"/"` or `"\\"`.
///
/// No sandbox check. This is a pure string-construction helper that does not
/// access the filesystem.
pub(crate) fn path_join(args: &[SteelVal]) -> SteelResult {
    if args.is_empty() {
        steel::stop!(ArityMismatch => "expects at least 1 arg, got 0");
    }
    let mut result = PathBuf::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            SteelVal::StringV(s) => result.push(s.as_str()),
            _ => steel::stop!(TypeMismatch =>
                "arg {} must be a string, got {:?}", i, arg),
        }
    }
    result
        .to_string_lossy()
        .as_ref()
        .into_steelval()
        .map_err(generic_err)
}

// ── safe-path-segment? ────────────────────────────────────────────────────────

/// `(safe-path-segment? name)`: `#t` iff `name` is a string that is safe to use
/// as one filesystem path segment, per `hume_platform::path::is_safe_segment`.
/// Any other value answers `#f`, so a plugin can hand it an unchecked argument.
///
/// `:` is rejected because on Windows a segment like `c:evil` after a single
/// path component makes `PathBuf::push` treat it as a drive-relative root,
/// replacing the base path instead of joining onto it.
pub(crate) fn safe_path_segment(args: &[SteelVal]) -> SteelResult {
    let [name] = args else {
        steel::stop!(ArityMismatch =>
            "expects 1 arg, got {}", args.len());
    };
    Ok(SteelVal::BoolV(
        matches!(name, SteelVal::StringV(s) if hume_platform::path::is_safe_segment(s.as_str())),
    ))
}

// ── path->display ─────────────────────────────────────────────────────────────

/// `(path->display path)`: run `path` through HUME's display-form pipeline
/// (Windows `\\?\` stripping, `~`-collapse) so a path a plugin only has as a
/// string (e.g. `lsp/place-display`'s `path->display` call on the
/// already-decoded path `lsp-locations->places` hands it) renders the
/// same way as paths that came from `buffer-display-path`. No filesystem
/// access; expects an absolute path (`~`-collapse is a no-op on relative
/// input).
pub(crate) fn path_to_display(args: &[SteelVal]) -> SteelResult {
    if args.len() != 1 {
        steel::stop!(ArityMismatch => "expects exactly 1 arg, got {}", args.len());
    }
    let SteelVal::StringV(s) = &args[0] else {
        steel::stop!(TypeMismatch => "arg must be a string, got {:?}", args[0]);
    };
    hume_platform::path::display_form(Path::new(s.as_str()))
        .into_steelval()
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
