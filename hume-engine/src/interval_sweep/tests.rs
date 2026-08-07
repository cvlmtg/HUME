use super::*;
use std::cmp::Reverse;

fn run<R: Ord + Copy>(raw: Vec<(usize, usize, R, u16)>) -> Vec<(usize, usize, u16)> {
    run_tb(raw, TieBreak::LastPushed)
}

fn run_tb<R: Ord + Copy>(
    raw: Vec<(usize, usize, R, u16)>,
    tie_break: TieBreak,
) -> Vec<(usize, usize, u16)> {
    let mut raw = raw;
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut out = Vec::new();
    flatten_overlapping_spans(&mut raw, &mut stack, &mut events, &mut out, tie_break);
    out
}

// ── Depth-ranked (hume-treesitter's convention: higher rank wins as-is) ──

#[test]
fn inner_wins_non_shared_start() {
    let got = run(vec![(0, 8, 0u8, 0), (5, 7, 1, 1)]);
    assert_eq!(got, vec![(0, 5, 0), (5, 7, 1), (7, 8, 0)]);
}

#[test]
fn three_level_nesting() {
    let got = run(vec![(0, 20, 0u8, 0), (5, 15, 1, 1), (8, 10, 2, 2)]);
    assert_eq!(
        got,
        vec![(0, 5, 0), (5, 8, 1), (8, 10, 2), (10, 15, 1), (15, 20, 0)]
    );
}

#[test]
fn deeper_layer_wins_regardless_of_collection_order() {
    // Collected out of depth order — the deeper span must still win.
    let got = run(vec![(3, 7, 2u8, 2), (0, 10, 0, 0)]);
    assert_eq!(got, vec![(0, 3, 0), (3, 7, 2), (7, 10, 0)]);
}

#[test]
fn adjacent_same_scope_segments_merge() {
    let got = run(vec![(0, 2, 0u8, 0), (2, 5, 0, 0), (5, 9, 0, 0)]);
    assert_eq!(got, vec![(0, 9, 0)]);
}

// ── Priority-ranked via `Reverse` (hume-editor's convention: lower priority
// number wins, achieved with zero inversion arithmetic at the call site) ──

#[test]
fn reverse_wrapped_rank_makes_the_lower_priority_number_win() {
    // Priority 0 (highest severity) must beat priority 5, even though 0 <
    // 5 as plain integers — `Reverse` is what flips this.
    let got = run(vec![(0, 10, Reverse(5u8), 0), (2, 6, Reverse(0u8), 1)]);
    assert_eq!(got, vec![(0, 2, 0), (2, 6, 1), (6, 10, 0)]);
}

#[test]
fn equal_priority_ties_keep_first_pushed_under_that_tie_break() {
    let got = run_tb(
        vec![(0, 5, Reverse(1u8), 0), (0, 5, Reverse(1u8), 1)],
        TieBreak::FirstPushed,
    );
    assert_eq!(got, vec![(0, 5, 0)]);
}

#[test]
fn equal_rank_ties_keep_last_pushed_under_that_tie_break() {
    // Same input, opposite `tie_break` — the other span must win instead,
    // proving the flag actually controls the outcome rather than being
    // ignored.
    let got = run_tb(
        vec![(0, 5, Reverse(1u8), 0), (0, 5, Reverse(1u8), 1)],
        TieBreak::LastPushed,
    );
    assert_eq!(got, vec![(0, 5, 1)]);
}

#[test]
fn disjoint_spans_leave_the_gap_unemitted() {
    let got = run(vec![(0, 3, 0u8, 0), (5, 8, 0, 1)]);
    assert_eq!(got, vec![(0, 3, 0), (5, 8, 1)]);
}

#[test]
fn scratch_reuse_across_calls_leaves_no_stale_events() {
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut out = Vec::new();

    let mut first: Vec<(usize, usize, u8, u16)> = vec![(0, 3, 0, 0)];
    flatten_overlapping_spans(
        &mut first,
        &mut stack,
        &mut events,
        &mut out,
        TieBreak::LastPushed,
    );
    assert_eq!(out, vec![(0, 3, 0)]);

    out.clear();
    let mut second: Vec<(usize, usize, u8, u16)> = vec![(10, 12, 0, 1)];
    flatten_overlapping_spans(
        &mut second,
        &mut stack,
        &mut events,
        &mut out,
        TieBreak::LastPushed,
    );
    assert_eq!(out, vec![(10, 12, 1)]);
}
