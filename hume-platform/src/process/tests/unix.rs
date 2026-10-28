//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;
use crate::path::test_support::write_script;

// ── Launch ─────────────────────────────────────────────────────────────────

/// A bare program name is looked up on the launch's search path, which need
/// not be the process's own `PATH`.
#[test]
fn a_bare_program_name_is_found_on_the_launch_search_path() {
    let bin = tempfile::tempdir().unwrap();
    write_script(bin.path(), "hume-launch-probe", "echo found\n", 0o755);

    let output = run_capture(
        "hume-launch-probe",
        &[],
        Launch {
            cwd: Path::new("."),
            search_path: Some(bin.path().as_os_str()),
        },
    )
    .expect("spawn the probe");

    assert_eq!(output.stdout, b"found\n");
}

/// Asserts that `printed`, a child's `ps -o pgid=` output, names this
/// process's own group.
fn assert_in_callers_group(printed: &[u8]) {
    let pgid: i32 = String::from_utf8_lossy(printed)
        .trim()
        .parse()
        .expect("ps prints the child's pgid");
    assert_eq!(pgid, nix::unistd::getpgrp().as_raw());
}

// ── run_capture ────────────────────────────────────────────────────────────

/// A terminal Ctrl-c during an `#:inline-output` command reaches the
/// foreground process group, so the child must be in the caller's group to
/// be interrupted by it.
#[test]
fn run_capture_child_shares_the_callers_process_group() {
    let output = run_capture(
        "sh",
        &["-c".to_string(), "ps -o pgid= -p $$".to_string()],
        Launch::with_process_path(Path::new(".")),
    )
    .expect("spawn sh");
    assert_in_callers_group(&output.stdout);
}

/// Guards the deadlock this module's doc describes: reading stdout to EOF,
/// then waiting, then reading stderr blocks forever once a child fills its
/// stderr pipe before exiting. Both pipes are drained concurrently, so this
/// returns rather than hangs, keeping the stream's first and last 4 KiB.
#[test]
fn run_capture_drains_a_large_stderr_and_keeps_its_head_and_tail() {
    let out = run_capture(
        "sh",
        &["-c".to_string(), "yes | head -c 200000 1>&2".to_string()],
        Launch::with_process_path(Path::new(".")),
    )
    .expect("run_capture");
    assert_eq!(
        decode_lossy(out.stderr),
        test_support::kept_stderr(&"y\n".repeat(100_000))
    );
    assert!(out.status.success());
}

/// `run_capture`'s stdout counterpart to the stderr deadlock test above.
/// Pins that a large stdout stream is captured whole, not truncated at
/// whatever a pipe's OS buffer happens to hold.
#[test]
fn run_capture_captures_large_stdout_whole() {
    let out = run_capture(
        "sh",
        &["-c".to_string(), "yes | head -c 200000".to_string()],
        Launch::with_process_path(Path::new(".")),
    )
    .expect("run_capture");
    assert_eq!(out.stdout.len(), 200_000);
}

/// A program writing more stdout than the capture keeps fails the capture,
/// naming the program, rather than handing back a prefix.
#[test]
fn run_capture_fails_past_the_stdout_cap() {
    let captured = run_capture(
        "sh",
        &[
            "-c".to_string(),
            format!("head -c {} /dev/zero", 64 * 1024 * 1024 + 1),
        ],
        Launch::with_process_path(Path::new(".")),
    );
    let msg = match captured {
        Ok(out) => panic!("captured {} bytes of stdout", out.stdout.len()),
        Err(e) => e.to_string(),
    };
    assert!(msg.starts_with("sh: "), "{msg}");
    assert!(msg.contains("output exceeded 67108864 bytes"), "{msg}");
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
        Launch::with_process_path(Path::new(".")),
    )
    .expect("run_capture");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0");
}

// ── run_inline_output ─────────────────────────────────────────────────────

#[test]
fn run_inline_output_returns_exit_status_of_child() {
    let status = run_inline_output("true", &[], Launch::with_process_path(Path::new(".")), &[])
        .expect("spawn true");
    assert!(status.success());

    let status = run_inline_output("false", &[], Launch::with_process_path(Path::new(".")), &[])
        .expect("spawn false");
    assert!(!status.success());
}

/// A git credential prompt is denied for every spawn in this crate, the same
/// as for `run_capture`.
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
        Launch::with_process_path(dir.path()),
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
        Launch::with_process_path(dir.path()),
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
        Launch::with_process_path(dir.path()),
        &[],
    )
    .expect("spawn test -f");
    assert!(status.success(), "marker.txt must be found via cwd");
}

/// The child must be in the caller's foreground group, where a terminal
/// Ctrl-c during the inline-output frame reaches it.
#[test]
fn run_inline_output_child_shares_the_callers_process_group() {
    let dir = tempfile::tempdir().expect("tempdir");
    let status = run_inline_output(
        "sh",
        &["-c".to_string(), "ps -o pgid= -p $$ > out.txt".to_string()],
        Launch::with_process_path(dir.path()),
        &[],
    )
    .expect("spawn sh");
    assert!(status.success());
    let printed = std::fs::read(dir.path().join("out.txt")).expect("read out.txt");
    assert_in_callers_group(&printed);
}

// ── grammar_build ─────────────────────────────────────────────────────────

/// `compile-grammar!` runs the compiler inside the inline-output frame, so it
/// must be interruptible by a terminal Ctrl-c the same way.
#[test]
fn grammar_build_child_shares_the_callers_process_group() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Called as `tool -o OUT SRC`: writes its pgid to OUT.
    let tool = write_script(
        dir.path(),
        "fake-grammar-build",
        "ps -o pgid= -p $$ > \"$2\"\n",
        0o755,
    );
    let out = dir.path().join("out.txt");

    let output = grammar_build(
        &tool,
        dir.path(),
        &out,
        Launch::with_process_path(dir.path()),
    )
    .expect("spawn the tool");
    assert!(output.status.success());
    let printed = std::fs::read(&out).expect("read out.txt");
    assert_in_callers_group(&printed);
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

    let _ = child.child_mut().kill();
    let _ = child.child_mut().wait();
}
