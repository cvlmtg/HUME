//! The run loop's frame pacing: a frame draws at once after input with no
//! more input queued behind it, and otherwise no sooner than `FRAME_GAP`
//! after the last one.

use std::time::{Duration, Instant};

use crate::editor::frame::{FRAME_GAP, FramePacer, InputBacklog};

#[test]
fn the_first_frame_is_due_at_once() {
    assert_eq!(FramePacer::default().wait(Instant::now()), None);
}

#[test]
fn a_frame_after_input_is_due_at_once() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.drew(drawn);
    pacer.input(InputBacklog::Empty);
    assert_eq!(pacer.wait(drawn + Duration::from_millis(1)), None);
}

#[test]
fn a_wake_within_the_gap_waits_out_the_rest_of_it() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.drew(drawn);
    let woken = drawn + FRAME_GAP / 4;
    assert_eq!(pacer.wait(woken), Some(FRAME_GAP - FRAME_GAP / 4));
}

#[test]
fn a_wake_once_the_gap_has_passed_is_due_at_once() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.drew(drawn);
    assert_eq!(pacer.wait(drawn + FRAME_GAP), None);
}

#[test]
fn drawing_spends_the_input() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.input(InputBacklog::Empty);
    pacer.drew(drawn);
    assert_eq!(pacer.wait(drawn), Some(FRAME_GAP));
}

#[test]
fn queued_input_within_the_gap_waits_out_the_rest_of_it() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.drew(drawn);
    pacer.input(InputBacklog::Queued);
    let read = drawn + FRAME_GAP / 4;
    assert_eq!(pacer.wait(read), Some(FRAME_GAP - FRAME_GAP / 4));
}

#[test]
fn queued_input_once_the_gap_has_passed_is_due_at_once() {
    let drawn = Instant::now();
    let mut pacer = FramePacer::default();
    pacer.drew(drawn);
    pacer.input(InputBacklog::Queued);
    assert_eq!(pacer.wait(drawn + FRAME_GAP), None);
}
