use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::*;
use crate::wake::counting_wake;

fn progress<T>(value: T, done: bool) -> Progress<T> {
    Progress { value, done }
}

/// Waits for `n` wakes, failing the test after a second without one.
fn wait_wakes(woken: &Receiver<()>, n: usize) {
    for _ in 0..n {
        woken
            .recv_timeout(Duration::from_secs(1))
            .expect("the producer wakes");
    }
}

#[test]
fn try_take_reads_only_the_latest_snapshot() {
    let (wake, woken) = counting_wake();
    let (release, gate) = mpsc::channel::<()>();
    let mut snapshots = spawn_snapshots("hume-test-snapshots", wake, move |reporter| {
        reporter.send(1);
        reporter.send(2);
        let _ = gate.recv();
    })
    .expect("spawn");
    wait_wakes(&woken, 2);
    assert_eq!(snapshots.try_take(), Some(progress(2, false)));
    assert_eq!(snapshots.try_take(), None);
    release.send(()).expect("producer waits");
    wait_wakes(&woken, 1);
    assert_eq!(snapshots.try_take(), Some(progress(2, true)));
    assert_eq!(snapshots.try_take(), None);
}

#[test]
fn each_snapshot_and_the_end_wake_the_consumer() {
    let (wake, woken) = counting_wake();
    let mut snapshots = spawn_snapshots("hume-test-snapshots", wake, |reporter| {
        for n in 1..=3 {
            reporter.send(n);
        }
    })
    .expect("spawn");
    wait_wakes(&woken, 4);
    assert_eq!(snapshots.try_take(), Some(progress(3, true)));
    assert_eq!(woken.try_iter().count(), 0);
}

#[test]
fn a_producer_that_reports_nothing_ends_with_the_default() {
    let (wake, woken) = counting_wake();
    let mut snapshots =
        spawn_snapshots::<u32, _>("hume-test-snapshots", wake, |_| {}).expect("spawn");
    wait_wakes(&woken, 1);
    assert_eq!(snapshots.try_take(), Some(progress(0, true)));
}

#[test]
fn a_panicking_producer_ends_with_its_last_snapshot() {
    let (wake, woken) = counting_wake();
    let mut snapshots = spawn_snapshots("hume-test-snapshots", wake, |reporter| {
        reporter.send(5);
        panic!("simulated producer crash");
    })
    .expect("spawn");
    wait_wakes(&woken, 2);
    assert_eq!(snapshots.try_take(), Some(progress(5, true)));
}

#[test]
fn a_reporter_is_live_until_its_consumer_is_dropped() {
    let (wake, woken) = counting_wake();
    let (release, gate) = mpsc::channel::<()>();
    let (liveness_tx, liveness) = mpsc::channel();
    let snapshots = spawn_snapshots::<u32, _>("hume-test-snapshots", wake, move |reporter| {
        let _ = liveness_tx.send(reporter.is_live());
        let _ = gate.recv();
        let _ = liveness_tx.send(reporter.is_live());
    })
    .expect("spawn");
    assert_eq!(liveness.recv_timeout(Duration::from_secs(1)), Ok(true));
    drop(snapshots);
    release.send(()).expect("producer waits");
    assert_eq!(liveness.recv_timeout(Duration::from_secs(1)), Ok(false));
    wait_wakes(&woken, 1);
}
