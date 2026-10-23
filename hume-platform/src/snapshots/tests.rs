use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::*;
use crate::wake::counting_wake;

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
    assert_eq!(
        snapshots.try_take(),
        Some(Progress {
            value: 2,
            done: false
        })
    );
    assert_eq!(snapshots.try_take(), None);
    release.send(()).expect("producer waits");
    wait_wakes(&woken, 1);
    assert_eq!(
        snapshots.try_take(),
        Some(Progress {
            value: 2,
            done: true
        })
    );
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
    assert_eq!(
        snapshots.try_take(),
        Some(Progress {
            value: 3,
            done: true
        })
    );
    assert_eq!(woken.try_iter().count(), 0);
}

#[test]
fn a_producer_that_reports_nothing_ends_with_the_default() {
    let (wake, woken) = counting_wake();
    let mut snapshots =
        spawn_snapshots::<u32, _>("hume-test-snapshots", wake, |_| {}).expect("spawn");
    wait_wakes(&woken, 1);
    assert_eq!(
        snapshots.try_take(),
        Some(Progress {
            value: 0,
            done: true
        })
    );
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
    assert_eq!(
        snapshots.try_take(),
        Some(Progress {
            value: 5,
            done: true
        })
    );
}

#[test]
fn send_returns_false_once_the_consumer_is_dropped() {
    let (wake, woken) = counting_wake();
    let (release, gate) = mpsc::channel::<()>();
    let (report_sent, sent) = mpsc::channel();
    let snapshots = spawn_snapshots("hume-test-snapshots", wake, move |reporter| {
        let _ = gate.recv();
        let _ = report_sent.send(reporter.send(1));
    })
    .expect("spawn");
    drop(snapshots);
    release.send(()).expect("producer waits");
    assert_eq!(sent.recv_timeout(Duration::from_secs(1)), Ok(false));
    wait_wakes(&woken, 1);
}
