//! Directory TLS and path-sandbox predicates for HUME's scripting filesystem builtins.
//!
//! **Security invariant** (see `feedback_security_canonicalize`): every
//! `canonicalize` call on an untrusted path must hard-fail (return `None` or
//! propagate `Err`) on any error — never fall back to the unresolved path.
//!
//! All three sandbox roots (`data_plugins`, `data_grammars`, `runtime_plugins`) are
//! set to `None` when the underlying directory cannot be created or canonicalized.
//! Sandbox checks against a `None` root fail closed.
//!
//! # Public surface (within the crate)
//!
//! | Item                          | Used by                           |
//! |-------------------------------|-----------------------------------|
//! | [`init_dirs`]                 | `ScriptingHost::new`, tests       |
//! | [`with_data_plugins`]         | `shell.rs` git/curl operations    |
//! | [`with_data_grammars`]        | `shell.rs`, `grammar.rs`          |
//! | [`with_data_grammars_or_subpath`] | `grammar.rs`                  |
//! | [`has_dotdot`]                | `shell.rs`, `grammar.rs`, `fs.rs` |

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

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
    /// Canonical `<runtime>/plugins/` — allowed for read-path ops only.
    runtime_plugins: Option<PathBuf>,
}

thread_local! {
    static SCRIPT_DIRS: RefCell<Option<ScriptDirs>> = const { RefCell::new(None) };
}

// ── UNC prefix stripping ──────────────────────────────────────────────────────

/// Strip the `\\?\` extended-length prefix from a Windows path so that the
/// result is a plain drive-letter path (e.g. `C:\Users\…\hume`).
///
/// Plain drive paths accept forward slashes from Scheme's `string-append`;
/// `\\?\`-prefixed paths go through the NT object manager directly and are
/// strict about backslashes.  Scheme plugins build paths via `(path-join …)`
/// which uses the native separator, but the display form must be prefix-free
/// so that even old-style string concatenation doesn't produce malformed paths.
///
/// Only strips verbatim drive prefixes (`\\?\C:\…`).  Verbatim UNC paths
/// (`\\?\UNC\…`) are left unchanged; they are rare and the `\\` prefix they
/// collapse to is already a valid UNC path.
///
/// On non-Windows targets this is a no-op.
#[cfg(windows)]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    const VERBATIM: &str = r"\\?\";
    match p.to_str() {
        Some(s) if s.starts_with(VERBATIM) && !s[VERBATIM.len()..].starts_with("UNC\\") => {
            PathBuf::from(&s[VERBATIM.len()..])
        }
        _ => p,
    }
}

#[cfg(not(windows))]
#[inline]
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    p
}

/// Initialize the directory TLS.  Must be called exactly once during
/// [`crate::ScriptingHost::new`] before any builtins are invoked.
///
/// Eagerly creates `<data>/plugins/`, `<data>/grammars/`, and
/// `<data>/grammars/sources/` so grammar and plugin paths exist before any
/// sandbox check runs.  If creation or canonicalization fails for a sandbox
/// root, that root is set to `None` and the corresponding write operations
/// fail closed rather than silently permitting writes to a bogus prefix.
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

    // Canonicalize data_dir for the display form; fall back to raw path when
    // the directory doesn't exist (e.g. sandboxed FS test environments).
    let canonical_data =
        data_dir.map(|d| hume_platform::fs::canonicalize(&d).unwrap_or(d));
    // Display form strips `\\?\` so Scheme can safely concatenate `/`-separated
    // segments on Windows without producing malformed extended-length paths.
    let data_dir_display = canonical_data.map(strip_unc_prefix);

    let canonical_runtime = runtime_dir.and_then(|rt| hume_platform::fs::canonicalize(&rt).ok());
    let runtime_dir_display = canonical_runtime
        .as_ref()
        .map(|rt| strip_unc_prefix(rt.clone()));
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

/// Call `f` with `<data>/grammars/<seg>` where `seg` must be a single Normal
/// path component (no `..`, no `.`, no separators).
pub(crate) fn with_data_grammars_or_subpath<R>(
    seg: &str,
    f: impl FnOnce(&Path) -> R,
) -> Result<R, SteelErr> {
    let seg_path = std::path::Path::new(seg);
    let mut comps = seg_path.components();
    let valid = matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    );
    if !valid {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!("invalid path segment '{seg}': must be a single normal component (no '..' / '.' / separators)"),
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
    })
}

/// Returns `true` when `canonical` is inside `<data>/grammars/`.
///
/// Used by `delete-file`, which has a narrower sandbox than `delete-dir`.
pub(crate) fn is_under_grammars_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| {
        dirs.data_grammars
            .as_deref()
            .is_some_and(|g| canonical.starts_with(g))
    })
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

/// Returns `true` if `path` contains any `..` (ParentDir) components.
///
/// Used for write-path ops where the target may not exist yet — we cannot
/// call `canonicalize` on a non-existent path, so we reject `..` components
/// explicitly before the `starts_with` prefix check.
pub(crate) fn has_dotdot(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

/// Normalize a path lexically (without filesystem access) by collapsing `.`
/// and `..` components.
///
/// **Not a security substitute for `canonicalize`** (symlinks are not
/// resolved).  Safe to use only when combined with an explicit `..`-rejection
/// check via [`has_dotdot`].
pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

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
    }

    // ── has_dotdot ────────────────────────────────────────────────────────────

    #[test]
    fn has_dotdot_detects_bare_parent() {
        assert!(has_dotdot(Path::new("..")));
    }

    #[test]
    fn has_dotdot_detects_mid_path_parent() {
        assert!(has_dotdot(Path::new("foo/../bar")));
    }

    #[test]
    fn has_dotdot_does_not_flag_cur_dir() {
        // "." is CurDir, not ParentDir — has_dotdot only guards against "..".
        assert!(!has_dotdot(Path::new(".")));
        assert!(!has_dotdot(Path::new("foo/./bar")));
    }

    #[test]
    fn has_dotdot_clean_path_is_false() {
        assert!(!has_dotdot(Path::new("foo/bar/baz")));
    }

    // ── normalize_lexical ─────────────────────────────────────────────────────

    #[test]
    fn normalize_lexical_removes_cur_dir() {
        assert_eq!(normalize_lexical(Path::new("a/./b")), PathBuf::from("a/b"));
    }

    #[test]
    fn normalize_lexical_pops_parent_dir() {
        assert_eq!(
            normalize_lexical(Path::new("a/b/../c")),
            PathBuf::from("a/c")
        );
    }

    #[test]
    fn normalize_lexical_pop_on_empty_is_safe() {
        // Leading ".." when the output is empty: pop() on an empty PathBuf is a
        // no-op, so the ".." is silently discarded and only "a" survives.
        assert_eq!(normalize_lexical(Path::new("../a")), PathBuf::from("a"));
    }

    #[test]
    fn normalize_lexical_normal_path_unchanged() {
        assert_eq!(
            normalize_lexical(Path::new("a/b/c")),
            PathBuf::from("a/b/c")
        );
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
