//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

// ── run_capture ────────────────────────────────────────────────────────────

/// A Ctrl-c delivered to the foreground process group must not reach a
/// `run_capture` child: it has to lead a group of its own.
#[test]
fn run_capture_child_leads_its_own_process_group() {
    use nix::unistd::getpgrp;

    let output = run_capture(
        "sh",
        &["-c".to_string(), "ps -o pgid= -p $$".to_string()],
        Path::new("."),
    )
    .expect("spawn sh");
    let child_pgid: i32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps prints the child's pgid");
    assert_ne!(
        child_pgid,
        getpgrp().as_raw(),
        "the child must not share HUME's process group"
    );
}

/// Guards the deadlock `run_capture`'s own doc comment documents: reading
/// stdout to EOF, then waiting, then reading stderr blocks forever once a child fills its stderr
/// pipe before exiting. `Command::output` drains both concurrently, so this
/// must return well under any reasonable test timeout rather than hang.
#[test]
fn run_capture_does_not_deadlock_on_large_stderr() {
    let out = run_capture(
        "sh",
        &["-c".to_string(), "yes | head -c 200000 1>&2".to_string()],
        Path::new("."),
    )
    .expect("run_capture");
    assert_eq!(out.stderr.len(), 200_000);
}

/// `run_capture`'s stdout counterpart to the stderr deadlock test above.
/// Pins that a large stdout stream is captured whole, not truncated at
/// whatever a pipe's OS buffer happens to hold.
#[test]
fn run_capture_captures_large_stdout_whole() {
    let out = run_capture(
        "sh",
        &["-c".to_string(), "yes | head -c 200000".to_string()],
        Path::new("."),
    )
    .expect("run_capture");
    assert_eq!(out.stdout.len(), 200_000);
}

/// `run_capture` must deny git a credential prompt: this call has no
/// terminal to put one on (stdin is closed), so left unset, `git` would try
/// `/dev/tty` directly and hang instead of failing fast.
#[test]
fn run_capture_sets_git_terminal_prompt_to_deny_credential_prompts() {
    let out = run_capture(
        "sh",
        &[
            "-c".to_string(),
            "printf %s \"$GIT_TERMINAL_PROMPT\"".to_string(),
        ],
        Path::new("."),
    )
    .expect("run_capture");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0");
}

// ── run_inline_output ─────────────────────────────────────────────────────

#[test]
fn run_inline_output_returns_exit_status_of_child() {
    let status = run_inline_output("true", &[], Path::new("."), &[]).expect("spawn true");
    assert!(status.success());

    let status = run_inline_output("false", &[], Path::new("."), &[]).expect("spawn false");
    assert!(!status.success());
}

/// `run_inline_output` puts the child in its own *background* process
/// group (for Ctrl-c safety; see the test below), not the terminal's
/// foreground one, so a credential prompt it wrote would be followed by a
/// read that hangs on `SIGTTIN` rather than an answerable question. Must
/// deny the prompt outright, the same as `run_capture`.
#[test]
fn run_inline_output_sets_git_terminal_prompt_to_deny_credential_prompts() {
    // Inherited stdio means the child's stdout is this test process's own, so
    // write to a file instead of asserting on captured output.
    let dir = tempfile::tempdir().expect("tempdir");
    let status = run_inline_output(
        "sh",
        &[
            "-c".to_string(),
            "printf %s \"$GIT_TERMINAL_PROMPT\" > out.txt".to_string(),
        ],
        dir.path(),
        &[],
    )
    .expect("spawn sh");
    assert!(status.success());
    let out = std::fs::read_to_string(dir.path().join("out.txt")).expect("read out.txt");
    assert_eq!(out, "0");
}

#[test]
fn run_inline_output_applies_env() {
    let dir = tempfile::tempdir().expect("tempdir");
    let status = run_inline_output(
        "sh",
        &[
            "-c".to_string(),
            "printf %s \"$HUME_ENV_PROBE\" > out.txt".to_string(),
        ],
        dir.path(),
        &[("HUME_ENV_PROBE".to_string(), "probe-value".to_string())],
    )
    .expect("spawn sh");
    assert!(status.success());
    let out = std::fs::read_to_string(dir.path().join("out.txt")).expect("read out.txt");
    assert_eq!(out, "probe-value");
}

#[test]
fn run_inline_output_honors_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("marker.txt"), b"hi").expect("write marker");
    let status = run_inline_output(
        "test",
        &["-f".to_string(), "marker.txt".to_string()],
        dir.path(),
        &[],
    )
    .expect("spawn test -f");
    assert!(status.success(), "marker.txt must be found via cwd");
}

/// Verify that Ctrl-c (SIGINT to the child's process group) kills the
/// child but not HUME.
///
/// Behavioral guarantee: after `process_group(0)` the child is its own
/// process group leader, so `killpg(child_pid, SIGINT)` targets only that
/// group.  If the test process survives past the assert the guarantee holds.
///
/// `nix::killpg` is used instead of spawning `kill -INT -<pgid>` because
/// BSD `kill` and util-linux `kill` disagree on negative-pgid argument
/// parsing: the Linux version returned exit 0 without signalling, causing
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
    // it first). Either way the group is correct.
    let _ = setpgid(pid, pid);

    killpg(pid, Signal::SIGINT).expect("killpg");

    // Wait for the child, which must have been killed by the signal.
    let exit = child.wait_with_output().expect("wait").status;
    assert!(
        !exit.success(),
        "child should have been killed by SIGINT, got: {exit:?}"
    );

    // Reaching here means HUME survived: the guarantee holds.
}

// ── spawn_in_own_group ──────────────────────────────────────────────────────

/// The precondition `tracked::TrackedChild`'s group-directed kill rests on:
/// without this, `killpg` on the child's own pid would target HUME's own
/// group instead (see `spawn_in_own_group`'s doc for the parent-side
/// `setpgid` race this closes).
#[test]
fn spawn_in_own_group_makes_the_child_its_own_group_leader() {
    use nix::unistd::{Pid, getpgid};

    let mut cmd = Command::new("sleep");
    cmd.arg("5");
    let mut child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = Pid::from_raw(i32::try_from(child.id()).expect("pid fits i32"));

    assert_eq!(
        getpgid(Some(pid)),
        Ok(pid),
        "the child must be its own process group leader immediately after spawn_in_own_group returns"
    );

    let _ = child.kill();
    let _ = child.wait();
}
