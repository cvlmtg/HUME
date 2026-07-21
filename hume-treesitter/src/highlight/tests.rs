use super::*;

fn s(n: u16) -> ScopeId {
    ScopeId(n)
}

/// Run `flatten_overlaps` with every interval at depth 0 (single-layer,
/// the pre-injection behavior) — `d()` below builds multi-depth input
/// for the depth-priority tests.
fn run(raw: Vec<(usize, usize, ScopeId)>) -> Vec<(usize, usize, ScopeId)> {
    run_d(raw.into_iter().map(|(a, b, c)| (a, b, c, 0)).collect())
}

fn run_d(mut raw: Vec<(usize, usize, ScopeId, u8)>) -> Vec<(usize, usize, ScopeId)> {
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut out = Vec::new();
    flatten_overlaps(&mut raw, &mut stack, &mut events, &mut out);
    out
}

#[test]
fn inner_wins_non_shared_start() {
    // Regression: outer @string [0,8), inner @string.escape [5,7).
    // Old sweep dropped the inner; stack flattener emits it correctly.
    let got = run(vec![(0, 8, s(0)), (5, 7, s(1))]);
    assert_eq!(got, vec![(0, 5, s(0)), (5, 7, s(1)), (7, 8, s(0))]);
}

#[test]
fn inner_wins_shared_start() {
    // Shared start: outer [0,10), inner [0,4) — inner wins its region.
    let got = run(vec![(0, 10, s(0)), (0, 4, s(1))]);
    assert_eq!(got, vec![(0, 4, s(1)), (4, 10, s(0))]);
}

#[test]
fn disjoint_gap_preserved() {
    // Two disjoint intervals; uncovered gap has no output.
    let got = run(vec![(0, 3, s(0)), (5, 8, s(1))]);
    assert_eq!(got, vec![(0, 3, s(0)), (5, 8, s(1))]);
}

#[test]
fn three_level_nesting() {
    // [0,20) A, [5,15) B, [8,10) C — each level wins its region.
    let got = run(vec![(0, 20, s(0)), (5, 15, s(1)), (8, 10, s(2))]);
    assert_eq!(
        got,
        vec![
            (0, 5, s(0)),
            (5, 8, s(1)),
            (8, 10, s(2)),
            (10, 15, s(1)),
            (15, 20, s(0)),
        ]
    );
}

#[test]
fn partial_overlap_last_started_wins() {
    // A=[0,5), B=[3,8): overlapping, not nested.  In the overlap region
    // [3,5) B started last so it wins; B continues alone in [5,8).
    let got = run(vec![(0, 5, s(0)), (3, 8, s(1))]);
    assert_eq!(got, vec![(0, 3, s(0)), (3, 8, s(1))]);
}

#[test]
fn identical_ranges_last_pushed_wins() {
    // Two captures of the same byte range: last-opened (s(1)) wins entirely.
    let got = run(vec![(0, 5, s(0)), (0, 5, s(1))]);
    assert_eq!(got, vec![(0, 5, s(1))]);
}

#[test]
fn same_start_three_way_tie_resolves_by_seq() {
    // Three captures share a start byte with distinct ends and distinct
    // scopes. The highest-seq (last-captured) interval must win at byte
    // 0; as it and the middle one end, each remaining interval takes over
    // in seq order. `sort_unstable_by` does not guarantee tie order by
    // API contract — the explicit `seq` field makes this deterministic
    // rather than an accident of the current sort implementation.
    let got = run(vec![(0, 5, s(0)), (0, 8, s(1)), (0, 6, s(2))]);
    assert_eq!(got, vec![(0, 6, s(2)), (6, 8, s(1))]);
}

#[test]
fn scratch_reuse_across_calls_leaves_no_stale_events() {
    // Regression for O1: `events`/`stack` are caller-owned scratch reused
    // across calls (mirroring TsState in the real highlighter). A second,
    // disjoint call through the same scratch must not see the first
    // call's intervals leak through.
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut out = Vec::new();

    let mut first = vec![(0, 3, s(0), 0)];
    flatten_overlaps(&mut first, &mut stack, &mut events, &mut out);
    assert_eq!(out, vec![(0, 3, s(0))]);

    out.clear();
    let mut second = vec![(10, 12, s(1), 0)];
    flatten_overlaps(&mut second, &mut stack, &mut events, &mut out);
    assert_eq!(out, vec![(10, 12, s(1))]);
}

#[test]
fn three_segment_chain_merges_into_one() {
    // Same scope, three adjacent segments: [0,2)+[2,5)+[5,9) → one [0,9).
    // Exercises the dedup_by merge pass over more than one join.
    let got = run(vec![(0, 2, s(0)), (2, 5, s(0)), (5, 9, s(0))]);
    assert_eq!(got, vec![(0, 9, s(0))]);
}

// ── Depth priority (multi-layer / injections) ────────────────────────────

#[test]
fn deeper_layer_wins_regardless_of_collection_order() {
    // A depth-0 (root) span [0,10) fully contains a depth-2 (nested
    // injection) span [3,7) — collected in REVERSE depth order (the
    // depth-2 entry pushed into `raw` before the depth-0 one), simulating
    // a nested injection gathered ahead of its shallower parent. The
    // deeper span must still win its region regardless of collection order.
    let got = run_d(vec![(3, 7, s(2), 2), (0, 10, s(0), 0)]);
    assert_eq!(got, vec![(0, 3, s(0)), (3, 7, s(2)), (7, 10, s(0))]);
}

#[test]
fn shallower_layer_started_later_still_loses_to_deeper() {
    // Depth-1 span [0,10) started AFTER (higher seq) a depth-3 span
    // [2,5) in collection order — plain seq-order priority (pre-depth-
    // aware flattening) would have picked the depth-1 span at [2,5)
    // since it has the higher seq; depth must win instead.
    let got = run_d(vec![(2, 5, s(3), 3), (0, 10, s(1), 1)]);
    assert_eq!(got, vec![(0, 2, s(1)), (2, 5, s(3)), (5, 10, s(1))]);
}
