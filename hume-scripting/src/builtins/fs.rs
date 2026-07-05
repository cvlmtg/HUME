//! Filesystem and directory builtins for HUME's Steel scripting engine.
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
//! on any `Err` — never fall back to the unresolved path.  The sandbox roots
//! in [`super::sandbox`] already enforce this during [`super::sandbox::init_dirs`].
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

use std::path::PathBuf;

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use super::conv_err;
use super::one_string;
use super::sandbox::{
    canonical_ancestor_join, has_dotdot, is_under_grammars_sandbox, is_under_read_sandbox,
    is_under_write_sandbox, normalize_lexical,
};

// ── data-dir / runtime-dir ───────────────────────────────────────────────────

/// Shared body for `(data-dir)`/`(runtime-dir)`: both take no args and return
/// `dir()`'s display-form path as a string, or `#f` if `dir()` is `None`.
fn dir_builtin(
    args: &[SteelVal],
    name: &'static str,
    dir: impl FnOnce() -> Option<PathBuf>,
) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "{name} expects 0 args, got {}", args.len());
    }
    match dir() {
        Some(p) => p
            .to_string_lossy()
            .as_ref()
            .into_steelval()
            .map_err(conv_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(data-dir)` — returns the HUME data directory as a string, or `#f` if
/// HOME/APPDATA is unset.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows) so Scheme plugins can safely join segments with `(path-join …)`
/// or, if necessary, plain string concatenation.
pub(crate) fn data_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    dir_builtin(args, "data-dir", super::sandbox::data_dir_display)
}

/// `(runtime-dir)` — returns the HUME runtime directory as a string, or `#f`
/// if no runtime directory was found.
///
/// The returned path is the display form (no `\\?\` extended-length prefix on
/// Windows).
pub(crate) fn runtime_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    dir_builtin(args, "runtime-dir", super::sandbox::runtime_dir_display)
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
        .map_err(conv_err)
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
    let (for_sandbox, exists) = match hume_platform::fs::canonicalize(&path) {
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

// ── canonicalize helper (list-dir / delete-dir / delete-file) ────────────────

/// Canonicalize `raw` for a sandbox check, treating `NotFound` as `Ok(None)`
/// rather than an error. Shared by `list-dir`, `delete-dir`, and
/// `delete-file` — each maps `None` to its own idempotent no-op result. Any
/// other canonicalize failure hard-fails (see module doc: never fall back to
/// the unresolved path). `path-exists?` doesn't use this — its `NotFound` arm
/// needs the ancestor-join fallback to still run the sandbox check.
fn canonicalize_or_notfound(op: &str, raw: &str) -> Result<Option<PathBuf>, SteelErr> {
    match hume_platform::fs::canonicalize(&PathBuf::from(raw)) {
        Ok(c) => Ok(Some(c)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SteelErr::new(
            ErrorKind::Generic,
            format!("{op}: cannot resolve path '{raw}': {e}"),
        )),
    }
}

// ── list-dir ─────────────────────────────────────────────────────────────────

/// `(list-dir path)` — return a sorted list of entry *names* (not full paths)
/// in directory `path`.
///
/// Returns an empty list if `path` does not exist or is not a directory.
/// Sandboxed to `<data>/plugins/` and `<runtime>/plugins/`.
pub(crate) fn list_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "list-dir")?;
    let Some(canonical) = canonicalize_or_notfound("list-dir", &raw)? else {
        return Vec::<SteelVal>::new().into_steelval().map_err(conv_err);
    };

    if !is_under_read_sandbox(&canonical) {
        steel::stop!(Generic => "list-dir: path is outside the allowed sandbox: {}", raw);
    }

    if !canonical.is_dir() {
        return Vec::<SteelVal>::new().into_steelval().map_err(conv_err);
    }

    let mut names: Vec<String> = hume_platform::fs::read_dir(&canonical)
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

    vals.into_steelval().map_err(conv_err)
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

    // Create through the sandbox-checked resolved path, not the raw input —
    // the raw path could be re-pointed (symlinked) between check and create.
    hume_platform::fs::create_dir_all(&effective).map_err(|e| {
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
    let Some(canonical) = canonicalize_or_notfound("delete-dir", &raw)? else {
        return Ok(SteelVal::Void); // idempotent — nothing to delete
    };

    if !is_under_write_sandbox(&canonical) {
        steel::stop!(Generic =>
            "delete-dir: refusing to delete '{}' — outside the write sandbox (<data>/plugins/ or <data>/grammars/)",
            canonical.display());
    }

    hume_platform::fs::remove_dir_all(&canonical).map_err(|e| {
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
    let Some(canonical) = canonicalize_or_notfound("delete-file", &raw)? else {
        return Ok(SteelVal::Void); // idempotent — nothing to delete
    };

    if !is_under_grammars_sandbox(&canonical) {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!(
                "delete-file: refusing '{}' — outside the grammars sandbox (<data>/grammars/)",
                canonical.display()
            ),
        ));
    }

    if canonical.is_dir() {
        steel::stop!(Generic =>
            "delete-file: '{}' is a directory; use delete-dir instead",
            canonical.display());
    }

    match hume_platform::fs::remove_file(&canonical) {
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

    use super::super::sandbox::init_dirs;

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
        fs::write(&f, f.to_string_lossy().as_bytes()).unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let err = delete_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("grammars sandbox"),
            "expected grammars sandbox error, got: {err}"
        );
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
