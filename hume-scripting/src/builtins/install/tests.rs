use super::*;
use crate::builtins::dirs::ScriptDirs;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use std::fs;
use tempfile::TempDir;

/// Point `h`'s directory state at a fresh `<tmp>/hume/servers/` and
/// return the canonical servers path.
fn setup(h: &mut SteelCtxTestHarness, tmp: &TempDir) -> PathBuf {
    let data_dir = tmp.path().join("hume");
    h.dirs = ScriptDirs::new(Some(data_dir.clone()), None);
    std::fs::canonicalize(data_dir.join("servers")).unwrap()
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
    let mut h = SteelCtxTestHarness::new();
    let servers = setup(&mut h, &tmp);
    let mut ctx = h.ctx();
    assert!(acquire_install_lock(&mut ctx).is_ok());
    assert!(servers.join(".install-lock").exists());
}

#[test]
fn acquire_install_lock_fails_loudly_on_a_second_live_acquire() {
    let tmp = TempDir::new().unwrap();
    let mut h = SteelCtxTestHarness::new();
    setup(&mut h, &tmp);
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
    let mut h = SteelCtxTestHarness::new();
    let servers = setup(&mut h, &tmp);
    let mut ctx = h.ctx();
    acquire_install_lock(&mut ctx).expect("first acquire");

    // Backdate the lock file's mtime past the 1h staleness threshold —
    // no real waiting required.
    let lock_path = servers.join(".install-lock");
    let file = OpenOptions::new().write(true).open(&lock_path).unwrap();
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
    let mut h = SteelCtxTestHarness::new();
    let servers = setup(&mut h, &tmp);
    let mut ctx = h.ctx();
    acquire_install_lock(&mut ctx).expect("first acquire");

    let lock_path = servers.join(".install-lock");
    let file = OpenOptions::new().write(true).open(&lock_path).unwrap();
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
    let mut h = SteelCtxTestHarness::new();
    setup(&mut h, &tmp);
    let mut ctx = h.ctx();
    assert!(
        release_install_lock(&mut ctx).is_ok(),
        "no lock ever acquired"
    );
    assert!(
        release_install_lock(&mut ctx).is_ok(),
        "second release call"
    );
}

#[test]
fn release_install_lock_removes_the_file_so_a_later_acquire_succeeds_immediately() {
    let tmp = TempDir::new().unwrap();
    let mut h = SteelCtxTestHarness::new();
    let servers = setup(&mut h, &tmp);
    let mut ctx = h.ctx();
    acquire_install_lock(&mut ctx).expect("first acquire");
    assert!(release_install_lock(&mut ctx).is_ok());
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

#[cfg(unix)]
mod unix;
