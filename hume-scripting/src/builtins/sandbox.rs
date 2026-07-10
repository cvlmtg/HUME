//! Directory TLS and path-sandbox predicates for HUME's scripting filesystem builtins.
//!
//! **Security invariant** (see `feedback_security_canonicalize`): every
//! `canonicalize` call on an untrusted path must hard-fail (return `None` or
//! propagate `Err`) on any error — never fall back to the unresolved path.
//!
//! All four sandbox roots (`data_plugins`, `data_grammars`, `data_servers`,
//! `runtime_plugins`) are set to `None` when the underlying directory cannot
//! be created or canonicalized. Sandbox checks against a `None` root fail
//! closed.
//!
//! # Public surface (within the crate)
//!
//! | Item                          | Used by                           |
//! |-------------------------------|-----------------------------------|
//! | [`init_dirs`]                 | `ScriptingHost::new`, tests       |
//! | [`with_data_plugins`]         | `shell.rs` git/curl operations    |
//! | [`with_data_grammars`]        | `shell.rs`, `grammar.rs`          |
//! | [`with_data_grammars_or_subpath`] | `grammar.rs`                  |
//! | [`with_data_servers`]         | `shell.rs`, `install.rs` (LSP server installs) |

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
    /// Canonical `<data>/plugins/` — sandbox root for plugin operations.
    /// `None` when `data_dir` is unavailable or directory creation fails;
    /// write sandbox checks fail closed.
    data_plugins: Option<PathBuf>,
    /// Canonical `<data>/grammars/` — sandbox root for grammar operations.
    /// `None` when `data_dir` is unavailable or directory creation fails.
    data_grammars: Option<PathBuf>,
    /// Canonical `<data>/servers/` — sandbox root for LSP server install
    /// operations (downloads, unpacking, npm installs, receipts).
    /// `None` when `data_dir` is unavailable or directory creation fails.
    data_servers: Option<PathBuf>,
    /// Canonical `<runtime>/plugins/` — allowed for read-path ops only.
    runtime_plugins: Option<PathBuf>,
}

thread_local! {
    static SCRIPT_DIRS: RefCell<Option<ScriptDirs>> = const { RefCell::new(None) };
}

/// Initialize the directory TLS.  Must be called exactly once during
/// [`crate::ScriptingHost::new`] before any builtins are invoked.
///
/// Eagerly creates `<data>/plugins/`, `<data>/grammars/`,
/// `<data>/grammars/sources/`, and `<data>/servers/` so grammar, plugin, and
/// server-install paths exist before any sandbox check runs.  If creation or
/// canonicalization fails for a sandbox root, that root is set to `None` and
/// the corresponding write operations fail closed rather than silently
/// permitting writes to a bogus prefix.
pub fn init_dirs(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) {
    // Eagerly create sandbox subdirs from the raw data_dir path before
    // canonicalizing, so a first-run (dir doesn't exist yet) gets the dirs.
    // Each step is fail-closed: if creation or canonicalize fails the sandbox
    // root is None and write operations will be rejected.

    // <data>/plugins/
    let data_plugins = data_dir.as_ref().and_then(|d| {
        let p = d.join("plugins");
        hume_platform::fs::create_dir_all(&p).ok()?;
        hume_platform::fs::canonicalize(&p).ok()
    });

    // <data>/grammars/ and <data>/grammars/sources/
    let data_grammars = data_dir.as_ref().and_then(|d| {
        let g = d.join("grammars");
        hume_platform::fs::create_dir_all(&g.join("sources")).ok()?;
        hume_platform::fs::canonicalize(&g).ok()
    });

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
    let runtime_dir_display = canonical_runtime
        .as_ref()
        .map(|rt| hume_platform::path::strip_unc_prefix(rt.clone()));
    let runtime_plugins = canonical_runtime
        .as_ref()
        .and_then(|rt| hume_platform::fs::canonicalize(&rt.join("plugins")).ok());
    // Store canonical forms for sandbox prefix checks; display forms for Scheme
    // consumption.  If the runtime dir doesn't exist leave it as None.
    SCRIPT_DIRS.with(|cell| {
        *cell.borrow_mut() = Some(ScriptDirs {
            data_dir_display,
            runtime_dir_display,
            data_plugins,
            data_grammars,
            data_servers,
            runtime_plugins,
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

/// Call `f` with the canonical write-sandbox root (`<data>/plugins/`).
/// Used by `shell.rs` to sandbox git operations.
///
/// Returns `Err` when no data directory is available (HOME/APPDATA unset),
/// which fails the write sandbox check closed rather than silently permitting it.
pub(crate) fn with_data_plugins<R>(f: impl FnOnce(&Path) -> R) -> Result<R, SteelErr> {
    with_dirs(|dirs| match dirs.data_plugins.as_deref() {
        Some(p) => Ok(f(p)),
        None => Err(SteelErr::new(
            ErrorKind::Generic,
            "no data directory — HOME/APPDATA unset; write operations unavailable".to_string(),
        )),
    })
}

/// Call `f` with the canonical write-sandbox root (`<data>/grammars/`).
/// Used by grammar builtins to sandbox compile/fetch operations.
pub(crate) fn with_data_grammars<R>(f: impl FnOnce(&Path) -> R) -> Result<R, SteelErr> {
    with_dirs(|dirs| match dirs.data_grammars.as_deref() {
        Some(p) => Ok(f(p)),
        None => Err(SteelErr::new(
            ErrorKind::Generic,
            "no data directory — HOME/APPDATA unset; grammar operations unavailable".to_string(),
        )),
    })
}

/// Call `f` with the canonical write-sandbox root (`<data>/servers/`).
/// Used by `shell.rs`/`install.rs` to sandbox LSP server install operations
/// (downloads, unpacking, npm installs, receipts).
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

/// Call `f` with `<data>/grammars/<seg>` where `seg` must be a safe, single
/// path component (no `..`, no `.`, no separators).
pub(crate) fn with_data_grammars_or_subpath<R>(
    seg: &str,
    f: impl FnOnce(&Path) -> R,
) -> Result<R, SteelErr> {
    if !hume_platform::path::is_safe_segment(seg) {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!(
                "invalid path segment '{seg}': must be a single normal component (no '..' / '.' / separators)"
            ),
        ));
    }
    with_data_grammars(|grammars| f(&grammars.join(seg)))
}

// ── Sandbox predicates ────────────────────────────────────────────────────────

pub(crate) fn is_under_write_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| {
        dirs.data_plugins
            .as_deref()
            .is_some_and(|p| canonical.starts_with(p))
            || dirs
                .data_grammars
                .as_deref()
                .is_some_and(|g| canonical.starts_with(g))
            || dirs
                .data_servers
                .as_deref()
                .is_some_and(|s| canonical.starts_with(s))
    })
}

/// Returns `true` when `canonical` is inside `<data>/grammars/`.
///
/// Used by operations whose sandbox is narrower than the full write sandbox.
pub(crate) fn is_under_grammars_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| {
        dirs.data_grammars
            .as_deref()
            .is_some_and(|g| canonical.starts_with(g))
    })
}

/// Returns `true` when `canonical` is inside `<data>/servers/`.
pub(crate) fn is_under_servers_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| {
        dirs.data_servers
            .as_deref()
            .is_some_and(|s| canonical.starts_with(s))
    })
}

/// Returns `true` when `canonical` is inside `<data>/grammars/` or
/// `<data>/servers/` — the "install" sandbox shared by operations that touch
/// both artifact classes (`curl-fetch`, `write-file`, `delete-file`).
pub(crate) fn is_under_install_sandbox(canonical: &Path) -> bool {
    is_under_grammars_sandbox(canonical) || is_under_servers_sandbox(canonical)
}

pub(crate) fn is_under_read_sandbox(canonical: &Path) -> bool {
    // Write sandbox ⊆ read sandbox; extend with the read-only runtime root.
    is_under_write_sandbox(canonical)
        || with_dirs(|dirs| {
            dirs.runtime_plugins
                .as_deref()
                .is_some_and(|rp| canonical.starts_with(rp))
        })
}

// Thin re-exports so crate-internal callers resolve without import churn until
// the call sites are updated in a follow-on step.
pub(crate) use hume_platform::path::has_dotdot;
pub(crate) use hume_platform::path::normalize_lexical;

/// Canonicalize the deepest existing ancestor of `path`, then rejoin any
/// non-existing suffix components.
///
/// Used by [`super::fs::make_dir`] to sandbox-check a path that does not yet
/// exist.  The `..` rejection in `make_dir` (via [`has_dotdot`]) prevents
/// traversal attacks that this function cannot catch.
///
/// Returns `None` if no ancestor exists at all (e.g. a completely bogus path).
pub(crate) fn canonical_ancestor_join(path: &Path) -> Option<PathBuf> {
    let mut suffix = vec![];
    let mut current = path;
    // Walk up until we find a component that exists on disk.
    loop {
        if hume_platform::fs::exists(current) {
            break;
        }
        suffix.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
    let canonical_base = hume_platform::fs::canonicalize(current).ok()?;
    let mut result = canonical_base;
    for component in suffix.into_iter().rev() {
        result.push(component);
    }
    Some(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_dirs_creates_grammars_sources() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        // data_dir does not exist yet; init_dirs should create the subdirs.
        init_dirs(Some(data_dir.clone()), None);
        assert!(data_dir.join("plugins").is_dir());
        assert!(data_dir.join("grammars").is_dir());
        assert!(data_dir.join("grammars/sources").is_dir());
        assert!(data_dir.join("servers").is_dir());
    }

    // ── canonical_ancestor_join ───────────────────────────────────────────────

    #[test]
    fn canonical_ancestor_join_resolves_existing_parent() {
        let tmp = TempDir::new().unwrap();
        // "sub/leaf" does not exist yet; the function must canonicalize tmp
        // and rejoin the non-existing suffix.
        let target = tmp.path().join("sub").join("leaf");
        let result = canonical_ancestor_join(&target).unwrap();
        let canonical_tmp = std::fs::canonicalize(tmp.path()).unwrap();
        // Result must be rooted at the canonical tmp dir and end with the
        // non-existing suffix components.
        assert!(
            result.starts_with(&canonical_tmp),
            "{result:?} must start with {canonical_tmp:?}"
        );
        assert_eq!(result, canonical_tmp.join("sub").join("leaf"));
    }

    #[test]
    fn canonical_ancestor_join_existing_path_returns_canonical() {
        let tmp = TempDir::new().unwrap();
        let result = canonical_ancestor_join(tmp.path()).unwrap();
        let expected = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(result, expected);
    }

    // ── fail-closed sandbox predicates (TLS) ─────────────────────────────────
    //
    // Each test calls init_dirs() at the start to reset TLS state for this
    // thread; tests that need a None root pass (None, None).

    #[test]
    fn sandbox_predicates_fail_closed_when_dirs_unavailable() {
        // Simulate HOME/APPDATA unset — all roots become None.
        init_dirs(None, None);
        // Every path must be denied; fail-closed is the security invariant.
        assert!(!is_under_write_sandbox(Path::new("/any/path")));
        assert!(!is_under_grammars_sandbox(Path::new("/any/path")));
        assert!(!is_under_servers_sandbox(Path::new("/any/path")));
        assert!(!is_under_install_sandbox(Path::new("/any/path")));
        assert!(!is_under_read_sandbox(Path::new("/any/path")));
    }

    #[test]
    fn with_data_plugins_errs_when_dirs_unavailable() {
        init_dirs(None, None);
        assert!(with_data_plugins(|_| ()).is_err());
    }

    #[test]
    fn with_data_grammars_errs_when_dirs_unavailable() {
        init_dirs(None, None);
        assert!(with_data_grammars(|_| ()).is_err());
    }

    #[test]
    fn with_data_servers_errs_when_dirs_unavailable() {
        init_dirs(None, None);
        assert!(with_data_servers(|_| ()).is_err());
    }

    // ── servers sandbox (new root) ────────────────────────────────────────────

    #[test]
    fn is_under_servers_sandbox_accepts_path_inside_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        init_dirs(Some(data_dir.clone()), None);
        let inside = std::fs::canonicalize(data_dir.join("servers"))
            .unwrap()
            .join("rust-analyzer");
        assert!(is_under_servers_sandbox(&inside));
        assert!(is_under_write_sandbox(&inside));
        assert!(is_under_install_sandbox(&inside));
        assert!(is_under_read_sandbox(&inside));
    }

    #[test]
    fn is_under_servers_sandbox_rejects_path_outside_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        init_dirs(Some(data_dir.clone()), None);
        let outside = std::fs::canonicalize(data_dir.join("plugins")).unwrap();
        assert!(!is_under_servers_sandbox(&outside));
    }

    #[test]
    fn is_under_install_sandbox_accepts_grammars_and_servers_not_plugins() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("hume");
        init_dirs(Some(data_dir.clone()), None);
        let grammars = std::fs::canonicalize(data_dir.join("grammars")).unwrap();
        let servers = std::fs::canonicalize(data_dir.join("servers")).unwrap();
        let plugins = std::fs::canonicalize(data_dir.join("plugins")).unwrap();
        assert!(is_under_install_sandbox(&grammars));
        assert!(is_under_install_sandbox(&servers));
        assert!(!is_under_install_sandbox(&plugins));
    }

    // ── with_data_grammars_or_subpath segment validation ─────────────────────
    //
    // The segment-validation rejects before touching TLS, so these tests work
    // with an uninitialised root (None, None) without interfering with the TLS
    // state of other tests.

    #[test]
    fn grammars_or_subpath_rejects_dotdot() {
        init_dirs(None, None);
        assert!(with_data_grammars_or_subpath("..", |_| ()).is_err());
    }

    #[test]
    fn grammars_or_subpath_rejects_cur_dir() {
        init_dirs(None, None);
        assert!(with_data_grammars_or_subpath(".", |_| ()).is_err());
    }

    #[test]
    fn grammars_or_subpath_rejects_nested_path() {
        // Two normal components ("foo/bar") are not a single segment.
        init_dirs(None, None);
        assert!(with_data_grammars_or_subpath("foo/bar", |_| ()).is_err());
    }
}
