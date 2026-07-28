//! Unix-only tests, gated once at the `mod unix;` declaration in the
//! parent — mirrors `process/tests.rs`'s own split.
//!
//! Every test below builds its own [`ChildRegistry`] rather than touching
//! the crate's real global: `cargo test` runs this crate's tests as threads
//! in one process, so a shared registry would let one test's `kill_all()`
//! reach a completely different, concurrently-running test's child.

use super::super::*;
use crate::process::spawn_in_own_group;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn is_alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}

/// SIGKILLs and reaps a child this test spawned but never routed through a
/// `Child` the test still holds — `kill_tracked_children`'s own reap path
/// deliberately never waits (see `kill_slot`'s doc), so a child it kills is
/// left a zombie until *something* calls `waitpid` on it; in production
/// that's the OS once the whole process exits, but this test process keeps
/// running afterward, so it must reap explicitly or leak a zombie into the
/// rest of the suite.
fn kill_and_reap(pid: i32) {
    let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    let _ = waitpid(Pid::from_raw(pid), None);
}

/// `kill_all` doesn't `wait()` its victims (force-exit must never block on
/// one) — SIGKILL delivery is asynchronous, so death is polled rather than
/// asserted immediately after the call.
fn wait_until_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn registering_a_child_makes_it_visible_in_the_registry() {
    let registry = ChildRegistry::new();
    assert_eq!(registry.live_count(), 0);

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = child.id() as i32;
    let tracked = TrackedChild::in_registry(child, &registry);

    assert_eq!(registry.live_count(), 1);
    tracked.reap();
    assert!(wait_until_dead(pid, Duration::from_secs(2)));
}

#[test]
fn dropping_the_only_handle_removes_it_from_the_registry() {
    let registry = ChildRegistry::new();

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = child.id() as i32;
    let tracked = TrackedChild::in_registry(child, &registry);
    assert_eq!(registry.live_count(), 1);

    // Nothing reaped it — dropping the handle alone must not leak the
    // process, so kill it directly for the assertion and to clean up.
    drop(tracked);
    assert_eq!(
        registry.live_count(),
        0,
        "a dropped handle's Weak must no longer count as live"
    );

    kill_and_reap(pid);
    assert!(wait_until_dead(pid, Duration::from_secs(2)));
}

#[test]
fn dead_entries_are_pruned_on_the_next_registration() {
    let registry = ChildRegistry::new();

    let mut first_cmd = Command::new("true");
    let first = spawn_in_own_group(&mut first_cmd).expect("spawn true");
    let first_tracked = TrackedChild::in_registry(first, &registry);
    first_tracked.reap();
    drop(first_tracked);

    let mut second_cmd = Command::new("sleep");
    second_cmd.arg("30");
    let second = spawn_in_own_group(&mut second_cmd).expect("spawn sleep");
    let second_pid = second.id() as i32;
    let second_tracked = TrackedChild::in_registry(second, &registry);

    assert_eq!(
        registry.live_count(),
        1,
        "registering the second child must prune the first's dead Weak, not accumulate it"
    );

    second_tracked.reap();
    assert!(wait_until_dead(second_pid, Duration::from_secs(2)));
}

#[test]
fn kill_all_kills_the_whole_process_group() {
    let registry = ChildRegistry::new();

    // The direct child forks a grandchild and prints its pid before both
    // sleep; a direct (non-group) kill of just the tracked pid would leave
    // the grandchild alive, which is exactly the gap this test exists to
    // catch. The background job inherits the shell's stdout, so the pipe
    // only reaches EOF once every group member exits — `read_line` instead
    // of reading to EOF, so this doesn't block on that for up to 30s.
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 30 & echo $!; exec sleep 30"])
        .stdout(std::process::Stdio::piped());
    let mut child = spawn_in_own_group(&mut cmd).expect("spawn sh");
    let direct_pid = child.id() as i32;

    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stdout), &mut line)
        .expect("read grandchild pid line");
    let grandchild_pid: i32 = line.trim().parse().expect("grandchild pid printed");

    let tracked = TrackedChild::in_registry(child, &registry);

    registry.kill_all();

    // `kill_all` deliberately never waits (force-exit must never block), so
    // the direct child — parented to *this* test process — is confirmed and
    // reaped the same way `kill_and_reap` does. The grandchild's parent is
    // the direct child, not us, so `waitpid` isn't ours to call on it; once
    // its parent exits it's reparented to launchd/init, which reaps it —
    // `wait_until_dead`'s `kill(pid, 0)` poll genuinely observes that.
    let status = waitpid(Pid::from_raw(direct_pid), None).expect("reap direct child");
    assert!(
        matches!(
            status,
            nix::sys::wait::WaitStatus::Signaled(_, Signal::SIGKILL, _)
        ),
        "kill_all must kill the tracked child itself, got {status:?}"
    );
    assert!(
        wait_until_dead(grandchild_pid, Duration::from_secs(2)),
        "kill_all must reach the tracked child's own children via the group kill"
    );
    drop(tracked);
}

#[test]
fn kill_all_does_not_reach_a_dropped_and_deregistered_child() {
    let registry = ChildRegistry::new();

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = child.id() as i32;
    let tracked = TrackedChild::in_registry(child, &registry);
    drop(tracked);

    registry.kill_all();

    // Give a misbehaving kill_all a moment to (wrongly) act, then prove the
    // child is still alive before cleaning it up ourselves.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        is_alive(pid),
        "kill_all must not reach a child whose handle was already dropped"
    );
    kill_and_reap(pid);
    assert!(wait_until_dead(pid, Duration::from_secs(2)));
}

#[test]
fn try_wait_reflects_a_child_that_already_exited() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("true");
    let child = spawn_in_own_group(&mut cmd).expect("spawn true");
    let tracked = TrackedChild::in_registry(child, &registry);

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut status = None;
    while Instant::now() < deadline {
        if let Ok(Some(s)) = tracked.try_wait() {
            status = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        status.is_some_and(|s| s.success()),
        "try_wait must observe a child that exited on its own"
    );
}

#[test]
fn reap_kills_the_whole_process_group() {
    let registry = ChildRegistry::new();

    // Same fixture as `kill_all_kills_the_whole_process_group`: a direct
    // child that forks a grandchild and prints its pid before both sleep.
    // This exercises `TrackedChild::reap` — the *normal-exit* `Drop` path —
    // rather than `kill_all`'s force-exit path, so it catches a regression
    // where only `kill_all` was taught to signal the whole group.
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 30 & echo $!; exec sleep 30"])
        .stdout(std::process::Stdio::piped());
    let mut child = spawn_in_own_group(&mut cmd).expect("spawn sh");

    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stdout), &mut line)
        .expect("read grandchild pid line");
    let grandchild_pid: i32 = line.trim().parse().expect("grandchild pid printed");

    let tracked = TrackedChild::in_registry(child, &registry);

    let status = tracked.reap().expect("reap must return an exit status");
    assert!(
        matches!(
            status.signal(),
            Some(sig) if sig == Signal::SIGKILL as i32
        ),
        "reap must kill the tracked child itself, got {status:?}"
    );
    assert!(
        wait_until_dead(grandchild_pid, Duration::from_secs(2)),
        "reap must reach the tracked child's own children via the group kill, \
         not just the direct pid"
    );
}

#[test]
fn kill_all_waits_for_a_contended_table_lock() {
    let registry = Arc::new(ChildRegistry::new());

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = child.id() as i32;
    let tracked = TrackedChild::in_registry(child, &registry);

    // Hold the table lock ourselves first, standing in for `register()`
    // running on another thread — `kill_all` must wait for it rather than
    // bailing out and killing nothing.
    let table_guard = registry.entries.lock().unwrap();

    let killer_registry = Arc::clone(&registry);
    let killer = std::thread::spawn(move || killer_registry.kill_all());

    // Give a misbehaving `kill_all` a chance to (wrongly) return early and
    // skip the child while the lock is still held.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        is_alive(pid),
        "kill_all must not have run yet: the table lock is still held"
    );

    drop(table_guard);
    killer.join().expect("kill_all thread must not panic");

    // `kill_all` deliberately never waits (force-exit must never block), so
    // the child — parented to *this* test process — is a zombie until
    // reaped here, the same way `kill_and_reap`/`kill_all_kills_the_whole_process_group`
    // do; a plain `kill(pid, 0)` liveness poll would see the zombie as still
    // "alive" and this assertion would never fail even on a broken `kill_all`.
    let status = waitpid(Pid::from_raw(pid), None).expect("reap the child");
    assert!(
        matches!(
            status,
            nix::sys::wait::WaitStatus::Signaled(_, Signal::SIGKILL, _)
        ),
        "kill_all must reach the child once the table lock is released, \
         not skip it because the lock was momentarily contended: got {status:?}"
    );
    drop(tracked);
}

#[test]
fn kill_all_tolerates_an_already_reaped_child() {
    let registry = ChildRegistry::new();
    let mut cmd = Command::new("true");
    let child = spawn_in_own_group(&mut cmd).expect("spawn true");
    let tracked = TrackedChild::in_registry(child, &registry);

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && !matches!(tracked.try_wait(), Ok(Some(_))) {
        std::thread::sleep(Duration::from_millis(10));
    }

    // Must return promptly without panicking — the regression this guards
    // against is `kill_all` signalling a pid `Child` has already reaped
    // (and the kernel may have since recycled) instead of consulting the
    // cached exit status first.
    let started = Instant::now();
    registry.kill_all();
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn kill_all_retries_a_slot_contended_by_a_non_killing_reader() {
    let registry = ChildRegistry::new();

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let child = spawn_in_own_group(&mut cmd).expect("spawn sleep");
    let pid = child.id() as i32;
    let tracked = TrackedChild::in_registry(child, &registry);

    // Stand in for a concurrent `id()`/`try_wait()` call, which holds the
    // same slot mutex without killing anything — unlike a concurrent
    // `reap()`, there is no one else here to signal the child, so `kill_all`
    // giving up on the first contended `try_lock` (the pre-retry behaviour)
    // would leave it alive. A channel handshake (not a blind sleep before
    // calling `kill_all`) guarantees the lock is actually held by the time
    // `kill_all` makes its first attempt — the race this test relies on is
    // real time elapsing during `kill_all`'s own retries, not in the setup.
    let (acquired_tx, acquired_rx) = mpsc::channel::<()>();
    let slot = Arc::clone(&tracked.0);
    let holder = std::thread::spawn(move || {
        let _guard = slot.lock().unwrap();
        let _ = acquired_tx.send(());
        std::thread::sleep(Duration::from_millis(1));
    });
    acquired_rx
        .recv()
        .expect("holder thread must signal once it holds the lock");

    registry.kill_all();
    holder.join().expect("holder thread must not panic");

    // `kill_all` deliberately never `wait()`s its victims (see its own doc),
    // so a killed-but-unreaped child is still a zombie — `is_alive`'s
    // signal-0 probe would report it "alive" either way, telling a retried
    // kill apart from an abandoned child requires reaping it ourselves, same
    // as `kill_all_waits_for_a_contended_table_lock` above.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut status = None;
    while Instant::now() < deadline {
        match waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => std::thread::sleep(Duration::from_millis(10)),
            Ok(s) => {
                status = Some(s);
                break;
            }
            Err(_) => break,
        }
    }
    assert!(
        matches!(status, Some(WaitStatus::Signaled(_, Signal::SIGKILL, _))),
        "kill_all must retry a slot contended by a non-killing reader and reach \
         the child once it's released, not abandon it on the first failed \
         try_lock: got {status:?}"
    );
}
