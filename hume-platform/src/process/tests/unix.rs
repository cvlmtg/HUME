//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

// ── run_inline_output ─────────────────────────────────────────────────────

#[test]
fn run_inline_output_returns_exit_status_of_child() {
    let status = run_inline_output("true", &[], None).expect("spawn true");
    assert!(status.success());

    let status = run_inline_output("false", &[], None).expect("spawn false");
    assert!(!status.success());
}

#[test]
fn run_inline_output_honors_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("marker.txt"), b"hi").expect("write marker");
    let status = run_inline_output(
        "test",
        &["-f".to_string(), "marker.txt".to_string()],
        Some(dir.path()),
    )
    .expect("spawn test -f");
    assert!(status.success(), "marker.txt must be found via cwd");
}

/// Verify that Ctrl+C (SIGINT to the child's process group) kills the
/// child but not HUME.
///
/// Behavioral guarantee: after `process_group(0)` the child is its own
/// process group leader, so `killpg(child_pid, SIGINT)` targets only that
/// group.  If the test process survives past the assert the guarantee holds.
///
/// `nix::killpg` is used instead of spawning `kill -INT -<pgid>` because
/// BSD `kill` and util-linux `kill` disagree on negative-pgid argument
/// parsing — the Linux version returned exit 0 without signalling, causing
/// `sleep` to run to completion and the test to fail.
#[test]
fn sigint_to_child_group_does_not_kill_hume() {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::{Pid, setpgid};
    use std::process::Command;

    // Spawn a long-lived child so we can signal it before it exits.
    let child = Command::new("sleep")
        .arg("30")
        .new_process_group()
        .spawn()
        .expect("spawn sleep");
    let pid = Pid::from_raw(i32::try_from(child.id()).expect("pid fits i32"));

    // `process_group(0)` calls setpgid(0,0) in the child's pre-exec hook,
    // which races with the parent.  Calling setpgid(child, child) from the
    // parent is idempotent and closes the race: if the child hasn't run its
    // hook yet we set it; if it already exec'd we get EACCES (the child set
    // it first) — either way the group is correct.
    let _ = setpgid(pid, pid);

    killpg(pid, Signal::SIGINT).expect("killpg");

    // Wait for the child — must have been killed by the signal.
    let exit = child.wait_with_output().expect("wait").status;
    assert!(
        !exit.success(),
        "child should have been killed by SIGINT, got: {exit:?}"
    );

    // Reaching here means HUME survived — the guarantee holds.
}

// ── unpack_gz ──────────────────────────────────────────────────────────────

#[test]
fn unpack_gz_round_trip_sets_exec_bit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let plain = dir.path().join("server-bin");
    std::fs::write(&plain, b"#!/bin/sh\necho hi\n").expect("write plain");

    let gz_status = Command::new("gzip")
        .arg("-k") // keep the original, we only need the .gz for the test
        .arg(&plain)
        .status()
        .expect("spawn gzip");
    assert!(gz_status.success());
    let gz_path = dir.path().join("server-bin.gz");

    let dest = dir.path().join("unpacked-bin");
    unpack_gz(&gz_path, &dest).expect("unpack_gz");

    let contents = std::fs::read(&dest).expect("read unpacked");
    assert_eq!(contents, b"#!/bin/sh\necho hi\n");
    let mode = std::fs::metadata(&dest)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755, "unpacked binary must be executable");
}

#[test]
fn unpack_gz_missing_source_is_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("missing.gz");
    let dest = dir.path().join("dest");
    assert!(unpack_gz(&src, &dest).is_err());
}

// ── unpack_zip ─────────────────────────────────────────────────────────────

#[test]
fn unpack_zip_round_trip_sets_exec_bit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).expect("mkdir src");
    // Default create mode (0o644, no exec bit) — matches what CI-built
    // release zips routinely ship, the exact case this test guards.
    std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");

    let zip_path = dir.path().join("archive.zip");
    let zip_status = Command::new("zip")
        .arg("-j") // junk paths — we only care about the entry name, not src/
        .arg(&zip_path)
        .arg(src_dir.join("bin-name"))
        .status()
        .expect("spawn zip");
    assert!(zip_status.success());

    let dest_dir = dir.path().join("dest");
    std::fs::create_dir(&dest_dir).expect("mkdir dest");
    unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

    let unpacked = dest_dir.join("bin-name");
    let contents = std::fs::read(&unpacked).expect("read unpacked entry");
    assert_eq!(contents, b"binary contents");
    let mode = std::fs::metadata(&unpacked)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o755,
        "unpacked zip binary must be executable"
    );
}

#[test]
fn unpack_zip_chmods_every_regular_file_not_just_bin_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).expect("mkdir src");
    std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");
    // A sibling helper/wrapper the archive ships alongside the seeded
    // entry point — must end up executable too, not just "bin-name".
    std::fs::write(src_dir.join("helper"), b"helper contents").expect("write fixture");

    let zip_path = dir.path().join("archive.zip");
    let zip_status = Command::new("zip")
        .arg("-j")
        .arg(&zip_path)
        .arg(src_dir.join("bin-name"))
        .arg(src_dir.join("helper"))
        .status()
        .expect("spawn zip");
    assert!(zip_status.success());

    let dest_dir = dir.path().join("dest");
    std::fs::create_dir(&dest_dir).expect("mkdir dest");
    unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

    let helper_mode = std::fs::metadata(dest_dir.join("helper"))
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(
        helper_mode & 0o777,
        0o755,
        "a sibling file in the archive must also end up executable"
    );
}

#[test]
fn unpack_zip_never_follows_a_symlink_entry_for_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let outside_target = dir.path().join("outside-target.txt");
    std::fs::write(&outside_target, b"secret").expect("write fixture");
    std::fs::set_permissions(&outside_target, std::fs::Permissions::from_mode(0o600))
        .expect("chmod fixture to non-executable");

    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).expect("mkdir src");
    std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");
    std::os::unix::fs::symlink(&outside_target, src_dir.join("link-name"))
        .expect("create symlink fixture");

    let zip_path = dir.path().join("archive.zip");
    let zip_status = Command::new("zip")
        .arg("--symlinks")
        .arg("-j")
        .arg(&zip_path)
        .arg(src_dir.join("bin-name"))
        .arg(src_dir.join("link-name"))
        .status()
        .expect("spawn zip");
    assert!(zip_status.success());

    let dest_dir = dir.path().join("dest");
    std::fs::create_dir(&dest_dir).expect("mkdir dest");
    unpack_zip(&zip_path, &dest_dir, Path::new("bin-name")).expect("unpack_zip");

    assert!(
        std::fs::symlink_metadata(dest_dir.join("link-name"))
            .expect("extracted entry must exist")
            .file_type()
            .is_symlink(),
        "the archive's symlink entry must extract as a real symlink, not get dereferenced"
    );
    let target_mode = std::fs::metadata(&outside_target)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(
        target_mode & 0o777,
        0o600,
        "chmod must never follow the symlink to its target outside dest_dir"
    );
}

#[test]
fn unpack_zip_missing_source_is_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("missing.zip");
    let dest_dir = dir.path().join("dest");
    std::fs::create_dir(&dest_dir).expect("mkdir dest");
    assert!(unpack_zip(&src, &dest_dir, Path::new("bin-name")).is_err());
}

#[test]
fn unpack_zip_missing_expected_binary_is_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).expect("mkdir src");
    std::fs::write(src_dir.join("bin-name"), b"binary contents").expect("write fixture");

    let zip_path = dir.path().join("archive.zip");
    let zip_status = Command::new("zip")
        .arg("-j")
        .arg(&zip_path)
        .arg(src_dir.join("bin-name"))
        .status()
        .expect("spawn zip");
    assert!(zip_status.success());

    let dest_dir = dir.path().join("dest");
    std::fs::create_dir(&dest_dir).expect("mkdir dest");
    let err = unpack_zip(&zip_path, &dest_dir, Path::new("wrong-name")).unwrap_err();
    assert!(
        err.to_string().contains("wrong-name"),
        "expected error naming the missing binary, got: {err}"
    );
}

