//! LSP server install pipeline builtins: platform identification, sha256
//! hashing, archive unpacking, and a cross-process install lock.
//!
//! No sandbox checks — full-trust plugin model (see `docs/ROADMAP.md`'s
//! plugin trust model decision). sha256 hashing and archive unpacking shell
//! out to per-platform system tools (`hume_platform::process`) rather than
//! pulling in hashing/archive crates — see `docs/LSP-INSTALL.md`'s "Required
//! external tools" note for exactly what each platform needs installed.
//!
//! | Steel name                     | Signature              | Notes                              |
//! |---------------------------------|------------------------|-------------------------------------|
//! | `hume-target`                   | `() → string \| #f`    | install-target id, or `#f`         |
//! | `sha256-file`                   | `string → string`      | sha256 digest as lowercase hex     |
//! | `unpack-gz`                     | `string string → void` | `gzip -dc`, chmod 0755 on Unix     |
//! | `unpack-zip`                    | `string string string → void` | `unzip`/`tar`, bin-path chmod'd |
//! | `acquire-install-lock!`         | `() → void`            | O_EXCL over `<data>/servers/.install-lock`; stale (>1h) → replace |
//! | `release-install-lock!`        | `() → void`            | idempotent — a missing lock is not an error |
//! | `%run-inline-output!`           | `string list string|#f → int` | process-group-isolated spawn for `#:inline-output` commands; see `run_inline_output` doc |

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use crate::SteelCtx;
use crate::log::LogLevel;

use super::sandbox::with_data_servers;
use super::{list_to_strings, string_arg};

const INSTALL_LOCK_FILE_NAME: &str = ".install-lock";

/// A lock file older than this is treated as abandoned by a crashed or
/// killed prior process — replaced with a warning, rather than left to
/// block every future install/uninstall forever.
const STALE_INSTALL_LOCK_AGE: Duration = Duration::from_secs(60 * 60);

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

/// `(sha256-file path)` — the sha256 digest of `path` as lowercase hex.
///
/// No sandbox check and no compare/delete logic — full-trust plugin model
/// (see `docs/ROADMAP.md`'s plugin trust model decision). Compare-and-delete-
/// on-mismatch, previously done here in the removed `verify-sha256!`
/// builtin, now lives in Scheme (`plum/verify-sha256!` in `servers.scm`) —
/// this is a thin wrapper over the platform tool selection (`shasum`/
/// `sha256sum`/`certutil`) that a Scheme rewrite would only make worse.
pub(crate) fn sha256_file(ctx: &mut SteelCtx, path: String) -> Result<SteelVal, SteelErr> {
    ctx.log(LogLevel::Trace, format!("sha256-file: hashing {path}"));
    let digest = hume_platform::process::sha256_file(Path::new(&path)).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("sha256-file: cannot hash '{path}': {e}"),
        )
    })?;
    digest.into_steelval().map_err(super::conv_err)
}

/// `(unpack-gz src dest)` — decode the single-file gzip archive at `src`
/// into `dest` (shells out to `gzip -dc`; on Unix, `dest` is chmod'd `0o755`
/// after success — Mason `.gz` assets are bare server executables).
///
/// On error, any partial `dest` is removed before raising.
pub(crate) fn unpack_gz(
    ctx: &mut SteelCtx,
    src: String,
    dest: String,
) -> Result<SteelVal, SteelErr> {
    let src_path = PathBuf::from(&src);
    let dest_path = PathBuf::from(&dest);

    ctx.log(LogLevel::Trace, format!("unpack-gz: {src} → {dest}"));

    if let Err(e) = hume_platform::process::unpack_gz(&src_path, &dest_path) {
        let _ = hume_platform::fs::remove_file(&dest_path);
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
/// (see `docs/LSP-INSTALL.md`'s accepted tradeoffs). `dest-dir` is created if
/// absent (`tar -C` requires an existing directory). On error, `dest-dir` is
/// left as-is — a dir-without-receipt is already the interrupted-install
/// signal the installer relies on, so cleaning up here would duplicate that
/// mechanism.
pub(crate) fn unpack_zip(
    ctx: &mut SteelCtx,
    src: String,
    dest_dir: String,
    bin_path: String,
) -> Result<SteelVal, SteelErr> {
    let src_path = PathBuf::from(&src);
    let dest_path = PathBuf::from(&dest_dir);
    hume_platform::fs::create_dir_all(&dest_path).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("unpack-zip: cannot create dest dir '{dest_dir}': {e}"),
        )
    })?;

    ctx.log(
        LogLevel::Trace,
        format!("unpack-zip: {src} → {dest_dir} (bin: {bin_path})"),
    );

    // `unzip`/`tar` inherit stdio (see `hume_platform::process::unpack_zip`'s
    // doc), so this is a real terminal write — open the bracket first.
    ctx.host
        .ensure_inline_output_screen()
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("unpack-zip: {e}")))?;
    hume_platform::process::unpack_zip(&src_path, &dest_path, Path::new(&bin_path))
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("unpack-zip: {e}")))?;
    Ok(SteelVal::Void)
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
/// present and *provably* older than an hour is treated as abandoned (the
/// process that held it crashed or was killed without releasing) and
/// replaced, with a warning. Everything else — younger than an hour, or an
/// age that can't be positively determined at all (unreadable metadata, or
/// a future mtime from clock skew / a networked or synced filesystem) —
/// is treated as live: deleting a lock we can't prove abandoned risks two
/// installs racing on the same server directory.
///
/// # Errors
/// A live (or indeterminate-age) lock already exists, or the lock file
/// can't be created/replaced.
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
        // `duration_since` errors (rather than defaulting to "unknown") on a
        // future mtime — clock skew or a networked/synced filesystem — which
        // is exactly the case that must NOT be treated as stale.
        let is_stale = std::fs::metadata(&lock_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > STALE_INSTALL_LOCK_AGE);
        if !is_stale {
            return Err(SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                "acquire-install-lock!: another install/uninstall is already in progress"
                    .to_string(),
            ));
        }
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
    })?
}

/// `(%run-inline-output! cmd args cwd)` — spawn `cmd` with `args` (a list of
/// strings), inherited stdio, in its own process group; blocks until exit and
/// returns the exit code as an int. `cwd` is a string or `#f`.
///
/// The process-group isolation is the entire reason this is a Rust builtin
/// rather than Steel's own `spawn-process`: `#:inline-output` commands run
/// with terminal raw mode off (see `run_inline_output`'s doc comment in
/// `hume-platform::process`), so an unisolated child would be killed by the
/// same Ctrl+C that's meant to interrupt only it. No sandbox checks — plugins
/// are trusted code (see `docs/ROADMAP.md`'s plugin trust model decision).
///
/// # Errors
/// The binary can't be spawned (e.g. not found on `PATH`).
pub(crate) fn run_inline_output(
    ctx: &mut SteelCtx,
    cmd: String,
    args_val: SteelVal,
    cwd_val: SteelVal,
) -> Result<SteelVal, SteelErr> {
    let args = list_to_strings(args_val, "%run-inline-output! args")?;
    let cwd = match cwd_val {
        SteelVal::BoolV(false) => None,
        other => Some(PathBuf::from(string_arg(other, "%run-inline-output! cwd")?)),
    };

    // The child inherits stdio, so this is a real terminal write — open the
    // bracket before spawning it.
    ctx.host.ensure_inline_output_screen().map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("run-inline-output!: {e}"),
        )
    })?;

    let status =
        hume_platform::process::run_inline_output(&cmd, &args, cwd.as_deref()).map_err(|e| {
            SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("run-inline-output!: cannot run '{cmd}': {e}"),
            )
        })?;

    // `-1` for a signal-killed child (no exit code) — matches the sentinel a
    // real exit code can never produce, since process exit codes are u8-wide.
    Ok(SteelVal::IntV(status.code().unwrap_or(-1) as isize))
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
    use crate::null_host::RecordingInlineOutputHost;
    use crate::test_support::SteelCtxTestHarness;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> PathBuf {
        let data_dir = tmp.path().join("hume");
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

    // ── sha256-file ──────────────────────────────────────────────────────────

    #[test]
    fn sha256_file_returns_lowercase_hex_digest() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("fixture.bin");
        fs::write(&f, b"hume").unwrap();

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = sha256_file(&mut ctx, f.to_string_lossy().to_string()).unwrap();
        assert_eq!(
            result,
            SteelVal::StringV(
                "604f73953b84e48e552fea0b7fed0d938b038b5b1b18f7c10f5bb640ae5e9c40".into()
            )
        );
    }

    #[test]
    fn sha256_file_missing_source_is_error() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("does-not-exist.bin");

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(sha256_file(&mut ctx, f.to_string_lossy().to_string()).is_err());
    }

    // ── unpack-gz / unpack-zip ───────────────────────────────────────────────
    //
    // Round-trip behavior (content, exec bit, zip entries, symlink safety)
    // is covered by `hume-platform`'s own tests against the real system
    // tools; these tests pin the Steel-boundary argument wiring and error
    // propagation only — no sandbox checks (full-trust plugin model).

    #[test]
    fn unpack_gz_missing_src_is_error() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("does-not-exist.gz");
        let dest = tmp.path().join("out-bin");

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(
            unpack_gz(
                &mut ctx,
                src.to_string_lossy().to_string(),
                dest.to_string_lossy().to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn unpack_zip_missing_src_is_error() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("does-not-exist.zip");
        let dest_dir = tmp.path().join("out-dir");

        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(
            unpack_zip(
                &mut ctx,
                src.to_string_lossy().to_string(),
                dest_dir.to_string_lossy().to_string(),
                "bin".to_string(),
            )
            .is_err()
        );
    }

    /// `unpack-zip` shells out to `unzip`/`tar` with inherited stdio — it
    /// must open the inline-output bracket before spawning that tool, even
    /// when the spawn itself then fails (missing src).
    #[test]
    fn unpack_zip_calls_ensure_before_unzip() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("does-not-exist.zip");
        let dest_dir = tmp.path().join("out-dir");

        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_with_host(&mut host);
        let _ = unpack_zip(
            &mut ctx,
            src.to_string_lossy().to_string(),
            dest_dir.to_string_lossy().to_string(),
            "bin".to_string(),
        );
        drop(ctx);
        assert_eq!(host.ensure_calls, 1);
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

    /// Regression: a lock file with an mtime in the FUTURE (clock skew, or a
    /// networked/synced filesystem racing the write) must never be treated
    /// as stale — `duration_since` errors on a future mtime, and that error
    /// must fall on the "live, don't delete" side, not the "unknown age,
    /// assume abandoned" side.
    #[test]
    fn acquire_install_lock_treats_a_future_mtime_lock_as_live() {
        let tmp = TempDir::new().unwrap();
        let servers = setup(&tmp);
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        acquire_install_lock(&mut ctx).expect("first acquire");

        let lock_path = servers.join(".install-lock");
        let file = fs::File::open(&lock_path).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(60 * 60))
            .unwrap();

        let err = acquire_install_lock(&mut ctx).unwrap_err();
        assert!(
            err.to_string().contains("already in progress"),
            "a future-dated mtime must not be treated as stale, got: {err}"
        );
        assert!(
            lock_path.exists(),
            "the live lock must not be deleted when its age can't be determined"
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

    // ── run_inline_output (%run-inline-output!) ─────────────────────────────

    fn list_val(items: &[&str]) -> SteelVal {
        use steel::rvals::IntoSteelVal as _;
        items
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_steelval()
            .unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn run_inline_output_returns_exit_code() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = run_inline_output(
            &mut ctx,
            "true".to_string(),
            list_val(&[]),
            SteelVal::BoolV(false),
        )
        .unwrap();
        assert_eq!(result, SteelVal::IntV(0));

        let result = run_inline_output(
            &mut ctx,
            "false".to_string(),
            list_val(&[]),
            SteelVal::BoolV(false),
        )
        .unwrap();
        assert_eq!(result, SteelVal::IntV(1));
    }

    #[test]
    #[cfg(unix)]
    fn run_inline_output_honors_cwd_arg() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("marker.txt"), b"hi").unwrap();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = run_inline_output(
            &mut ctx,
            "test".to_string(),
            list_val(&["-f", "marker.txt"]),
            SteelVal::StringV(tmp.path().to_string_lossy().into_owned().into()),
        )
        .unwrap();
        assert_eq!(
            result,
            SteelVal::IntV(0),
            "marker.txt must be found via cwd"
        );
    }

    #[test]
    fn run_inline_output_missing_binary_raises() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = run_inline_output(
            &mut ctx,
            "definitely-not-a-real-binary-xyz".to_string(),
            list_val(&[]),
            SteelVal::BoolV(false),
        )
        .unwrap_err();
        assert!(err.to_string().contains("definitely-not-a-real-binary-xyz"));
    }

    /// The spawned process inherits stdio — the bracket must open before the
    /// spawn attempt, even when the spawn itself then fails.
    #[test]
    fn run_inline_output_calls_ensure_before_spawn() {
        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_with_host(&mut host);
        let _ = run_inline_output(
            &mut ctx,
            "definitely-not-a-real-binary-xyz".to_string(),
            list_val(&[]),
            SteelVal::BoolV(false),
        );
        drop(ctx);
        assert_eq!(host.ensure_calls, 1);
    }

    #[test]
    #[cfg(unix)]
    fn run_inline_output_scheme_wrapper_raises_on_nonzero_exit() {
        // End-to-end through the BOOTSTRAP `run-inline-output!` Scheme wrapper
        // (the #:cwd keyword sugar + raise-on-nonzero contract), not just the
        // raw `%run-inline-output!`.
        use crate::ScriptingHost;
        use crate::null_host::NullHost;

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let ok_src = r#"(run-inline-output! "true" '())"#;
        host.eval_source(ok_src, &mut null_host)
            .expect("run-inline-output! success path must not raise");

        let mut host2 = ScriptingHost::new();
        let mut null_host2 = NullHost;
        let fail_src = r#"
            (with-handler
              (lambda (err)
                (if (string-contains? (to-string err) "false")
                    (begin)
                    (error (string-append "error did not name cmd: " (to-string err)))))
              (begin
                (run-inline-output! "false" '())
                (error "expected run-inline-output! to raise on nonzero exit")))
        "#;
        host2
            .eval_source(fail_src, &mut null_host2)
            .expect("run-inline-output! failure-path assertion failed");
    }
}
