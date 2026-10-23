//! Editor-integration filesystem builtins for HUME's Steel scripting engine.
//!
//! Full-trust plugin model (see `user-manual/docs/writing-plugins.md`'s
//! "Filesystem and processes"): general filesystem access goes through Steel's own
//! `steel/filesystem`/`steel/ports` stdlib, not a HUME builtin. What remains
//! here is editor-integration info Steel can't derive on its own,
//! `path-join`'s Windows UNC-prefix stripping (Steel's own
//! `canonicalize-path` leaks `\\?\`, which external tools reject), and what
//! the session has to own: a tree delete the budget does not count, a
//! directory swap the budget cannot split, and file locks released when the
//! session ends.
//!
//! A relative path these builtins take resolves through
//! [`editor_path`], as `:e` resolves a typed one.
//!
//! | Steel name      | Signature                      | Notes                                        |
//! |-----------------|--------------------------------|----------------------------------------------|
//! | `data-dir`      | `() → string \| #f`            | HUME data directory (XDG), or `#f` if unset  |
//! | `runtime-dir`   | `() → string \| #f`            | Runtime dir, or `#f` if absent               |
//! | `cwd`           | `() → string`                  | The editor's working directory               |
//! | `set-cwd!`      | `string → void`                | `:cd`: moves the editor's working directory  |
//! | `delete-dir-tree!` | `string → void`             | Recursive delete, budget stopped while it runs |
//! | `replace-dir!`  | `string string → void`         | Moves a directory into another's place, all or nothing |
//! | `%delete-directory!` | `string → void`           | Steel's `delete-directory!`, budget stopped; backs its shim |
//! | `try-lock-file!` | `string → bool`               | Exclusive OS lock, held until `unlock-file!` or the session ends |
//! | `unlock-file!`  | `string → void`                | Releases a lock this session took            |
//! | `path-join`     | `string… → string`             | OS-native join; no sandbox, no filesystem access |
//! | `path->display` | `string → string`               | UNC-strip + `~`-collapse; no filesystem access |
//! | `safe-path-segment?` | `any → bool`              | `#t` for a string safe as one path segment   |
//! | `path-entries`  | `string string → list`         | What a typed path completes to, as `:e` lists it |
//! | `path-programs` | `string list → list`           | Programs on the search path by name prefix   |
//! | `%which`        | `string → string \| #f`        | Where a program runs from; backs the `which` shim |
//! | `home-dir`      | `() → string \| #f`            | The user's home directory                    |
//!
//! `path-separator` is *not* registered here: steel-core's `steel/meta`
//! module already provides it as a bare global (`Engine::new()` baseline),
//! and HUME registering its own would silently shadow it.

use std::fs::TryLockError;
use std::path::{Path, PathBuf};

use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;

use super::SteelResult;
use super::args::{editor_path, list_to_strings, string_list};
use super::errors::generic_err;

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
/// Steel's own `current-directory` reads the process cwd: `:cd` moves that
/// too, but a plugin that moves it with `change-current-directory!` leaves
/// the editor's cwd behind.
pub(crate) fn cwd(ctx: &mut SteelCtx) -> SteelResult {
    let cwd = hume_platform::path::strip_unc_prefix(ctx.host.buffers().cwd());
    dir_builtin(Some(&cwd))
}

/// `(set-cwd! path)`: moves the editor's working directory, like
/// `:cd path` (a relative `path` joins onto the current one). Raises when
/// `path` is not a directory. Never calls Steel's `change-current-directory!`.
pub(crate) fn set_cwd(ctx: &mut SteelCtx, path: String) -> SteelResult {
    ctx.host
        .buffers()
        .set_cwd(Path::new(&path))
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(delete-dir-tree! path)`: removes the directory `path` (a relative one
/// joins onto `(cwd)`) and everything under it; an absent `path` is a no-op.
/// The removal is a wait on the OS that can take seconds for a large tree,
/// so it runs with the session's budget stopped, unlike Steel's own
/// `delete-directory!`.
pub(crate) fn delete_dir_tree(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let dir = ctx.host.buffers().cwd().join(&path);
    ctx.while_waiting(|| remove_dir_tree_if_present(&dir))
        .map(|()| SteelVal::Void)
        .map_err(|e| generic_err(format!("delete-dir-tree!: {path}: {e}")))
}

/// `(replace-dir! src dest)`: moves the directory `src` to `dest` (relative
/// paths join onto `(cwd)`), replacing whatever directory `dest` held and
/// creating `dest`'s parent when absent. Either `src` ends up at `dest`, or
/// the call raises with `dest` as it was. One builtin call runs to the end
/// even when the budget expires during it, so a stopped script cannot leave
/// `dest` deleted and `src` unmoved. `dest`'s old contents move aside to a
/// sibling `<name>.replaced` and are deleted last, with the budget stopped;
/// a backup a crash left there is cleared first.
pub(crate) fn replace_dir(ctx: &mut SteelCtx, src: String, dest: String) -> SteelResult {
    let cwd = ctx.host.buffers().cwd();
    let (src_path, dest_path) = (
        editor_path(Path::new(&src), &cwd),
        editor_path(Path::new(&dest), &cwd),
    );
    let (Some(parent), Some(name)) = (dest_path.parent(), dest_path.file_name()) else {
        return Err(generic_err(format!(
            "replace-dir!: {dest} names no directory to replace"
        )));
    };
    let mut backup_name = name.to_os_string();
    backup_name.push(".replaced");
    let backup = parent.join(backup_name);
    ctx.while_waiting(|| {
        std::fs::create_dir_all(parent)?;
        remove_dir_tree_if_present(&backup)?;
        let had_dest = match std::fs::rename(&dest_path, &backup) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        if let Err(e) = std::fs::rename(&src_path, &dest_path) {
            if had_dest {
                std::fs::rename(&backup, &dest_path)?;
            }
            return Err(e);
        }
        remove_dir_tree_if_present(&backup)
    })
    .map(|()| SteelVal::Void)
    .map_err(|e| generic_err(format!("replace-dir!: {src} -> {dest}: {e}")))
}

/// `(%delete-directory! dir)`, which the prelude's `delete-directory!`
/// calls: Steel's own `delete-directory!` (a relative `dir` resolves against
/// the process cwd, and a missing one raises), run with the session's budget
/// stopped.
pub(crate) fn delete_directory(ctx: &mut SteelCtx, dir: String) -> SteelResult {
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
pub(crate) fn try_lock_file(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let file_path = editor_path(Path::new(&path), &ctx.host.buffers().cwd());
    if ctx.file_locks.contains_key(&file_path) {
        return Err(generic_err(format!(
            "try-lock-file!: this session already holds {path}"
        )));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&file_path)
        .map_err(|e| generic_err(format!("try-lock-file!: {path}: {e}")))?;
    match file.try_lock() {
        Ok(()) => {
            ctx.file_locks.insert(file_path, file);
            Ok(SteelVal::BoolV(true))
        }
        Err(TryLockError::WouldBlock) => Ok(SteelVal::BoolV(false)),
        Err(TryLockError::Error(e)) => Err(generic_err(format!("try-lock-file!: {path}: {e}"))),
    }
}

/// `(unlock-file! path)`: releases the lock `try-lock-file!` took on `path`
/// this session. Raises when this session does not hold it.
pub(crate) fn unlock_file(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let file_path = editor_path(Path::new(&path), &ctx.host.buffers().cwd());
    ctx.file_locks
        .remove(&file_path)
        .map(|_| SteelVal::Void)
        .ok_or_else(|| generic_err(format!("unlock-file!: this session does not hold {path}")))
}

// ── Completion listings ──────────────────────────────────────────────────────

/// `(path-entries dir prefix)`: the names in `dir` (relative to `(cwd)`)
/// that a typed path ending in `prefix` completes to, as `:e` lists them:
/// [`hume_platform::path::completion_entries`]'s rule.
pub(crate) fn path_entries(ctx: &mut SteelCtx, dir: String, prefix: String) -> SteelResult {
    let dir = editor_path(Path::new(&dir), &ctx.host.buffers().cwd());
    Ok(string_list(hume_platform::path::completion_entries(
        &dir, &prefix, false,
    )))
}

/// `(path-programs prefix extensions)`: the programs on the editor's search
/// path whose names start with `prefix`; `extensions` (`".exe"`, …) are the
/// file extensions that name a program on Windows.
pub(crate) fn path_programs(
    ctx: &mut SteelCtx,
    prefix: String,
    extensions: SteelVal,
) -> SteelResult {
    let extensions = list_to_strings(extensions, "path-programs")?;
    Ok(programs_on(
        ctx.host.buffers().search_path().as_deref(),
        &prefix,
        &extensions,
    ))
}

/// `(%which name)`, which the prelude's `which` calls: the program `name`
/// runs as, found by steel-core's own `which` lookup but on the editor's
/// search path, with a relative entry taken from `(cwd)`. `#f` when there is
/// none, or no search path.
pub(crate) fn which_program(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let buffers = ctx.host.buffers();
    let found = buffers
        .search_path()
        .and_then(|path| ::which::which_in(&name, Some(path), buffers.cwd()).ok());
    dir_builtin(found.as_deref())
}

/// Testable core of [`path_programs`]: [`hume_platform::path::executables_in`]
/// over `search_path`, `'()` without one.
fn programs_on(
    search_path: Option<&std::ffi::OsStr>,
    prefix: &str,
    extensions: &[String],
) -> SteelVal {
    string_list(search_path.map_or_else(Vec::new, |path| {
        hume_platform::path::executables_in(path, prefix, extensions)
    }))
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
        steel::stop!(ArityMismatch => "path-join expects at least 1 arg, got 0");
    }
    let mut result = PathBuf::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            SteelVal::StringV(s) => result.push(s.as_str()),
            _ => steel::stop!(TypeMismatch =>
                "path-join: arg {} must be a string, got {:?}", i, arg),
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
            "safe-path-segment? expects 1 arg, got {}", args.len());
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
        steel::stop!(ArityMismatch => "path->display expects exactly 1 arg, got {}", args.len());
    }
    let SteelVal::StringV(s) = &args[0] else {
        steel::stop!(TypeMismatch => "path->display: arg must be a string, got {:?}", args[0]);
    };
    hume_platform::path::display_form(Path::new(s.as_str()))
        .into_steelval()
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
