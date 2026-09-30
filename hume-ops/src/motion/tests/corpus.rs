//! Every motion runs over the shared unicode corpus: a cursor on each
//! single-cluster sample between two ASCII chars.

use super::super::*;
use test_fixtures::assert_state;
use test_fixtures::unicode::{ALL, LONE_MARK};
use unicode_segmentation::UnicodeSegmentation;

fn single_cluster_samples() -> impl Iterator<Item = &'static str> {
    ALL.iter()
        .copied()
        .filter(|s| s.graphemes(true).count() == 1)
}

#[test]
fn move_right_steps_over_exactly_one_cluster() {
    for s in single_cluster_samples() {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
            &format!("x\n{s}-[b]>\n")
        );
    }
}

#[test]
fn move_left_steps_over_exactly_one_cluster() {
    for s in single_cluster_samples() {
        assert_state!(
            &format!("x\n{s}-[b]>\n"),
            |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
            &format!("x\n-[{s}]>b\n")
        );
    }
}

#[test]
fn extend_right_covers_the_cluster_and_the_next_char() {
    for s in single_cluster_samples() {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Extend),
            &format!("x\n-[{s}b]>\n")
        );
    }
}

#[test]
fn goto_line_start_lands_on_the_first_cluster() {
    for s in single_cluster_samples() {
        assert_state!(
            &format!("x\n{s}-[b]>\n"),
            |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
            &format!("x\n-[{s}]>b\n")
        );
    }
}

#[test]
fn goto_line_end_lands_on_the_start_of_the_last_cluster() {
    for s in single_cluster_samples().filter(|&s| s != LONE_MARK) {
        assert_state!(
            &format!("x\n-[b]>{s}\n"),
            |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
            &format!("x\nb-[{s}]>\n")
        );
    }
}

#[test]
fn select_line_covers_the_whole_line_and_its_newline() {
    for s in single_cluster_samples() {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| cmd_select_line(&text, sels, 1, MotionMode::Move),
            &format!("x\n-[{s}b\n]>")
        );
    }
}
