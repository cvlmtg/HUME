//! Windows-only tests, gated once at the `mod windows;` declaration in the
//! parent. Each test builds its own [`ChildRegistry`], for the reason
//! `unix.rs` gives.

use super::super::*;
use crate::process::spawn_in_own_group;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(10);

/// What PowerShell prints for `script`, trimmed.
fn powershell(script: &str) -> String {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("run powershell");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The processes `pid` started that are still running.
fn children_of(pid: u32) -> Vec<u32> {
    powershell(&format!(
        "(Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}').ProcessId"
    ))
    .lines()
    .filter_map(|line| line.trim().parse().ok())
    .collect()
}

fn is_running(pid: u32) -> bool {
    powershell(&format!(
        "[bool](Get-Process -Id {pid} -ErrorAction SilentlyContinue)"
    )) == "True"
}

/// Polls `check` until it holds, for at most [`BOUND`].
fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + BOUND;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn reap_ends_the_processes_the_child_started() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "ping", "-n", "60", "127.0.0.1"])
        .stdout(Stdio::null());
    let child = spawn_in_own_group(&mut cmd).expect("spawn cmd");
    let pid = child.id();
    let tracked = TrackedChild::in_registry(child, &registry);

    let mut grandchild = None;
    assert!(
        eventually(|| {
            grandchild = children_of(pid).first().copied();
            grandchild.is_some()
        }),
        "cmd never started ping"
    );
    let grandchild = grandchild.expect("found above");

    tracked.reap();
    assert!(
        eventually(|| !is_running(grandchild)),
        "reap must end the child's own child {grandchild}, not just the child"
    );
}

#[test]
fn reap_ends_what_the_child_left_running_after_it_exited() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "/b", "ping", "-n", "60", "127.0.0.1"])
        .stdout(Stdio::null());
    let child = spawn_in_own_group(&mut cmd).expect("spawn cmd");
    let pid = child.id();
    let tracked = TrackedChild::in_registry(child, &registry);

    let mut grandchild = None;
    assert!(
        eventually(|| {
            grandchild = children_of(pid).first().copied();
            grandchild.is_some()
        }),
        "cmd never started ping"
    );
    let grandchild = grandchild.expect("found above");
    tracked
        .wait_for_exit()
        .expect("cmd exits after starting ping");

    tracked.reap();
    assert!(
        eventually(|| !is_running(grandchild)),
        "reap must end {grandchild}, left running by a child that already exited"
    );
}

#[test]
fn a_child_started_suspended_runs_once_spawned() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "echo", "hi"]).stdout(Stdio::piped());
    let mut child = spawn_in_own_group(&mut cmd).expect("spawn cmd");
    let mut stdout = child.child_mut().stdout.take().expect("piped stdout");
    let tracked = TrackedChild::in_registry(child, &registry);

    let mut out = String::new();
    stdout.read_to_string(&mut out).expect("read stdout");
    assert_eq!(out.trim(), "hi");
    let status = tracked.wait_for_exit().expect("the wait succeeds");
    assert!(status.success(), "got {status:?}");
}

#[test]
fn wait_for_exit_reports_the_exit_code() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "exit", "3"]);
    let child = spawn_in_own_group(&mut cmd).expect("spawn cmd");
    let tracked = TrackedChild::in_registry(child, &registry);

    let status = tracked.wait_for_exit().expect("the wait succeeds");
    assert_eq!(status.code(), Some(3));
}
