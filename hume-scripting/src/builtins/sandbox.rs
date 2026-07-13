//! Directory TLS for HUME's scripting filesystem builtins.
//!
//! Full-trust plugin model (see `docs/ROADMAP.md`'s plugin trust model
//! decision) — this no longer enforces a sandbox. What remains is editor-
//! integration state that must be computed once and read from many builtins:
//! the display-form data/runtime dirs (`data-dir`/`runtime-dir`), and the
//! canonical `<data>/servers/` root the cross-process install lock
//! (`acquire-install-lock!`/`release-install-lock!`) needs pre-created and
//! canonicalized at init time (unlike plugin/grammar install targets, which
//! Steel's own `create-directory!`, or `git clone`'s own recursive parent
//! creation, now create on demand — see `lsp/install-github!`'s
//! `create-directory!` call and the `git clone` behavior it relies on).
//!
//! # Public surface (within the crate)
//!
//! | Item                          | Used by                           |
//! |-------------------------------|-----------------------------------|
//! | [`init_dirs`]                 | `ScriptingHost::new`, tests       |
//! | [`with_data_servers`]         | `install.rs` (install lock)        |

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use steel::rerrs::{ErrorKind, SteelErr};

// ── Permanent dirs TLS ────────────────────────────────────────────────────────

struct ScriptDirs {
    /// `<data>/hume/` as a *display* (non-UNC) path — what `(data-dir)` returns
    /// to Scheme.  On Windows the canonical form carries a `\\?\` prefix that
    /// the NT object manager does not accept with forward slashes, so we expose
    /// the plain drive-letter form instead (e.g. `C:\Users\…\hume`).
    data_dir_display: Option<PathBuf>,
    /// `<runtime>/` as a display path (same UNC reasoning).
    runtime_dir_display: Option<PathBuf>,
    /// Canonical `<data>/servers/` — the cross-process install lock's root.
    /// `None` when `data_dir` is unavailable or directory creation fails;
    /// lock operations fail closed in that case.
    data_servers: Option<PathBuf>,
}

thread_local! {
    static SCRIPT_DIRS: RefCell<Option<ScriptDirs>> = const { RefCell::new(None) };
}

/// Initialize the directory TLS.  Must be called exactly once during
/// [`crate::ScriptingHost::new`] before any builtins are invoked.
///
/// Eagerly creates `<data>/servers/` so the install lock has a canonical root
/// to work with from the first call. If creation or canonicalization fails,
/// `data_servers` is `None` and lock operations fail closed rather than
/// silently permitting writes to a bogus prefix.
pub fn init_dirs(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) {
    // <data>/servers/
    let data_servers = data_dir.as_ref().and_then(|d| {
        let s = d.join("servers");
        hume_platform::fs::create_dir_all(&s).ok()?;
        hume_platform::fs::canonicalize(&s).ok()
    });

    // Canonicalize data_dir for the display form; fall back to raw path when
    // the directory doesn't exist (e.g. sandboxed FS test environments).
    let canonical_data = data_dir.map(|d| hume_platform::fs::canonicalize(&d).unwrap_or(d));
    // Display form strips `\\?\` so Scheme can safely concatenate `/`-separated
    // segments on Windows without producing malformed extended-length paths.
    let data_dir_display = canonical_data.map(hume_platform::path::strip_unc_prefix);

    let canonical_runtime = runtime_dir.and_then(|rt| hume_platform::fs::canonicalize(&rt).ok());
    let runtime_dir_display = canonical_runtime.map(hume_platform::path::strip_unc_prefix);

    SCRIPT_DIRS.with(|cell| {
        *cell.borrow_mut() = Some(ScriptDirs {
            data_dir_display,
            runtime_dir_display,
            data_servers,
        });
    });
}

fn with_dirs<R>(f: impl FnOnce(&ScriptDirs) -> R) -> R {
    SCRIPT_DIRS.with(|cell| {
        let borrow = cell.borrow();
        f(borrow
            .as_ref()
            .expect("SCRIPT_DIRS not initialized — ScriptingHost::new() must call fs::init_dirs"))
    })
}

/// Access the display-form data directory — what `(data-dir)` returns to Scheme.
pub(crate) fn data_dir_display() -> Option<PathBuf> {
    with_dirs(|dirs| dirs.data_dir_display.clone())
}

/// Access the display-form runtime directory — what `(runtime-dir)` returns to Scheme.
pub(crate) fn runtime_dir_display() -> Option<PathBuf> {
    with_dirs(|dirs| dirs.runtime_dir_display.clone())
}

/// Call `f` with the canonical `<data>/servers/` root. Used by `install.rs`'s
/// cross-process install lock.
///
/// Returns `Err` when no data directory is available (HOME/APPDATA unset),
/// which fails the lock closed rather than silently permitting it.
pub(crate) fn with_data_servers<R>(f: impl FnOnce(&Path) -> R) -> Result<R, SteelErr> {
    with_dirs(|dirs| match dirs.data_servers.as_deref() {
        Some(p) => Ok(f(p)),
        None => Err(SteelErr::new(
            ErrorKind::Generic,
            "no data directory — HOME/APPDATA unset; server install operations unavailable"
                .to_string(),
        )),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_dirs_creates_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        // data_dir does not exist yet; init_dirs should create it.
        init_dirs(Some(data_dir.clone()), None);
        assert!(data_dir.join("servers").is_dir());
    }

    #[test]
    fn with_data_servers_errs_when_dirs_unavailable() {
        init_dirs(None, None);
        assert!(with_data_servers(|_| ()).is_err());
    }

    #[test]
    fn with_data_servers_succeeds_when_data_dir_available() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        init_dirs(Some(data_dir.clone()), None);
        let servers = std::fs::canonicalize(data_dir.join("servers")).unwrap();
        assert!(with_data_servers(|p| p == servers).unwrap());
    }
}
