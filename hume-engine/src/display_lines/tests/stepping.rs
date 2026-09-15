//! Display-line stepping tests (`next`/`prev`/`advance`/`distance`/`max_scroll_top`).

use super::*;
use crate::pane::ViewGeometry;

/// [`ViewGeometry`] for a `height`-row viewport and `margin` scrolloff —
/// `max_scroll_top` can no longer be called at `height == 0` (there is no
/// `ViewGeometry` to construct one from), so every fixture here is implicitly
/// nonzero-height.
fn geo(height: u16, margin: usize) -> ViewGeometry {
    crate::pane::Viewport::new(0, height)
        .geometry(margin)
        .expect("test fixtures use a nonzero height")
}

/// Walk the whole document forward from its first display line.
fn walk_forward(dlm: &mut DisplayLineMap<'_>) -> Vec<DisplayLinePos> {
    let mut positions = vec![DisplayLinePos::default()];
    while let Some(next) = dlm.next(*positions.last().expect("seeded above")) {
        positions.push(next);
    }
    positions
}

// ---------------------------------------------------------------------------
// Stepping
// ---------------------------------------------------------------------------

/// "a\nb\nc\n" with 2 Before display lines on line 1 and 1 After display line on line 2. Hand-
/// written display line list, in document order:
///
/// ```text
///   (0,0)  line 0 content
///   (1,0)  line 1 Before #0
///   (1,1)  line 1 Before #1
///   (1,2)  line 1 content
///   (2,0)  line 2 content
///   (2,1)  line 2 After #0
/// ```
fn three_line_doc() -> (Rope, ProviderSet, Vec<DisplayLinePos>) {
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
    )));
    let expected = vec![
        DisplayLinePos::new(ContentLine::new(0), 0),
        DisplayLinePos::new(ContentLine::new(1), 0),
        DisplayLinePos::new(ContentLine::new(1), 1),
        DisplayLinePos::new(ContentLine::new(1), 2),
        DisplayLinePos::new(ContentLine::new(2), 0),
        DisplayLinePos::new(ContentLine::new(2), 1),
    ];
    (rope, providers, expected)
}

#[test]
fn next_walks_the_documents_display_lines_in_order() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(walk_forward(&mut dlm), expected);
}

#[test]
fn prev_is_the_inverse_of_next() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let mut backward = vec![*expected.last().expect("non-empty")];
    while let Some(prev) = dlm.prev(*backward.last().expect("seeded above")) {
        backward.push(prev);
    }
    backward.reverse();
    assert_eq!(backward, expected);
}

#[test]
fn next_and_prev_stop_exactly_at_the_documents_edges() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.prev(first),
        None,
        "no display line precedes the document's first display line"
    );
    assert_eq!(
        dlm.next(last),
        None,
        "no display line follows the document's last display line"
    );
}

#[test]
fn advance_saturating_matches_repeated_stepping_and_saturates_at_both_ends() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            assert_eq!(
                dlm.advance_saturating(from, delta),
                to,
                "advance_saturating({from:?}, {delta}) should reach {to:?}"
            );
        }
    }

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.advance_saturating(first, -10),
        first,
        "saturates at the first display line"
    );
    assert_eq!(
        dlm.advance_saturating(last, 10),
        last,
        "saturates at the last display line"
    );
}

#[test]
fn advance_counted_saturating_reports_how_far_it_actually_stepped() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    // In bounds: the count matches the requested delta exactly.
    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            let (pos, taken) = dlm.advance_counted_saturating(from, delta);
            assert_eq!(
                pos, to,
                "advance_counted_saturating({from:?}, {delta}) position"
            );
            assert_eq!(
                taken,
                delta.unsigned_abs(),
                "advance_counted_saturating({from:?}, {delta}) count"
            );
        }
    }

    // Past either edge: the document stops the walk short, so the count is
    // less than what was asked for.
    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.advance_counted_saturating(first, -10),
        (first, 0),
        "no display lines exist before the first display line"
    );
    assert_eq!(
        dlm.advance_counted_saturating(last, 10),
        (last, 0),
        "no display lines exist past the last display line"
    );

    // The documented "count is also the distance back" property
    // `scroll::scroll_back_from` relies on: having stepped `n` display lines backward
    // from `pos`, `distance` from the landing spot back to `pos` is that
    // same `n`.
    let pos = expected[4];
    let (landed, taken) = dlm.advance_counted_saturating(pos, -3);
    assert_eq!(dlm.distance(landed, pos, expected.len()), Some(taken));
}

#[test]
fn distance_counts_display_lines_forward_and_rejects_backward_or_distant() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate().skip(i) {
            assert_eq!(
                dlm.distance(from, to, 16),
                Some(j - i),
                "distance({from:?} → {to:?})"
            );
        }
    }

    assert_eq!(
        dlm.distance(expected[3], expected[1], 16),
        None,
        "a display line behind the start is not reachable forward"
    );
    assert_eq!(
        dlm.distance(expected[0], expected[5], 2),
        None,
        "5 display lines away is beyond a cap of 2"
    );
    assert_eq!(
        dlm.distance(expected[0], expected[2], 2),
        Some(2),
        "exactly at the cap still resolves"
    );
}

#[test]
fn max_scroll_top_counts_virtual_lines_toward_the_height() {
    // The hand-written list above is 6 display lines: 3 content + 3 virtual.
    let (rope, providers, expected) = three_line_doc();
    assert_eq!(expected.len(), 6);
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.max_scroll_top(geo(6, 0)),
        expected[0],
        "6 display lines fit in a 6-line viewport — top can't move"
    );
    assert_eq!(
        dlm.max_scroll_top(geo(5, 0)),
        expected[1],
        "one display line short: top may scroll past the first display line, no further"
    );
    assert_eq!(
        dlm.max_scroll_top(geo(3, 0)),
        expected[3],
        "counting content display lines alone would wrongly stop 3 short of the end"
    );
}

#[test]
fn max_scroll_top_margin_reserves_lookahead_rows_past_the_last_display_line() {
    // Same 6-display-line fixture. A margin > 0 pulls the bound back that
    // many display lines from the last one — matching
    // `Viewport::reveal`'s own bottom-margin arithmetic, so the two agree on
    // where "all the way down" is.
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.max_scroll_top(geo(6, 1)),
        expected[1],
        "height=6, margin=1: last display line lands 1 row above the bottom"
    );
    assert_eq!(
        dlm.max_scroll_top(geo(4, 1)),
        expected[3],
        "height=4, margin=1: same one-row reservation at a shorter viewport"
    );
}

#[test]
fn max_scroll_top_margin_is_capped_the_same_way_reveal_caps_it() {
    // A margin at or above half the viewport height must not swallow the
    // whole viewport — clamped to `(height - 1) / 2`, identically to
    // `Viewport::reveal`'s own margin. An uncapped margin=10 here behaves
    // exactly like the clamped margin=1 case.
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.max_scroll_top(geo(4, 10)),
        dlm.max_scroll_top(geo(4, 1))
    );
    assert_eq!(dlm.max_scroll_top(geo(4, 10)), expected[3]);
}

#[test]
fn zero_height_viewport_has_no_geometry_to_scroll_with() {
    // A zero-height viewport has no room to scroll into at all — encoded as
    // `Viewport::geometry` returning `None`, so `max_scroll_top` (and every
    // other scroll verb) can never be called at `height == 0` in the first
    // place; there is no `ViewGeometry` to construct one from.
    assert!(crate::pane::Viewport::new(0, 0).geometry(0).is_none());
}

#[test]
fn degenerate_single_empty_line_document() {
    // A bare "\n" is the smallest possible buffer under the invariant: one
    // real (empty) line plus the structural trailing newline.
    let rope = Rope::from_str("\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.last_line(), ContentLine::new(0));
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(99), 99)),
        DisplayLinePos::new(ContentLine::new(0), 0),
        "the only display line in a one-line document is (0, 0)"
    );
    assert_eq!(
        dlm.max_scroll_top(geo(1, 0)),
        DisplayLinePos::default(),
        "the document's single display line fits a height of 1 — top can't move"
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "content_width >= 1")]
fn new_panics_on_zero_content_width() {
    let rope = Rope::from_str("x\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    DisplayLineMap::new(
        &rope,
        &providers,
        0,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut s,
    );
}

#[test]
fn wrap_width_one_emits_one_grapheme_per_display_line_without_hanging() {
    // One grapheme per display line at wrap width 1, plus a 5th display line for the trailing
    // '\n's own sentinel: every display line at width 1 exactly fills, so the
    // sentinel always wraps onto a display line of its own here.
    let rope = Rope::from_str("abcd\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = DisplayLineMap::new(
        &rope,
        &providers,
        80,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::Soft { width: 1 },
            tab_width: 4,
            whitespace: ws(),
        },
        &mut s,
    );

    let breakdown = dlm.block(ContentLine::new(0));
    assert_eq!(breakdown.content, 5);
}
