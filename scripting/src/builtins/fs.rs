//! Filesystem, directory, and logging builtins for HUME's Steel scripting engine.
//!
//! **Write-path operations** (`make-dir`, `delete-dir`, `delete-file`) are
//! sandboxed:
//! - `make-dir`, `delete-dir` → the write sandbox (`<data>/plugins/` or
//!   `<data>/grammars/`); `delete-dir` on `<data>/grammars/sources/` is how
//!   `plum-update-grammar` purges a stale source tree.
//! - `delete-file` → `<data>/grammars/` only (narrower than the write sandbox).
//!
//! **Read-path operations** (`list-dir`, `path-exists?`) are additionally
//! allowed under `<runtime>/plugins/` and `<data>/grammars/`.
//!
//! Security invariant (see `feedback_security_canonicalize`): every
//! `canonicalize` call on an untrusted path must hard-fail via `steel::stop!`
//! on any `Err` — never fall back to the unresolved path.
//!
//! # Builtins registered here
//!
//! | Steel name      | Signature                      | Notes                                        |
//! |-----------------|--------------------------------|----------------------------------------------|
//! | `data-dir`      | `() → string \| #f`            | HUME data directory (XDG), or `#f` if unset  |
//! | `runtime-dir`   | `() → string \| #f`            | Runtime dir, or `#f` if absent               |
//! | `path-exists?`  | `string → bool`                | Sandboxed read                               |
//! | `list-dir`      | `string → list-of-string`      | Sandboxed read; returns names only           |
//! | `make-dir`      | `string → void`                | Sandboxed write (`<data>/plugins/`|`grammars/`)|
//! | `delete-dir`    | `string → void`                | Sandboxed write (`<data>/plugins/`|`grammars/`)|
//! | `delete-file`   | `string → void`                | Sandboxed write to `<data>/grammars/`        |
//! | `log!`          | `symbol string → void`         | Push to the pending message buffer           |

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use super::one_string;
use crate::log::LogLevel as Severity;
use crate::SteelCtx;

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
        platform::fs::create_dir_all(&p).ok()?;
        platform::fs::canonicalize(&p).ok()
    });

    // <data>/grammars/ and <data>/grammars/sources/
    let data_grammars = data_dir.as_ref().and_then(|d| {
        let g = d.join("grammars");
        platform::fs::create_dir_all(&g.join("sources")).ok()?;
        platform::fs::canonicalize(&g).ok()
    });

    // Canonicalize data_dir for the display form; fall back to raw path when
    // the directory doesn't exist (e.g. sandboxed FS test environments).
    let canonical_data =
        data_dir.map(|d| platform::fs::canonicalize(&d).unwrap_or(d));
    // Display form strips `\\?\` so Scheme can safely concatenate `/`-separated
    // segments on Windows without producing malformed extended-length paths.
    let data_dir_display = canonical_data.map(strip_unc_prefix);

    let canonical_runtime = runtime_dir.and_then(|rt| platform::fs::canonicalize(&rt).ok());
    let runtime_dir_display = canonical_runtime
        .as_ref()
        .map(|rt| strip_unc_prefix(rt.clone()));
    let runtime_plugins = canonical_runtime
        .as_ref()
        .and_then(|rt| platform::fs::canonicalize(&rt.join("plugins")).ok());
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

fn is_under_write_sandbox(canonical: &Path) -> bool {
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

fn is_under_read_sandbox(canonical: &Path) -> bool {
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
fn normalize_lexical(path: &Path) -> PathBuf {
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
        if platform::fs::exists(current) {
            break;
        }
        suffix.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
    let canonical_base = platform::fs::canonicalize(current).ok()?;
    let mut result = canonical_base;
    for component in suffix.into_iter().rev() {
        result.push(component);
    }
    Some(result)
}

// ── log! ──────────────────────────────────────────────────────────────────────

/// `(log! severity message)` — push `message` to the pending message buffer.
///
/// `severity` must be one of the symbols `'trace`, `'info`, `'warn`, or
/// `'error`.  Any other value raises a Steel error.
pub(crate) fn log_msg(
    ctx: &mut SteelCtx,
    severity: SteelVal,
    message: String,
) -> Result<SteelVal, SteelErr> {
    let sev_str = match &severity {
        SteelVal::SymbolV(s) => s.as_str().to_string(),
        _ => steel::stop!(TypeMismatch =>
            "log!: severity must be a symbol ('trace 'info 'warn 'error), got {:?}", severity),
    };
    let sev = match sev_str.as_str() {
        "trace" => Severity::Trace,
        "info" => Severity::Info,
        "warn" => Severity::Warning,
        "error" => Severity::Error,
        other => steel::stop!(Generic =>
            "log!: unknown severity '{}', expected 'trace, 'info, 'warn, or 'error", other),
    };
    ctx.pending_messages.push((sev, message));
    Ok(SteelVal::Void)
}

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// `(data-dir)` — returns the HUME data directory as a string, or `#f` if
/// HOME/APPDATA is unset.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows) so Scheme plugins can safely join segments with `(path-join …)`
/// or, if necessary, plain string concatenation.
pub(crate) fn data_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "data-dir expects 0 args, got {}", args.len());
    }
    with_dirs(|dirs| match &dirs.data_dir_display {
        Some(p) => p
            .to_string_lossy()
            .as_ref()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    })
}

/// `(runtime-dir)` — returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows).
pub(crate) fn runtime_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "runtime-dir expects 0 args, got {}", args.len());
    }
    with_dirs(|dirs| match &dirs.runtime_dir_display {
        Some(p) => p
            .to_string_lossy()
            .as_ref()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    })
}

// ── path-join ─────────────────────────────────────────────────────────────────

/// `(path-join seg1 seg2 …)` — join path segments using the OS-native
/// separator and return the result as a string.
///
/// Uses `PathBuf::push` semantics: if any segment is an absolute path it
/// replaces everything to the left (the same rule as `Path::join`).  This
/// lets plugins build paths portably without hard-coding `"/"` or `"\\"`.
///
/// No sandbox check — this is a pure string-construction helper that does not
/// access the filesystem.
pub(crate) fn path_join(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
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
        .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()))
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
    let (for_sandbox, exists) = match platform::fs::canonicalize(&path) {
        Ok(canonical) => (canonical, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let ancestor =
                canonical_ancestor_join(&path).unwrap_or_else(|| normalize_lexical(&path));
            (ancestor, false)
        }
        Err(e) => {
            return Err(SteelErr::new(
                ErrorKind::Generic,
                format!("path-exists?: cannot canonicalize '{}': {e}", raw),
            ));
        }
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
    let canonical = match platform::fs::canonicalize(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Vec::<SteelVal>::new()
                .into_steelval()
                .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()));
        }
        Err(e) => {
            return Err(SteelErr::new(
                ErrorKind::Generic,
                format!("list-dir: cannot canonicalize '{raw}': {e}"),
            ));
        }
    };

    if !is_under_read_sandbox(&canonical) {
        steel::stop!(Generic => "list-dir: path is outside the allowed sandbox: {}", raw);
    }

    if !canonical.is_dir() {
        return Vec::<SteelVal>::new()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()));
    }

    let mut names: Vec<String> = platform::fs::read_dir(&canonical)
        .map_err(|e| {
            SteelErr::new(
                ErrorKind::Generic,
                format!("list-dir: cannot read '{raw}': {e}"),
            )
        })?
        .filter_map(|entry| entry.ok().and_then(|e| e.file_name().into_string().ok()))
        .collect();

    names.sort();

    let vals: Vec<SteelVal> = names
        .into_iter()
        .map(|s| SteelVal::StringV(s.into()))
        .collect();

    vals.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::ConversionError, e.to_string()))
}

// ── make-dir ─────────────────────────────────────────────────────────────────

/// `(make-dir path)` — create `path` and any missing parent directories.
///
/// Sandboxed to the write sandbox (`<data>/plugins/` or `<data>/grammars/`).
/// Rejects any path containing `..`.
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
    let effective = canonical_ancestor_join(&path).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("make-dir: cannot resolve any ancestor of '{raw}'"),
        )
    })?;

    if !is_under_write_sandbox(&effective) {
        steel::stop!(Generic =>
            "make-dir: path is outside the write sandbox (<data>/plugins/ or <data>/grammars/): {}", raw);
    }

    platform::fs::create_dir_all(&path).map_err(|e| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("make-dir: cannot create '{}': {e}", raw),
        )
    })?;
    Ok(SteelVal::Void)
}

// ── delete-dir ───────────────────────────────────────────────────────────────

/// `(delete-dir path)` — recursively delete `path` and all its contents.
///
/// Sandboxed to the write sandbox (`<data>/plugins/` or `<data>/grammars/`).
/// `path` must exist; `canonicalize` failure is a hard error — never falls
/// back to the raw path.
///
/// Returns `#<void>` (including when `path` does not exist — idempotent).
pub(crate) fn delete_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "delete-dir")?;
    let path = PathBuf::from(&raw);

    // Canonicalize directly; treat NotFound as a no-op (idempotent).
    // Hard-fail on any other error — avoids TOCTOU between .exists() and canonicalize.
    let canonical = match platform::fs::canonicalize(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SteelVal::Void),
        Err(e) => {
            return Err(SteelErr::new(
                ErrorKind::Generic,
                format!("delete-dir: cannot resolve path '{raw}': {e}"),
            ));
        }
    };

    if !is_under_write_sandbox(&canonical) {
        steel::stop!(Generic =>
            "delete-dir: refusing to delete '{}' — outside the write sandbox (<data>/plugins/ or <data>/grammars/)",
            canonical.display());
    }

    platform::fs::remove_dir_all(&canonical).map_err(|e| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("delete-dir: cannot remove '{}': {e}", canonical.display()),
        )
    })?;
    Ok(SteelVal::Void)
}

// ── delete-file ──────────────────────────────────────────────────────────────

/// `(delete-file path)` — delete a single file.
///
/// Sandboxed to `<data>/grammars/`.  Idempotent: returns `#<void>` when the
/// path does not exist.  Rejects directories — use `delete-dir` for those.
pub(crate) fn delete_file(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "delete-file")?;
    let path = PathBuf::from(&raw);

    let canonical = match platform::fs::canonicalize(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SteelVal::Void),
        Err(e) => {
            return Err(SteelErr::new(
                ErrorKind::Generic,
                format!("delete-file: cannot resolve path '{raw}': {e}"),
            ));
        }
    };

    with_dirs(|dirs| {
        if !dirs.data_grammars.as_deref().is_some_and(|g| canonical.starts_with(g)) {
            Err(SteelErr::new(
                ErrorKind::Generic,
                format!(
                    "delete-file: refusing '{}' — outside the grammars sandbox (<data>/grammars/)",
                    canonical.display()
                ),
            ))
        } else {
            Ok(())
        }
    })?;

    if canonical.is_dir() {
        steel::stop!(Generic =>
            "delete-file: '{}' is a directory; use delete-dir instead",
            canonical.display());
    }

    match platform::fs::remove_file(&canonical) {
        Ok(()) => Ok(SteelVal::Void),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SteelVal::Void),
        Err(e) => Err(SteelErr::new(
            ErrorKind::Generic,
            format!("delete-file: cannot remove '{}': {e}", canonical.display()),
        )),
    }
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
        let plugins = data_dir.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        init_dirs(Some(data_dir), None);
        plugins
    }

    fn setup_grammars(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        init_dirs(Some(data_dir.clone()), None);
        data_dir.join("grammars")
    }

    // ── delete-dir ───────────────────────────────────────────────────────────

    #[test]
    fn delete_dir_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let plugin_dir = plugins.join("user/repo");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.scm"), b"; test").unwrap();

        let args = vec![SteelVal::StringV(
            plugin_dir.to_string_lossy().to_string().into(),
        )];
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
            err.to_string().contains("outside the write sandbox")
                || err.to_string().contains("cannot resolve"),
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

        let args = vec![SteelVal::StringV(
            plugins.to_string_lossy().to_string().into(),
        )];
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
        assert_eq!(
            path_exists(&[SteelVal::StringV(existing.into())]).unwrap(),
            SteelVal::BoolV(true)
        );

        let missing = plugins.join("nobody").to_string_lossy().to_string();
        assert_eq!(
            path_exists(&[SteelVal::StringV(missing.into())]).unwrap(),
            SteelVal::BoolV(false)
        );
    }

    // ── make-dir ─────────────────────────────────────────────────────────────

    #[test]
    fn make_dir_creates_nested() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let target = plugins.join("user/new-repo");
        let args = vec![SteelVal::StringV(
            target.to_string_lossy().to_string().into(),
        )];
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
        assert!(
            err.to_string().contains(".."),
            "expected .. error, got: {err}"
        );
    }

    // ── log! ─────────────────────────────────────────────────────────────────

    #[test]
    fn log_msg_valid_severity() {
        let mut h = crate::test_support::SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        log_msg(
            &mut ctx,
            SteelVal::SymbolV("info".into()),
            "hello".to_string(),
        )
        .unwrap();
        drop(ctx);
        assert_eq!(h.pending_messages.len(), 1);
        assert_eq!(h.pending_messages[0].1, "hello");
        assert!(matches!(h.pending_messages[0].0, Severity::Info));
    }

    #[test]
    fn log_msg_unknown_severity_errors() {
        let mut h = crate::test_support::SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(log_msg(&mut ctx, SteelVal::SymbolV("bad".into()), "msg".to_string()).is_err());
    }

    // ── path-join ────────────────────────────────────────────────────────────

    #[test]
    fn path_join_two_segments() {
        let args = vec![
            SteelVal::StringV("foo".into()),
            SteelVal::StringV("bar".into()),
        ];
        let result = path_join(&args).unwrap();
        let s = match result {
            SteelVal::StringV(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        };
        // The joined path must contain both components separated by the OS separator.
        let expected = std::path::PathBuf::from("foo").join("bar");
        assert_eq!(s, expected.to_string_lossy().as_ref());
    }

    #[test]
    fn path_join_single_segment() {
        let args = vec![SteelVal::StringV("only".into())];
        let result = path_join(&args).unwrap();
        assert!(matches!(result, SteelVal::StringV(s) if s.as_str() == "only"));
    }

    #[test]
    fn path_join_no_args_errors() {
        assert!(path_join(&[]).is_err());
    }

    #[test]
    fn path_join_type_error() {
        let args = vec![SteelVal::IntV(42)];
        assert!(path_join(&args).is_err());
    }

    // ── data-dir display (no UNC prefix) ─────────────────────────────────────

    /// On all platforms `(data-dir)` must return a string that does not begin
    /// with the Windows extended-length prefix `\\?\`.  On Unix this prefix
    /// never appears, so the test is platform-neutral.
    #[test]
    fn data_dir_no_unc_prefix() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let result = data_dir(&[]).unwrap();
        let s = match result {
            SteelVal::StringV(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        };
        assert!(
            !s.starts_with(r"\\?\"),
            "data-dir must not return an extended-length UNC path, got: {s}"
        );
    }

    // ── delete-file ──────────────────────────────────────────────────────────

    #[test]
    fn delete_file_removes_file() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let f = grammars.join("json.dylib");
        fs::write(&f, b"fake").unwrap();

        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        assert_eq!(delete_file(&args).unwrap(), SteelVal::Void);
        assert!(!f.exists());
    }

    #[test]
    fn delete_file_nonexistent_is_noop() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let missing = grammars.join("nobody.dylib").to_string_lossy().to_string();
        assert_eq!(
            delete_file(&[SteelVal::StringV(missing.into())]).unwrap(),
            SteelVal::Void
        );
    }

    #[test]
    fn delete_file_rejects_directory() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let dir = grammars.join("sources");
        let args = vec![SteelVal::StringV(dir.to_string_lossy().to_string().into())];
        let err = delete_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("directory"),
            "expected directory error, got: {err}"
        );
    }

    #[test]
    fn delete_file_rejects_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        // A file inside plugins/ is NOT in the grammars sandbox.
        let f = plugins.join("somefile");
        fs::write(&f, b"x").unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let err = delete_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("grammars sandbox"),
            "expected grammars sandbox error, got: {err}"
        );
    }

    // ── init_dirs fail-closed ─────────────────────────────────────────────────

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

    // ── Helper ───────────────────────────────────────────────────────────────

    fn steel_list_to_strings(val: SteelVal) -> Vec<String> {
        // Steel lists are `ListV` holding an immutable-list; convert through
        // `into_iter` which is implemented for the SteelVal list type.
        match val {
            SteelVal::ListV(list) => list
                .into_iter()
                .map(|v| match v {
                    SteelVal::StringV(s) => s.to_string(),
                    _ => panic!("expected string in list, got {v:?}"),
                })
                .collect(),
            _ => panic!("expected a list, got {val:?}"),
        }
    }
}
