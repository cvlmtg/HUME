//! Filesystem, directory, and logging builtins for HUME's Steel scripting engine.
//!
//! **Write-path operations** (`make-dir`, `delete-dir`) are hard-sandboxed to
//! `<data>/plugins/`.  **Read-path operations** (`list-dir`, `path-exists?`)
//! are additionally allowed under `<runtime>/plugins/`.
//!
//! Security invariant (see `feedback_security_canonicalize`): every
//! `canonicalize` call on an untrusted path must hard-fail via `steel::stop!`
//! on any `Err` — never fall back to the unresolved path.
//!
//! # Builtins registered here
//!
//! | Steel name      | Signature                      | Notes                              |
//! |-----------------|--------------------------------|------------------------------------|
//! | `data-dir`      | `() → string \| #f`            | HUME data directory (XDG), or `#f` if HOME/APPDATA unset |
//! | `runtime-dir`   | `() → string \| #f`            | Runtime dir, or `#f` if absent     |
//! | `path-exists?`  | `string → bool`                | Sandboxed read                     |
//! | `list-dir`      | `string → list-of-string`      | Sandboxed read; returns names only |
//! | `make-dir`      | `string → void`                | Sandboxed write to `<data>/plugins`|
//! | `delete-dir`    | `string → void`                | Sandboxed write to `<data>/plugins`|
//! | `log!`          | `symbol string → void`         | Push to the pending message buffer |

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use steel::rvals::{IntoSteelVal, SteelVal};
use steel::rerrs::{ErrorKind, SteelErr};

use crate::editor::Severity;
use super::one_string;

// ── Permanent dirs TLS ────────────────────────────────────────────────────────

struct ScriptDirs {
    /// `<data>/hume/` — or `None` if HOME/APPDATA is unset.
    data_dir:        Option<PathBuf>,
    runtime_dir:     Option<PathBuf>,
    /// Canonical `<data>/plugins/` — the write-path sandbox root.
    /// `None` when `data_dir` is `None`; every write sandbox check then fails
    /// closed.
    data_plugins:    Option<PathBuf>,
    /// Canonical `<runtime>/plugins/` — allowed for read-path ops only.
    runtime_plugins: Option<PathBuf>,
}

thread_local! {
    static SCRIPT_DIRS: RefCell<Option<ScriptDirs>> = RefCell::new(None);
}

/// Initialize the directory TLS.  Must be called exactly once during
/// [`crate::scripting::ScriptingHost::new`] before any builtins are invoked.
pub(crate) fn init_dirs(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) {
    // Canonicalize eagerly so all subsequent starts_with comparisons are
    // reliable (e.g. macOS /tmp → /private/tmp). Falls back to the raw path
    // when the directory does not exist yet (first run). When data_dir is
    // None (HOME/APPDATA unset), data_plugins is also None and every write
    // sandbox check fails closed.
    let canonical_data = data_dir.map(|d| d.canonicalize().unwrap_or_else(|_| d));
    let data_plugins = canonical_data.as_ref().map(|d| {
        let p = d.join("plugins");
        p.canonicalize().unwrap_or_else(|_| p)
    });
    let canonical_runtime = runtime_dir.and_then(|rt| rt.canonicalize().ok());
    let runtime_plugins = canonical_runtime.as_ref().and_then(|rt| {
        rt.join("plugins").canonicalize().ok()
    });
    // Store only the canonical form; if the runtime dir doesn't exist (or
    // canonicalize fails for any reason), leave it as None rather than storing
    // an unresolved path that would make the field inconsistently typed.
    SCRIPT_DIRS.with(|cell| {
        *cell.borrow_mut() = Some(ScriptDirs {
            data_dir:        canonical_data,
            runtime_dir:     canonical_runtime,
            data_plugins,
            runtime_plugins,
        });
    });
}

fn with_dirs<R>(f: impl FnOnce(&ScriptDirs) -> R) -> R {
    SCRIPT_DIRS.with(|cell| {
        let borrow = cell.borrow();
        f(borrow.as_ref()
            .expect("SCRIPT_DIRS not initialized — ScriptingHost::new() must call fs::init_dirs"))
    })
}

/// Call `f` with the canonical write-sandbox root (`<data>/plugins/`).
/// Used by `shell.rs` to sandbox git operations.
///
/// Returns `Err` when no data directory is available (HOME/APPDATA unset),
/// which fails the write sandbox check closed rather than silently permitting it.
pub(crate) fn with_data_plugins<R>(f: impl FnOnce(&Path) -> R) -> Result<R, SteelErr> {
    with_dirs(|dirs| match dirs.data_plugins.as_deref() {
        Some(p) => Ok(f(p)),
        None => Err(SteelErr::new(ErrorKind::Generic,
            "no data directory — HOME/APPDATA unset; write operations unavailable".to_string())),
    })
}

// ── Sandbox predicates ────────────────────────────────────────────────────────

fn is_under_write_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| dirs.data_plugins.as_deref().map_or(false, |p| canonical.starts_with(p)))
}

fn is_under_read_sandbox(canonical: &Path) -> bool {
    with_dirs(|dirs| {
        dirs.data_plugins.as_deref().map_or(false, |p| canonical.starts_with(p))
            || dirs.runtime_plugins.as_deref().map_or(false, |rp| canonical.starts_with(rp))
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
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir    => {}
            Component::ParentDir => { out.pop(); }
            other                => out.push(other),
        }
    }
    out
}

/// Canonicalize the deepest existing ancestor of `path`, then rejoin any
/// non-existing suffix components.
///
/// Used by [`make_dir`] to sandbox-check a path that does not yet exist.
/// The `..` rejection in `make_dir` (via [`has_dotdot`]) prevents traversal
/// attacks that this function cannot catch.
///
/// Returns `None` if no ancestor exists at all (e.g. a completely bogus path).
fn canonical_ancestor_join(path: &Path) -> Option<PathBuf> {
    let mut suffix = vec![];
    let mut current = path;
    // Walk up until we find a component that exists on disk.
    loop {
        if current.exists() {
            break;
        }
        suffix.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
    let canonical_base = current.canonicalize().ok()?;
    let mut result = canonical_base;
    for component in suffix.into_iter().rev() {
        result.push(component);
    }
    Some(result)
}

// ── LOG_QUEUE ─────────────────────────────────────────────────────────────────

// Pending messages accumulated by `log!` during a Steel eval or command
// invocation.  Initialized to `Some(Vec::new())` before each eval / command;
// taken afterward and drained into `Editor::report` by the call-site.
thread_local! {
    pub(crate) static LOG_QUEUE: RefCell<Option<Vec<(Severity, String)>>>
        = RefCell::new(None);
}

// ── log! ──────────────────────────────────────────────────────────────────────

/// `(log! severity message)` — push `message` to the pending message buffer.
///
/// `severity` must be one of the symbols `'trace`, `'info`, `'warn`, or
/// `'error`.  Any other value raises a Steel error.
pub(crate) fn log_msg(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if args.len() != 2 {
        steel::stop!(ArityMismatch => "log! expects 2 args (severity message), got {}", args.len());
    }
    let sev_str = match &args[0] {
        SteelVal::SymbolV(s) => s.as_str().to_string(),
        _ => steel::stop!(TypeMismatch =>
            "log!: severity must be a symbol ('trace 'info 'warn 'error), got {:?}", args[0]),
    };
    let severity = match sev_str.as_str() {
        "trace" => Severity::Trace,
        "info"  => Severity::Info,
        "warn"  => Severity::Warning,
        "error" => Severity::Error,
        other   => steel::stop!(Generic =>
            "log!: unknown severity '{}', expected 'trace, 'info, 'warn, or 'error", other),
    };
    let text = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "log!: message must be a string, got {:?}", args[1]),
    };
    LOG_QUEUE.with(|cell| {
        if let Some(queue) = cell.borrow_mut().as_mut() {
            queue.push((severity, text));
        }
        // If LOG_QUEUE is None (e.g. bootstrap eval), drop silently.
    });
    Ok(SteelVal::Void)
}

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// `(data-dir)` — returns the HUME data directory as a string, or `#f` if
/// HOME/APPDATA is unset.
pub(crate) fn data_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "data-dir expects 0 args, got {}", args.len());
    }
    with_dirs(|dirs| match &dirs.data_dir {
        Some(p) => p.to_string_lossy().as_ref()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    })
}

/// `(runtime-dir)` — returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
pub(crate) fn runtime_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "runtime-dir expects 0 args, got {}", args.len());
    }
    with_dirs(|dirs| match &dirs.runtime_dir {
        Some(p) => p.to_string_lossy().as_ref()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    })
}

// ── path-exists? ─────────────────────────────────────────────────────────────

/// `(path-exists? path)` — return `#t` if `path` exists on disk, `#f` otherwise.
///
/// Sandboxed to `<data>/plugins/` and `<runtime>/plugins/`.
pub(crate) fn path_exists(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "path-exists?")?;
    let path = PathBuf::from(&raw);

    // Resolve symlinks fully when the path exists; when it doesn't, canonicalize
    // the deepest existing ancestor and rejoin the suffix.  Either way the
    // sandbox check uses a real canonical prefix (handles macOS /var → /private/var).
    // Avoid a pre-flight `.exists()` check — handle NotFound from canonicalize
    // directly so there is no TOCTOU window between the check and the syscall.
    let (for_sandbox, exists) = match path.canonicalize() {
        Ok(canonical) => (canonical, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let ancestor = canonical_ancestor_join(&path)
                .unwrap_or_else(|| normalize_lexical(&path));
            (ancestor, false)
        }
        Err(e) => return Err(SteelErr::new(ErrorKind::Generic,
            format!("path-exists?: cannot canonicalize '{}': {e}", raw))),
    };

    if !is_under_read_sandbox(&for_sandbox) {
        steel::stop!(Generic => "path-exists?: path is outside the allowed sandbox: {}", raw);
    }
    Ok(SteelVal::BoolV(exists))
}

// ── list-dir ─────────────────────────────────────────────────────────────────

/// `(list-dir path)` — return a sorted list of entry *names* (not full paths)
/// in directory `path`.
///
/// Returns an empty list if `path` does not exist or is not a directory.
/// Sandboxed to `<data>/plugins/` and `<runtime>/plugins/`.
pub(crate) fn list_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "list-dir")?;
    let path = PathBuf::from(&raw);

    // Canonicalize directly; treat NotFound as an empty-list result.
    // Avoids the TOCTOU window between a pre-flight .exists() and canonicalize.
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Vec::<SteelVal>::new().into_steelval()
                .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()));
        }
        Err(e) => return Err(SteelErr::new(ErrorKind::Generic,
            format!("list-dir: cannot canonicalize '{raw}': {e}"))),
    };

    if !is_under_read_sandbox(&canonical) {
        steel::stop!(Generic => "list-dir: path is outside the allowed sandbox: {}", raw);
    }

    if !canonical.is_dir() {
        return Vec::<SteelVal>::new().into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()));
    }

    let mut names: Vec<String> = std::fs::read_dir(&canonical)
        .map_err(|e| SteelErr::new(ErrorKind::Generic,
            format!("list-dir: cannot read '{raw}': {e}")))?
        .filter_map(|entry| entry.ok().and_then(|e| e.file_name().into_string().ok()))
        .collect();

    names.sort();

    let vals: Vec<SteelVal> = names.into_iter()
        .map(|s| SteelVal::StringV(s.into()))
        .collect();

    vals.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()))
}

// ── make-dir ─────────────────────────────────────────────────────────────────

/// `(make-dir path)` — create `path` and any missing parent directories.
///
/// Sandboxed to `<data>/plugins/`.  Rejects any path containing `..`.
pub(crate) fn make_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "make-dir")?;
    let path = PathBuf::from(&raw);

    if has_dotdot(&path) {
        steel::stop!(Generic => "make-dir: path must not contain '..' components: {}", raw);
    }

    // The directory may not exist yet so we cannot `canonicalize` the full
    // path.  Instead we canonicalize the deepest existing ancestor and rejoin
    // the non-existing suffix.  `has_dotdot` above rules out traversal attacks
    // that lexical resolution cannot catch.
    let effective = canonical_ancestor_join(&path)
        .ok_or_else(|| SteelErr::new(ErrorKind::Generic,
            format!("make-dir: cannot resolve any ancestor of '{raw}'")))?;

    if !is_under_write_sandbox(&effective) {
        steel::stop!(Generic =>
            "make-dir: path is outside the write sandbox (<data>/plugins/): {}", raw);
    }

    std::fs::create_dir_all(&path)
        .map_err(|e| SteelErr::new(ErrorKind::Generic,
            format!("make-dir: cannot create '{}': {e}", raw)))?;
    Ok(SteelVal::Void)
}

// ── delete-dir ───────────────────────────────────────────────────────────────

/// `(delete-dir path)` — recursively delete `path` and all its contents.
///
/// Sandboxed to `<data>/plugins/`.  `path` must exist; `canonicalize` failure
/// is a hard error — never falls back to the raw path.
///
/// Returns `#<void>` (including when `path` does not exist — idempotent).
pub(crate) fn delete_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "delete-dir")?;
    let path = PathBuf::from(&raw);

    // Canonicalize directly; treat NotFound as a no-op (idempotent).
    // Hard-fail on any other error — avoids TOCTOU between .exists() and canonicalize.
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SteelVal::Void),
        Err(e) => return Err(SteelErr::new(ErrorKind::Generic,
            format!("delete-dir: cannot resolve path '{raw}': {e}"))),
    };

    if !is_under_write_sandbox(&canonical) {
        steel::stop!(Generic =>
            "delete-dir: refusing to delete '{}' — outside the write sandbox (<data>/plugins/)",
            canonical.display());
    }

    std::fs::remove_dir_all(&canonical)
        .map_err(|e| SteelErr::new(ErrorKind::Generic,
            format!("delete-dir: cannot remove '{}': {e}", canonical.display())))?;
    Ok(SteelVal::Void)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Each test gets its own TempDir. Because SCRIPT_DIRS is thread-local and
    // tests run in separate threads, concurrent tests don't interfere.
    fn setup(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
        let plugins  = data_dir.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        init_dirs(Some(data_dir), None);
        plugins
    }

    // ── delete-dir ───────────────────────────────────────────────────────────

    #[test]
    fn delete_dir_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let plugin_dir = plugins.join("user/repo");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.scm"), b"; test").unwrap();

        let args = vec![SteelVal::StringV(plugin_dir.to_string_lossy().to_string().into())];
        assert!(delete_dir(&args).is_ok());
        assert!(!plugin_dir.exists());
    }

    #[test]
    fn delete_dir_nonexistent_is_noop() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let missing = plugins.join("nobody/norepo").to_string_lossy().to_string();
        let args = vec![SteelVal::StringV(missing.into())];
        assert_eq!(delete_dir(&args).unwrap(), SteelVal::Void);
    }

    #[test]
    fn delete_dir_rejects_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        // Try to delete the temp root — outside <data>/plugins/.
        let outside = tmp.path().to_string_lossy().to_string();
        let args = vec![SteelVal::StringV(outside.into())];
        let err = delete_dir(&args).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("outside the write sandbox") || msg.contains("cannot resolve"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn delete_dir_rejects_dotdot_escape() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        // Construct an existing path with .. escape.
        // Create the directory so canonicalize succeeds, then check sandbox.
        fs::create_dir_all(plugins.join("user/repo")).unwrap();
        let escape = format!("{}/user/repo/../../..", plugins.display());
        let args = vec![SteelVal::StringV(escape.into())];
        let err = delete_dir(&args).unwrap_err();
        assert!(
            err.to_string().contains("outside the write sandbox") || err.to_string().contains("cannot resolve"),
            "expected sandbox error, got: {err}"
        );
    }

    // ── list-dir ─────────────────────────────────────────────────────────────

    #[test]
    fn list_dir_returns_sorted_names() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        fs::create_dir_all(plugins.join("beta")).unwrap();
        fs::create_dir_all(plugins.join("alpha")).unwrap();
        fs::write(plugins.join("file.txt"), b"").unwrap();

        let args = vec![SteelVal::StringV(plugins.to_string_lossy().to_string().into())];
        let result = list_dir(&args).unwrap();

        // Extract string values from the list.
        let names = steel_list_to_strings(result);
        assert_eq!(names, vec!["alpha", "beta", "file.txt"]);
    }

    #[test]
    fn list_dir_nonexistent_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let missing = plugins.join("nobody").to_string_lossy().to_string();
        let args = vec![SteelVal::StringV(missing.into())];
        let result = list_dir(&args).unwrap();
        assert_eq!(steel_list_to_strings(result), Vec::<String>::new());
    }

    #[test]
    fn list_dir_rejects_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        let outside = tmp.path().to_string_lossy().to_string();
        let args = vec![SteelVal::StringV(outside.into())];
        assert!(list_dir(&args).is_err());
    }

    // ── path-exists? ─────────────────────────────────────────────────────────

    #[test]
    fn path_exists_existing_and_missing() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);

        let existing = plugins.to_string_lossy().to_string();
        assert_eq!(path_exists(&[SteelVal::StringV(existing.into())]).unwrap(),
                   SteelVal::BoolV(true));

        let missing = plugins.join("nobody").to_string_lossy().to_string();
        assert_eq!(path_exists(&[SteelVal::StringV(missing.into())]).unwrap(),
                   SteelVal::BoolV(false));
    }

    // ── make-dir ─────────────────────────────────────────────────────────────

    #[test]
    fn make_dir_creates_nested() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let target = plugins.join("user/new-repo");
        let args = vec![SteelVal::StringV(target.to_string_lossy().to_string().into())];
        assert!(make_dir(&args).is_ok());
        assert!(target.is_dir());
    }

    #[test]
    fn make_dir_rejects_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        let bad = tmp.path().join("evil").to_string_lossy().to_string();
        assert!(make_dir(&[SteelVal::StringV(bad.into())]).is_err());
    }

    #[test]
    fn make_dir_rejects_dotdot() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let bad = format!("{}/user/../../../evil", plugins.display());
        let err = make_dir(&[SteelVal::StringV(bad.into())]).unwrap_err();
        assert!(err.to_string().contains(".."), "expected .. error, got: {err}");
    }

    // ── log! ─────────────────────────────────────────────────────────────────

    #[test]
    fn log_msg_valid_severity() {
        LOG_QUEUE.with(|q| *q.borrow_mut() = Some(Vec::new()));

        let args = vec![
            SteelVal::SymbolV("info".into()),
            SteelVal::StringV("hello".into()),
        ];
        assert_eq!(log_msg(&args).unwrap(), SteelVal::Void);

        let msgs = LOG_QUEUE.with(|q| q.borrow_mut().take().unwrap());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].1, "hello");
        assert!(matches!(msgs[0].0, Severity::Info));
    }

    #[test]
    fn log_msg_unknown_severity_errors() {
        LOG_QUEUE.with(|q| *q.borrow_mut() = Some(Vec::new()));
        let args = vec![
            SteelVal::SymbolV("bad".into()),
            SteelVal::StringV("msg".into()),
        ];
        assert!(log_msg(&args).is_err());
    }

    // ── Helper ───────────────────────────────────────────────────────────────

    fn steel_list_to_strings(val: SteelVal) -> Vec<String> {
        // Steel lists are `ListV` holding an immutable-list; convert through
        // `into_iter` which is implemented for the SteelVal list type.
        match val {
            SteelVal::ListV(list) => list.into_iter()
                .map(|v| match v {
                    SteelVal::StringV(s) => s.to_string(),
                    _ => panic!("expected string in list, got {v:?}"),
                })
                .collect(),
            _ => panic!("expected a list, got {val:?}"),
        }
    }
}
