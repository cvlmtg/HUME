//! LSP server install pipeline builtins: platform identification, sha256
//! verification, archive unpacking, a `$PATH` lookup predicate, and a
//! cross-process install lock.
//!
//! sha256 verification and archive unpacking shell out to per-platform
//! system tools (`hume_platform::process`) rather than pulling in
//! hashing/archive crates — see `docs/LSP-INSTALL.md`'s "Required external
//! tools" note for exactly what each platform needs installed.
//!
//! | Steel name                     | Signature              | Notes                              |
//! |---------------------------------|------------------------|-------------------------------------|
//! | `hume-target`                   | `() → string \| #f`    | install-target id, or `#f`         |
//! | `verify-sha256!`                | `string string → void` | deletes `path` on mismatch         |
//! | `unpack-gz`                     | `string string → void` | sandboxed to `<data>/servers/`     |
//! | `unpack-zip`                    | `string string string → void` | sandboxed to `<data>/servers/`, bin-path chmod'd |
//! | `exe-on-path?`                  | `string → bool`        | real `PATH` scan, no spawn         |
//! | `acquire-install-lock!`         | `() → void`            | O_EXCL over `<data>/servers/.install-lock`; stale (>1h) → replace |
//! | `release-install-lock!`        | `() → void`            | idempotent — a missing lock is not an error |

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

use super::one_string;
use super::sandbox::with_data_servers;
use super::shell::{SandboxKind, validate_new_path};

const INSTALL_LOCK_FILE_NAME: &str = ".install-lock";

/// A lock file older than this is treated as abandoned by a crashed or
/// killed prior process — replaced with a warning, rather than left to
/// block every future install/uninstall forever.
const STALE_INSTALL_LOCK_AGE: Duration = Duration::from_secs(60 * 60);

/// Canonicalize `src` and verify it resolves inside `<data>/servers/`,
/// shared by every builtin here whose `src` argument must already exist in
/// that sandbox (`verify-sha256!`, `unpack-gz`, `unpack-zip`).
fn resolve_src_in_servers_sandbox(fn_name: &str, src: &str) -> Result<PathBuf, SteelErr> {
    let canonical = hume_platform::fs::canonicalize(&PathBuf::from(src)).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("{fn_name}: cannot resolve src '{src}': {e}"),
        )
    })?;
    if !super::sandbox::is_under_servers_sandbox(&canonical) {
        steel::stop!(Generic =>
            "{fn_name}: src is outside the write sandbox (<data>/servers/): {}", src);
    }
    Ok(canonical)
}

/// `(hume-target)` — the install-target identifier for the current platform
/// (`"darwin-arm64"`, `"darwin-x64"`, `"linux-x64"`, `"windows-x64"`), or
/// `#f` on any other platform/architecture. `#f`, not an error, so
/// `:lsp-servers` can render "unsupported platform" rather than aborting.
pub(crate) fn hume_target(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "hume-target expects 0 args, got {}", args.len());
    }
    Ok(match hume_platform::target::hume_target() {
        Some(t) => SteelVal::StringV(t.into()),
        None => SteelVal::BoolV(false),
    })
}

/// `(verify-sha256! path expected)` — verify `path`'s sha256 digest matches
/// `expected`. Accepts either the seeded data-file literal (`"sha256:<hex>"`)
/// or bare hex; the comparison is ASCII-case-insensitive.
///
/// `path` must resolve inside `<data>/servers/`. On mismatch, `path` is
/// deleted (mirrors `curl-fetch`'s partial-artifact cleanup) and the error
/// names the path plus both digests.
pub(crate) fn verify_sha256(
    ctx: &mut SteelCtx,
    path: String,
    expected: String,
) -> Result<SteelVal, SteelErr> {
    let canonical = resolve_src_in_servers_sandbox("verify-sha256!", &path)?;

    let expected_hex = expected
        .strip_prefix("sha256:")
        .unwrap_or(&expected)
        .to_ascii_lowercase();

    ctx.log(LogLevel::Trace, format!("verify-sha256!: hashing {path}"));

    let actual = hume_platform::process::sha256_file(&canonical).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("verify-sha256!: cannot hash '{path}': {e}"),
        )
    })?;

    if actual != expected_hex {
        let _ = hume_platform::fs::remove_file(&canonical);
        steel::stop!(Generic =>
            "verify-sha256!: sha256 mismatch for '{path}': expected {expected_hex}, got {actual}");
    }
    Ok(SteelVal::Void)
}

/// `(unpack-gz src dest)` — decode the single-file gzip archive at `src`
/// into `dest` (shells out to `gzip -dc`; on Unix, `dest` is chmod'd `0o755`
/// after success — Mason `.gz` assets are bare server executables).
///
/// `src` must resolve inside `<data>/servers/`; `dest` is validated as a new
/// path in the same sandbox. On error, any partial `dest` is removed before
/// raising (mirrors `curl-fetch`'s cleanup contract).
pub(crate) fn unpack_gz(
    ctx: &mut SteelCtx,
    src: String,
    dest: String,
) -> Result<SteelVal, SteelErr> {
    let canonical_src = resolve_src_in_servers_sandbox("unpack-gz", &src)?;
    let canonical_dest =
        validate_new_path(&PathBuf::from(&dest), "unpack-gz", SandboxKind::Servers)?;

    ctx.log(LogLevel::Trace, format!("unpack-gz: {src} → {dest}"));

    if let Err(e) = hume_platform::process::unpack_gz(&canonical_src, &canonical_dest) {
        let _ = hume_platform::fs::remove_file(&canonical_dest);
        steel::stop!(Generic => "unpack-gz: {}", e);
    }
    Ok(SteelVal::Void)
}

/// `(unpack-zip src dest-dir bin-path)` — extract the zip archive at `src`
/// into `dest-dir` (`unzip -o` on Unix, `tar -xf` on Windows), then verify
/// `bin-path` (relative to `dest-dir`) exists and — on Unix — chmod it
/// `0o755`. Unlike `.gz` (always a bare executable, chmod'd unconditionally),
/// zip entries carry the archive's own stored permissions and CI-built
/// release zips routinely strip the exec bit.
///
/// Zip-slip and symlink-entry protection is delegated to the system tool —
/// the residual risk is bounded by the sha256 pin verified before unpacking
/// (see `docs/LSP-INSTALL.md`'s accepted tradeoffs).
///
/// `src` must resolve inside `<data>/servers/`; `dest-dir` is validated as a
/// new path in the same sandbox and created if absent (`tar -C` requires an
/// existing directory). `bin-path` must not contain `..` components. On
/// error, `dest-dir` is left as-is — a dir-without-receipt is already the
/// interrupted-install signal the installer relies on, so cleaning up here
/// would duplicate that mechanism.
pub(crate) fn unpack_zip(
    ctx: &mut SteelCtx,
    src: String,
    dest_dir: String,
    bin_path: String,
) -> Result<SteelVal, SteelErr> {
    let canonical_src = resolve_src_in_servers_sandbox("unpack-zip", &src)?;
    if super::sandbox::has_dotdot(Path::new(&bin_path)) {
        steel::stop!(Generic =>
            "unpack-zip: bin-path must not contain '..' components: {}", bin_path);
    }
    let canonical_dest = validate_new_path(
        &PathBuf::from(&dest_dir),
        "unpack-zip",
        SandboxKind::Servers,
    )?;
    hume_platform::fs::create_dir_all(&canonical_dest).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("unpack-zip: cannot create dest dir '{dest_dir}': {e}"),
        )
    })?;

    ctx.log(
        LogLevel::Trace,
        format!("unpack-zip: {src} → {dest_dir} (bin: {bin_path})"),
    );

    hume_platform::process::unpack_zip(&canonical_src, &canonical_dest, Path::new(&bin_path))
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("unpack-zip: {e}")))?;
    Ok(SteelVal::Void)
}

/// `(exe-on-path? name)` → bool. A real `PATH` scan, no spawn — a lookup
/// predicate must be side-effect-free (some tools do real work on
/// `--version`). Rejects `name` containing a path separator (must be a bare
/// command name).
pub(crate) fn exe_on_path(args: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    let name = one_string(args, "exe-on-path?")?;
    Ok(SteelVal::BoolV(hume_platform::process::exe_on_search_path(
        &name,
    )))
}

/// Create `path` with O_EXCL semantics: errors with `AlreadyExists` if it's
/// already there. Content doesn't matter, only existence + mtime.
fn create_lock_file(path: &Path) -> std::io::Result<()> {
    OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(())
}

/// `(acquire-install-lock!)` — create `<data>/servers/.install-lock` with
/// O_EXCL semantics, guarding `:lsp-install`/`:lsp-uninstall` against a
/// second HUME process running one of them concurrently. A lock already
/// present and younger than an hour is a live install in progress
/// elsewhere — fails loudly. Older than that, it's treated as abandoned
/// (the process that held it crashed or was killed without releasing) and
/// replaced, with a warning.
///
/// # Errors
/// A live (non-stale) lock already exists, or the lock file can't be
/// created/replaced.
pub(crate) fn acquire_install_lock(ctx: &mut SteelCtx) -> Result<SteelVal, SteelErr> {
    with_data_servers(|servers_dir| {
        let lock_path = servers_dir.join(INSTALL_LOCK_FILE_NAME);
        let Err(create_err) = create_lock_file(&lock_path) else {
            return Ok(SteelVal::Void);
        };
        if create_err.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("acquire-install-lock!: {create_err}"),
            ));
        }
        let age = std::fs::metadata(&lock_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        match age {
            Some(age) if age <= STALE_INSTALL_LOCK_AGE => Err(SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                "acquire-install-lock!: another install/uninstall is already in progress"
                    .to_string(),
            )),
            _ => {
                ctx.log(
                    LogLevel::Warning,
                    "acquire-install-lock!: stale lock (older than 1h) — replacing".to_string(),
                );
                std::fs::remove_file(&lock_path).map_err(|e| {
                    SteelErr::new(
                        steel::rerrs::ErrorKind::Generic,
                        format!("acquire-install-lock!: cannot remove stale lock: {e}"),
                    )
                })?;
                create_lock_file(&lock_path).map_err(|e| {
                    SteelErr::new(
                        steel::rerrs::ErrorKind::Generic,
                        format!(
                            "acquire-install-lock!: cannot create lock after removing stale one: {e}"
                        ),
                    )
                })?;
                Ok(SteelVal::Void)
            }
        }
    })?
}

/// `(release-install-lock!)` — remove `<data>/servers/.install-lock`.
/// Idempotent: a missing lock (already released, or never acquired) is not
/// an error.
pub(crate) fn release_install_lock() -> Result<SteelVal, SteelErr> {
    with_data_servers(|servers_dir| {
        let lock_path = servers_dir.join(INSTALL_LOCK_FILE_NAME);
        match std::fs::remove_file(&lock_path) {
            Ok(()) => Ok(SteelVal::Void),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SteelVal::Void),
            Err(e) => Err(SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("release-install-lock!: {e}"),
            )),
        }
    })?
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        super::super::sandbox::init_dirs(Some(data_dir.clone()), None);
        data_dir.join("servers")
    }

    // ── hume-target ────────────────────────────────────────────────────────

    #[test]
    fn hume_target_returns_string_or_false() {
        let result = hume_target(&[]).unwrap();
        match result {
            SteelVal::StringV(s) => assert!(
                matches!(
                    s.as_str(),
                    "darwin-arm64" | "darwin-x64" | "linux-x64" | "windows-x64"
                ),
                "unexpected hume-target value: {s}"
            ),
            SteelVal::BoolV(false) => {}
            other => panic!("expected string or #f, got {other:?}"),
        }
    }

    #[test]
    fn hume_target_rejects_extra_args() {
        assert!(hume_target(&[SteelVal::StringV("x".into())]).is_err());
    }

    // ── verify-sha256! ─────────────────────────────────────────────────────

    #[test]
    fn verify_sha256_accepts_matching_digest_with_prefix() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let expected =
            "sha256:604f73953b84e48e552fea0b7fed0d938b038b5b1b18f7c10f5bb640ae5e9c40".to_string();
        let result = verify_sha256(&mut ctx, f.to_string_lossy().to_string(), expected);
        assert!(result.is_ok(), "expected ok, got {result:?}");
        assert!(f.exists(), "matching digest must not delete the file");
    }

    #[test]
    fn verify_sha256_accepts_bare_hex_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let expected =
            "604F73953B84E48E552FEA0B7FED0D938B038B5B1B18F7C10F5BB640AE5E9C40".to_string();
        assert!(verify_sha256(&mut ctx, f.to_string_lossy().to_string(), expected).is_ok());
    }

    #[test]
    fn verify_sha256_mismatch_deletes_file_and_raises() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let f = servers.join("rust-analyzer.gz");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = verify_sha256(
            &mut ctx,
            f.to_string_lossy().to_string(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("mismatch"),
            "expected mismatch error, got: {err}"
        );
        assert!(!f.exists(), "mismatched digest must delete the file");
    }

    #[test]
    fn verify_sha256_rejects_path_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        let outside = tmp.path().join("hume/plugins");
        fs::create_dir_all(&outside).unwrap();
        let f = outside.join("evil.gz");
        fs::write(&f, b"data").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = verify_sha256(
            &mut ctx,
            f.to_string_lossy().to_string(),
            "sha256:00".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    // ── unpack-gz / unpack-zip sandbox rejections ───────────────────────────
    //
    // Happy-path unpack behavior (round-trip content, exec bit, zip entries)
    // is covered by `hume-platform`'s own tests against the real system
    // tools; these tests pin the Steel-boundary sandbox contract only.

    #[test]
    fn unpack_gz_rejects_src_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let outside = tmp.path().join("hume/plugins/evil.gz");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, b"data").unwrap();
        let dest = servers.join("out-bin").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_gz(&mut ctx, outside.to_string_lossy().to_string(), dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_gz_rejects_dest_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let src = servers.join("rust-analyzer.gz");
        fs::write(&src, b"data").unwrap();
        let bad_dest = tmp.path().join("evil-bin").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_gz(&mut ctx, src.to_string_lossy().to_string(), bad_dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_zip_rejects_src_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let outside = tmp.path().join("hume/plugins/evil.zip");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, b"data").unwrap();
        let dest_dir = servers.join("out-dir").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_zip(
            &mut ctx,
            outside.to_string_lossy().to_string(),
            dest_dir,
            "bin".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_zip_rejects_dest_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let src = servers.join("rust-analyzer.zip");
        fs::write(&src, b"data").unwrap();
        let bad_dest_dir = tmp.path().join("evil-dir").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_zip(
            &mut ctx,
            src.to_string_lossy().to_string(),
            bad_dest_dir,
            "bin".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn unpack_zip_rejects_bin_path_with_dotdot() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let src = servers.join("rust-analyzer.zip");
        fs::write(&src, b"data").unwrap();
        let dest_dir = servers.join("out-dir").to_string_lossy().to_string();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = unpack_zip(
            &mut ctx,
            src.to_string_lossy().to_string(),
            dest_dir,
            "../evil".to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(".."),
            "expected dotdot-rejection error, got: {err}"
        );
    }

    // ── exe-on-path? ─────────────────────────────────────────────────────────

    #[test]
    fn exe_on_path_rejects_path_separator_names() {
        assert_eq!(
            exe_on_path(&[SteelVal::StringV("some/path".into())]).unwrap(),
            SteelVal::BoolV(false)
        );
    }

    #[test]
    fn exe_on_path_missing_tool_is_false() {
        assert_eq!(
            exe_on_path(&[SteelVal::StringV("definitely-not-a-real-tool-xyz".into())]).unwrap(),
            SteelVal::BoolV(false)
        );
    }

    // ── acquire-install-lock! / release-install-lock! ───────────────────────

    #[test]
    fn acquire_install_lock_succeeds_when_no_lock_exists() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(acquire_install_lock(&mut ctx).is_ok());
        assert!(servers.join(".install-lock").exists());
    }

    #[test]
    fn acquire_install_lock_fails_loudly_on_a_second_live_acquire() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        acquire_install_lock(&mut ctx).expect("first acquire");
        let err = acquire_install_lock(&mut ctx).unwrap_err();
        assert!(
            err.to_string().contains("already in progress"),
            "expected an 'already in progress' error, got: {err}"
        );
    }

    #[test]
    fn acquire_install_lock_replaces_a_stale_lock_with_a_warning() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        acquire_install_lock(&mut ctx).expect("first acquire");

        // Backdate the lock file's mtime past the 1h staleness threshold —
        // no real waiting required.
        let lock_path = servers.join(".install-lock");
        let file = fs::File::open(&lock_path).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(60 * 60 + 1))
            .unwrap();

        assert!(
            acquire_install_lock(&mut ctx).is_ok(),
            "a stale lock must be replaced, not treated as live"
        );
        assert!(
            h.pending_messages
                .iter()
                .any(|(level, msg)| *level == LogLevel::Warning && msg.contains("stale")),
            "replacing a stale lock must log a warning: {:?}",
            h.pending_messages
        );
    }

    #[test]
    fn release_install_lock_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);
        assert!(release_install_lock().is_ok(), "no lock ever acquired");
        assert!(release_install_lock().is_ok(), "second release call");
    }

    #[test]
    fn release_install_lock_removes_the_file_so_a_later_acquire_succeeds_immediately() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        acquire_install_lock(&mut ctx).expect("first acquire");
        assert!(release_install_lock().is_ok());
        assert!(!servers.join(".install-lock").exists());
        assert!(
            acquire_install_lock(&mut ctx).is_ok(),
            "a released lock must not block the next acquire"
        );
    }
}
