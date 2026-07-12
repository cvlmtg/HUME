//! Filesystem and directory builtins for HUME's Steel scripting engine.
//!
//! **Write-path operations** (`make-dir`, `delete-dir`, `delete-file`) are
//! sandboxed:
//! - `make-dir`, `delete-dir` → the write sandbox (`<data>/plugins/`,
//!   `<data>/grammars/`, or `<data>/servers/`); `delete-dir` on
//!   `<data>/grammars/sources/` is how `plum-install-grammar` purges a stale
//!   source tree before re-cloning, and on `<data>/servers/<name>/` is how
//!   `:lsp-uninstall` removes an installed server.
//! - `delete-file` → `<data>/grammars/` or `<data>/servers/` (the narrower
//!   "install" sandbox — archive cleanup after unpacking, not the full write
//!   sandbox).
//!
//! **Read-path operations** (`list-dir`, `path-exists?`, `read-file`) are
//! additionally allowed under `<runtime>/plugins/`, `<data>/grammars/`, and
//! `<data>/servers/` (receipt scanning).
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
//! | `read-file`     | `string → string`              | Sandboxed read (`<runtime>/plugins/`|`<data>/grammars/`|`<data>/servers/`) |
//! | `make-dir`      | `string → void`                | Sandboxed write (`<data>/plugins/`|`grammars/`|`servers/`)|
//! | `delete-dir`    | `string → void`                | Sandboxed write (`<data>/plugins/`|`grammars/`|`servers/`)|
//! | `delete-file`   | `string → void`                | Sandboxed write to `<data>/grammars/` or `<data>/servers/` |
//! | `write-file`    | `string, string → void`        | Sandboxed write to `<data>/grammars/` or `<data>/servers/` |

use std::path::PathBuf;

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use super::conv_err;
use super::one_string;
use super::sandbox::{
    canonical_ancestor_join, has_dotdot, is_under_install_sandbox, is_under_read_sandbox,
    is_under_write_sandbox, normalize_lexical,
};
use super::shell::{SandboxKind, validate_new_path};

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

// ── read-file ─────────────────────────────────────────────────────────────────

/// `(read-file path)` — read a file's full contents as a UTF-8 string.
///
/// Sandboxed to `<data>/grammars/`, `<data>/servers/`, and
/// `<runtime>/plugins/` (the read sandbox), same as `list-dir`/`path-exists?`.
pub(crate) fn read_file(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "read-file")?;
    let Some(canonical) = canonicalize_or_notfound("read-file", &raw)? else {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!("read-file: no such file: {raw}"),
        ));
    };

    if !is_under_read_sandbox(&canonical) {
        steel::stop!(Generic => "read-file: path is outside the allowed sandbox: {}", raw);
    }

    let content = hume_platform::fs::read_to_string(&canonical).map_err(|e| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("read-file: cannot read '{raw}': {e}"),
        )
    })?;
    content.into_steelval().map_err(conv_err)
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
/// Sandboxed to the write sandbox (`<data>/plugins/`, `<data>/grammars/`, or
/// `<data>/servers/`). Rejects any path containing `..`.
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
            "make-dir: path is outside the write sandbox (<data>/plugins/, <data>/grammars/, or <data>/servers/): {}", raw);
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
/// Sandboxed to the write sandbox (`<data>/plugins/`, `<data>/grammars/`, or
/// `<data>/servers/`). `path` must exist; `canonicalize` failure is a hard
/// error — never falls back to the raw path. Rejects any raw path containing
/// `..` components before canonicalizing — the write sandbox spans three
/// sibling roots, so a `..` segment can resolve from one root into another
/// without ever leaving the sandbox (e.g. `servers/../plugins`).
///
/// Returns `#<void>` (including when `path` does not exist — idempotent).
pub(crate) fn delete_dir(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "delete-dir")?;
    if has_dotdot(std::path::Path::new(&raw)) {
        steel::stop!(Generic => "delete-dir: path must not contain '..' components: {}", raw);
    }
    let Some(canonical) = canonicalize_or_notfound("delete-dir", &raw)? else {
        return Ok(SteelVal::Void); // idempotent — nothing to delete
    };

    if !is_under_write_sandbox(&canonical) {
        steel::stop!(Generic =>
            "delete-dir: refusing to delete '{}' — outside the write sandbox (<data>/plugins/, <data>/grammars/, or <data>/servers/)",
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
/// Sandboxed to `<data>/grammars/` or `<data>/servers/` (the install
/// sandbox).  Idempotent: returns `#<void>` when the path does not exist.
/// Rejects directories — use `delete-dir` for those. Rejects any raw path
/// containing `..` components before canonicalizing, same rationale as
/// `delete-dir`.
pub(crate) fn delete_file(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let raw = one_string(args, "delete-file")?;
    if has_dotdot(std::path::Path::new(&raw)) {
        steel::stop!(Generic => "delete-file: path must not contain '..' components: {}", raw);
    }
    let Some(canonical) = canonicalize_or_notfound("delete-file", &raw)? else {
        return Ok(SteelVal::Void); // idempotent — nothing to delete
    };

    if !is_under_install_sandbox(&canonical) {
        return Err(SteelErr::new(
            ErrorKind::Generic,
            format!(
                "delete-file: refusing '{}' — outside the grammars/servers sandbox (<data>/grammars/ or <data>/servers/)",
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

// ── write-file ───────────────────────────────────────────────────────────────

/// `(write-file path content)` — write `content` to `path`, creating (or
/// overwriting) it and any missing parent directories.
///
/// Sandboxed to `<data>/grammars/` or `<data>/servers/` (the install
/// sandbox), same scope as `delete-file` — PLUM uses this to persist Helix
/// query files it has resolved from an `; inherits:` chain into a single
/// file on disk, and the LSP installer uses it to write install receipts.
pub(crate) fn write_file(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if args.len() != 2 {
        steel::stop!(ArityMismatch => "write-file expects 2 args (path, content), got {}", args.len());
    }
    let raw = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        other => steel::stop!(TypeMismatch => "write-file: path must be a string, got {:?}", other),
    };
    let content = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        other => {
            steel::stop!(TypeMismatch => "write-file: content must be a string, got {:?}", other)
        }
    };

    let dest_path = validate_new_path(&PathBuf::from(&raw), "write-file", SandboxKind::Install)?;

    // Create parent after the sandbox check so we don't mkdir outside the
    // sandbox — mirrors curl-fetch.
    if let Some(parent) = dest_path.parent() {
        hume_platform::fs::create_dir_all(parent).map_err(|e| {
            SteelErr::new(
                ErrorKind::Generic,
                format!("write-file: cannot create parent directory for '{raw}': {e}"),
            )
        })?;
    }

    hume_platform::fs::write(&dest_path, content.as_bytes()).map_err(|e| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("write-file: cannot write '{raw}': {e}"),
        )
    })?;
    Ok(SteelVal::Void)
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

    // `init_dirs` itself creates `<data>/servers/` (see sandbox.rs), so no
    // manual `create_dir_all` is needed here.
    fn setup_servers(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        init_dirs(Some(data_dir.clone()), None);
        data_dir.join("servers")
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
        fs::create_dir_all(plugins.join("user/repo")).unwrap();
        let escape = format!("{}/user/repo/../../..", plugins.display());
        let args = vec![SteelVal::StringV(escape.into())];
        let err = delete_dir(&args).unwrap_err();
        assert!(
            err.to_string().contains("must not contain '..' components"),
            "expected the dotdot rejection (checked before canonicalize), got: {err}"
        );
    }

    #[test]
    fn delete_dir_rejects_dotdot_even_within_sandbox() {
        // The write sandbox spans three sibling roots (plugins/grammars/servers),
        // so a `..` segment from one can resolve into another without ever
        // leaving the sandbox — `is_under_write_sandbox` alone would allow it.
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let data_dir = servers.parent().unwrap();
        fs::create_dir_all(data_dir.join("plugins/some-plugin")).unwrap();
        let escape = format!("{}/../plugins/some-plugin", servers.display());
        let args = vec![SteelVal::StringV(escape.into())];
        let err = delete_dir(&args).unwrap_err();
        assert!(
            err.to_string().contains("must not contain '..' components"),
            "expected the dotdot rejection, got: {err}"
        );
        assert!(
            data_dir.join("plugins/some-plugin").exists(),
            "the sibling sandbox root must survive untouched"
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
    fn delete_file_rejects_dotdot_escape() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        fs::write(grammars.join("json.dylib"), b"fake").unwrap();
        let escape = format!("{}/../grammars/json.dylib", grammars.display());
        let args = vec![SteelVal::StringV(escape.into())];
        let err = delete_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("must not contain '..' components"),
            "expected the dotdot rejection (checked before canonicalize), got: {err}"
        );
    }

    #[test]
    fn delete_file_rejects_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        // A file inside plugins/ is NOT in the grammars/servers install sandbox.
        let f = plugins.join("somefile");
        fs::write(&f, f.to_string_lossy().as_bytes()).unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let err = delete_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("grammars/servers sandbox"),
            "expected grammars/servers sandbox error, got: {err}"
        );
    }

    // ── read-file ────────────────────────────────────────────────────────────

    #[test]
    fn read_file_returns_contents() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let f = grammars.join("sources/highlights.scm");
        fs::write(&f, "(identifier) @variable\n").unwrap();

        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let result = read_file(&args).unwrap();
        assert!(matches!(result, SteelVal::StringV(s) if s.as_str() == "(identifier) @variable\n"));
    }

    #[test]
    fn read_file_missing_errors() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let missing = grammars
            .join("sources/nope.scm")
            .to_string_lossy()
            .to_string();
        assert!(read_file(&[SteelVal::StringV(missing.into())]).is_err());
    }

    #[test]
    fn read_file_rejects_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        // tmp root itself is outside both the plugins and grammars sandboxes.
        let f = tmp.path().join("somefile");
        fs::write(&f, b"secret").unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let err = read_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    // ── write-file ───────────────────────────────────────────────────────────

    #[test]
    fn write_file_creates_and_overwrites() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        // Mirrors the real pipeline: git-clone-rev creates `sources/<name>/`
        // before any query file gets written into it.
        fs::create_dir_all(grammars.join("sources/tsx")).unwrap();
        let f = grammars.join("sources/tsx/highlights.scm");

        let args = vec![
            SteelVal::StringV(f.to_string_lossy().to_string().into()),
            SteelVal::StringV("(identifier) @variable\n".into()),
        ];
        assert_eq!(write_file(&args).unwrap(), SteelVal::Void);
        assert_eq!(fs::read_to_string(&f).unwrap(), "(identifier) @variable\n");

        // A shorter overwrite must not leave stale trailing bytes behind.
        let args2 = vec![
            SteelVal::StringV(f.to_string_lossy().to_string().into()),
            SteelVal::StringV("x".into()),
        ];
        assert_eq!(write_file(&args2).unwrap(), SteelVal::Void);
        assert_eq!(fs::read_to_string(&f).unwrap(), "x");
    }

    #[test]
    fn write_file_rejects_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        let plugins = setup(&tmp);
        let f = plugins.join("somefile").to_string_lossy().to_string();
        let args = vec![SteelVal::StringV(f.into()), SteelVal::StringV("x".into())];
        let err = write_file(&args).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn write_file_rejects_dotdot() {
        let tmp = TempDir::new().unwrap();
        let grammars = setup_grammars(&tmp);
        let bad = format!("{}/sources/../../../evil", grammars.display());
        let args = vec![SteelVal::StringV(bad.into()), SteelVal::StringV("x".into())];
        assert!(write_file(&args).is_err());
    }

    // ── servers/ sandbox (step 2: LSP server installer) ──────────────────────
    //
    // Every fs builtin widened to accept `<data>/servers/` in step 2 gets one
    // acceptance test here, plus a `setup` (plugins-only) rejection test where
    // not already covered above.

    #[test]
    fn write_file_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let f = servers.join("rust-analyzer/receipt.scm");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let args = vec![
            SteelVal::StringV(f.to_string_lossy().to_string().into()),
            SteelVal::StringV("(name . \"rust-analyzer\")".into()),
        ];
        assert_eq!(write_file(&args).unwrap(), SteelVal::Void);
        assert_eq!(
            fs::read_to_string(&f).unwrap(),
            "(name . \"rust-analyzer\")"
        );
    }

    #[test]
    fn delete_file_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, b"fake archive").unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        assert_eq!(delete_file(&args).unwrap(), SteelVal::Void);
        assert!(!f.exists());
    }

    #[test]
    fn delete_dir_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let server_dir = servers.join("rust-analyzer");
        fs::create_dir_all(&server_dir).unwrap();
        fs::write(server_dir.join("rust-analyzer"), b"binary").unwrap();

        let args = vec![SteelVal::StringV(
            server_dir.to_string_lossy().to_string().into(),
        )];
        assert!(delete_dir(&args).is_ok());
        assert!(!server_dir.exists());
    }

    #[test]
    fn make_dir_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let target = servers.join("rust-analyzer");
        let args = vec![SteelVal::StringV(
            target.to_string_lossy().to_string().into(),
        )];
        assert!(make_dir(&args).is_ok());
        assert!(target.is_dir());
    }

    #[test]
    fn list_dir_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        fs::create_dir_all(servers.join("rust-analyzer")).unwrap();
        let args = vec![SteelVal::StringV(
            servers.to_string_lossy().to_string().into(),
        )];
        let result = list_dir(&args).unwrap();
        assert_eq!(steel_list_to_strings(result), vec!["rust-analyzer"]);
    }

    #[test]
    fn read_file_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let f = servers.join("rust-analyzer/receipt.scm");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "(name . \"rust-analyzer\")").unwrap();
        let args = vec![SteelVal::StringV(f.to_string_lossy().to_string().into())];
        let result = read_file(&args).unwrap();
        assert!(
            matches!(result, SteelVal::StringV(s) if s.as_str() == "(name . \"rust-analyzer\")")
        );
    }

    #[test]
    fn path_exists_accepts_servers_dir() {
        let tmp = TempDir::new().unwrap();
        let servers = setup_servers(&tmp);
        let existing = servers.to_string_lossy().to_string();
        assert_eq!(
            path_exists(&[SteelVal::StringV(existing.into())]).unwrap(),
            SteelVal::BoolV(true)
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
