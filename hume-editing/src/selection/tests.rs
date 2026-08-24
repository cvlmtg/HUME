use super::*;
use crate::changeset::ChangeSetBuilder;
use pretty_assertions::assert_eq;

/// Test-only shorthand: these tests exercise merge/translate invalidation,
/// not `DisplayColOrigin` itself, so every latch below is `BufferLine`
/// arbitrarily — origin-aware behaviour is pinned separately in
/// `hume-ops`/`hume-editor`.
fn sticky(display_col: u32) -> StickyDisplayCol {
    StickyDisplayCol {
        display_col,
        origin: DisplayColOrigin::BufferLine,
        wrap_width: None,
    }
}

// ── SelectionSet ──────────────────────────────────────────────────────────

#[test]
fn single_selection_is_primary() {
    let s = Selection::collapsed(0);
    let set = SelectionSet::single(s);
    assert_eq!(set.primary(), s);
    assert_eq!(set.len(), 1);
}

#[test]
fn merge_no_overlap() {
    // Two disjoint selections — should stay separate.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(0, 3), Selection::new(5, 8)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 2);
}

#[test]
fn merge_overlapping_selections() {
    // (anchor=0,head=5) and (anchor=3,head=8) overlap — should merge.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(0, 5), Selection::new(3, 8)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    assert_eq!(set.primary().start(), 0);
    assert_eq!(set.primary().end(), 8);
}

#[test]
fn merge_adjacent_selections() {
    // (anchor=0,head=3) and (anchor=3,head=6) touch at offset 3 — should merge.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(0, 3), Selection::new(3, 6)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    assert_eq!(set.primary().start(), 0);
    assert_eq!(set.primary().end(), 6);
}

#[test]
fn merge_duplicate_selections() {
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(2, 5), Selection::new(2, 5)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
}

#[test]
fn merge_contained_selection() {
    // (anchor=0,head=8) fully contains (anchor=2,head=5) — should merge.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(0, 8), Selection::new(2, 5)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    assert_eq!(set.primary().end(), 8);
}

#[test]
fn merge_clears_sticky_display_col_on_extended_selection() {
    // Merging two overlapping selections clears sticky_display_col on the
    // result — neither side's latched column is valid once the head moves
    // to the union boundary.
    let a = Selection::with_sticky_display_col(0, 5, sticky(42)); // sticky_display_col latched
    let b = Selection::with_sticky_display_col(3, 8, sticky(99)); // sticky_display_col latched
    let mut set = SelectionSet::from_vec_unchecked(vec![a, b], 0);
    // The two selections overlap → they merge into one.
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    // The merged selection must have sticky_display_col cleared — neither
    // side's column is valid.
    assert_eq!(
        set.primary().sticky_display_col,
        None,
        "merge must clear sticky_display_col"
    );
}

#[test]
fn merge_idempotent() {
    // Start with an unmerged overlapping set so the first merge does real work,
    // then verify a second merge is a no-op.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(0, 5), Selection::new(3, 8)], 0);
    set.merge_overlapping_in_place();
    // The first merge must have reduced the two overlapping selections to one.
    assert_eq!(
        set.len(),
        1,
        "first merge must reduce overlapping selections"
    );
    let mut set2 = set.clone();
    set2.merge_overlapping_in_place();
    assert_eq!(
        set, set2,
        "second merge must be a no-op on an already-merged set"
    );
}

#[test]
fn merge_three_into_one() {
    let mut set = SelectionSet::from_vec_unchecked(
        vec![
            Selection::new(0, 4),
            Selection::new(3, 7),
            Selection::new(6, 10),
        ],
        1,
    );
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    assert_eq!(set.primary().start(), 0);
    assert_eq!(set.primary().end(), 10);
}

#[test]
fn merge_overlapping_backward_selections() {
    // Two backward selections that overlap: (anchor=8, head=3) and
    // (anchor=10, head=5). After sorting by start(), the merge should
    // produce a single backward selection spanning 3–10.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(8, 3), Selection::new(10, 5)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 1);
    let s = set.primary();
    assert_eq!(s.start(), 3);
    assert_eq!(s.end(), 10);
    // Merged result should be backward (head < anchor).
    assert!(
        s.head < s.anchor,
        "merged backward selections should stay backward"
    );
}

#[test]
fn merge_sorts_unsorted_input() {
    // Pass selections out of order — merge should sort them first.
    let mut set =
        SelectionSet::from_vec_unchecked(vec![Selection::new(5, 8), Selection::new(0, 3)], 0);
    set.merge_overlapping_in_place();
    assert_eq!(set.len(), 2);
    assert_eq!(set.selections[0].start(), 0);
    assert_eq!(set.selections[1].start(), 5);
}

#[test]
fn map_relocates_primary_by_content() {
    let set = SelectionSet::from_vec(
        vec![Selection::collapsed(0), Selection::collapsed(5)],
        1, // primary is the second one
    );
    // shift(1) is order-preserving; primary should track to its new position.
    let shifted = set.map(|s| s.shift(1));
    assert_eq!(shifted.primary().head, 6); // was 5, shifted by 1
}

#[test]
fn replace_updates_selection() {
    let set = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(5)], 0);
    let updated = set.replace(1, Selection::collapsed(10));
    assert_eq!(updated.selections[1].head, 10);
}

#[test]
fn replace_canonicalizes_overlap() {
    // Replacing a selection with one that overlaps its neighbour must
    // merge them — replace may never leave the set violating invariants.
    let set = SelectionSet::from_vec(vec![Selection::new(0, 2), Selection::new(8, 9)], 0);
    let updated = set.replace(1, Selection::new(1, 5));
    assert_eq!(updated.len(), 1);
    assert_eq!(updated.primary().start(), 0);
    assert_eq!(updated.primary().end(), 5);
}

#[test]
fn replace_canonicalizes_ordering() {
    // Replacing the first selection with one past the second must re-sort.
    let set = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(5)], 1);
    let updated = set.replace(0, Selection::collapsed(9));
    assert_eq!(updated.selections[0].head, 5);
    assert_eq!(updated.selections[1].head, 9);
    // Primary was the selection at 5 — still is after the re-sort.
    assert_eq!(updated.primary().head, 5);
}

// ── map (merge semantics) ─────────────────────────────────────────────────

#[test]
fn map_collapses_to_same_position() {
    // Two cursors at different positions that a motion maps to the same
    // spot — e.g. "go to end of line" when both are on the same line.
    let set = SelectionSet::from_vec(vec![Selection::collapsed(2), Selection::collapsed(7)], 0);
    let merged = set.map(|_| Selection::collapsed(10));
    assert_eq!(merged.len(), 1);
    assert_eq!(merged.primary().head, 10);
}

#[test]
fn map_reorders_reversed_positions() {
    // A motion that reverses the order: cursor at 2 maps to 8, cursor
    // at 7 maps to 1. After merge the result should be sorted [1, 8].
    let set = SelectionSet::from_vec(
        vec![Selection::collapsed(2), Selection::collapsed(7)],
        1, // primary is the second one (at 7)
    );
    let merged = set.map(|s| {
        if s.head == 2 {
            Selection::collapsed(8)
        } else {
            Selection::collapsed(1)
        }
    });
    assert_eq!(merged.len(), 2);
    // Sorted by position: first at 1, second at 8.
    assert_eq!(merged.selections[0].head, 1);
    assert_eq!(merged.selections[1].head, 8);
    // Primary was the cursor at 7 → mapped to 1 → now at index 0.
    assert_eq!(merged.primary().head, 1);
}

// ── keep_primary ──────────────────────────────────────────────────────────

#[test]
fn keep_primary_drops_others() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        1, // primary is the middle one
    );
    let kept = set.keep_primary();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept.primary().head, 5);
    assert_eq!(kept.primary_index(), 0);
}

#[test]
fn keep_primary_single_is_noop() {
    let set = SelectionSet::single(Selection::collapsed(3));
    let kept = set.clone().keep_primary();
    assert_eq!(kept, set);
}

// ── remove ───────────────────────────────────────────────────────────────

#[test]
fn remove_before_primary_shifts_primary_down() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        2, // primary is the last one
    );
    let result = set.remove(0); // remove first
    assert_eq!(result.len(), 2);
    assert_eq!(result.primary().head, 10); // primary shifted from index 2 to 1
    assert_eq!(result.primary_index(), 1);
}

#[test]
fn remove_primary_advances_to_next() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        1, // primary is the middle one
    );
    let result = set.remove(1); // remove the primary
    assert_eq!(result.len(), 2);
    // Next in document order after index 1 is now index 1 (was 2, shifted down)
    assert_eq!(result.primary().head, 10);
}

#[test]
fn remove_primary_at_end_wraps_to_first() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        2, // primary is the last one
    );
    let result = set.remove(2);
    assert_eq!(result.len(), 2);
    // idx=2 % new_len=2 = 0 → wraps to the first selection
    assert_eq!(result.primary().head, 0);
}

#[test]
fn remove_after_primary_leaves_primary_unchanged() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        0, // primary is the first one
    );
    let result = set.remove(2); // remove last
    assert_eq!(result.len(), 2);
    assert_eq!(result.primary().head, 0);
    assert_eq!(result.primary_index(), 0);
}

#[test]
fn remove_single_is_noop() {
    let set = SelectionSet::single(Selection::collapsed(0));
    let result = set.clone().remove(0);
    assert_eq!(result, set); // unchanged — can't remove the only selection
}

// ── cycle_primary ─────────────────────────────────────────────────────────

#[test]
fn cycle_primary_forward() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        0,
    );
    let cycled = set.cycle_primary(1);
    assert_eq!(cycled.primary().head, 5);
    let cycled2 = cycled.cycle_primary(1);
    assert_eq!(cycled2.primary().head, 10);
}

#[test]
fn cycle_primary_forward_wraps() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        2,
    );
    let cycled = set.cycle_primary(1);
    assert_eq!(cycled.primary().head, 0); // wraps back to start
}

#[test]
fn cycle_primary_backward() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        2,
    );
    let cycled = set.cycle_primary(-1);
    assert_eq!(cycled.primary().head, 5);
}

#[test]
fn cycle_primary_backward_wraps() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        0,
    );
    let cycled = set.cycle_primary(-1);
    assert_eq!(cycled.primary().head, 10); // wraps to end
}

#[test]
fn cycle_primary_single_is_noop() {
    let set = SelectionSet::single(Selection::collapsed(5));
    let cycled = set.clone().cycle_primary(1);
    assert_eq!(cycled, set);
}

#[test]
fn map_overlapping_ranges() {
    // Two non-overlapping selections that a motion causes to overlap.
    let set = SelectionSet::from_vec(vec![Selection::new(0, 3), Selection::new(5, 8)], 0);
    // map both to the same range — merge fires automatically.
    let merged = set.map(|_| Selection::new(2, 5));
    assert_eq!(merged.len(), 1);
    assert_eq!(merged.primary().start(), 2);
    assert_eq!(merged.primary().end(), 5);
}

// ── SelectionSet::from_vec panics ─────────────────────────────────────────

#[test]
#[should_panic(expected = "SelectionSet must not be empty")]
fn from_vec_empty_panics() {
    let _ = SelectionSet::from_vec(vec![], 0);
}

#[test]
#[should_panic(expected = "primary index out of bounds")]
fn from_vec_primary_out_of_bounds_panics() {
    let _ = SelectionSet::from_vec(vec![Selection::collapsed(0)], 1);
}

// ── iter_sorted ───────────────────────────────────────────────────────────

#[test]
fn iter_sorted_yields_ascending_order() {
    let set = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0),
            Selection::collapsed(5),
            Selection::collapsed(10),
        ],
        2, // primary is last
    );
    let starts: Vec<usize> = set.iter_sorted().map(|s| s.start()).collect();
    assert_eq!(starts, vec![0, 5, 10]);
}

// ── SelectionSet::validate ────────────────────────────────────────────────

#[test]
fn validate_ok_for_valid_set() {
    let set = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(3)], 0);
    assert!(set.validate(10).is_ok());
}

#[test]
fn validate_err_when_buffer_is_empty() {
    let set = SelectionSet::single(Selection::collapsed(0));
    assert!(matches!(
        set.validate(0),
        Err(crate::error::ValidationError::EmptyBuffer)
    ));
}

#[test]
fn validate_err_when_head_out_of_bounds() {
    // buf_len = 3, head = 5 → out of bounds
    let set = SelectionSet::single(Selection::collapsed(5));
    assert!(matches!(
        set.validate(3),
        Err(crate::error::ValidationError::SelectionOutOfBounds { field: "head", .. })
    ));
}

#[test]
fn validate_err_when_anchor_out_of_bounds() {
    // anchor = 10, head = 1; buf_len = 5 → anchor out of bounds
    let set = SelectionSet::single(Selection::new(10, 1));
    assert!(matches!(
        set.validate(5),
        Err(crate::error::ValidationError::SelectionOutOfBounds {
            field: "anchor",
            ..
        })
    ));
}

#[test]
fn validate_passes_when_head_is_last_valid_char() {
    // head = buf_len - 1 is the largest valid position
    let set = SelectionSet::single(Selection::collapsed(4));
    assert!(set.validate(5).is_ok());
}

// ── translate_in_place ────────────────────────────────────────────────────

#[test]
fn translate_in_place_remaps_positions_and_resets_sticky_display_col_only_on_touched_lines() {
    // "aaa\nbbb\nccc\n" (12 chars): line0 = aaa\n [0,4), line1 = bbb\n [4,8),
    // line2 = ccc\n [8,12).
    let buf_pre = BufferText::from("aaa\nbbb\nccc");

    // sel0: collapsed at 1, on the untouched line0.
    // sel1: range (5,6), fully inside the edited span on line1.
    // sel2: collapsed at 9, on the untouched line2.
    let sel0 = Selection::with_sticky_display_col(1, 1, sticky(5));
    let sel1 = Selection::with_sticky_display_col(5, 6, sticky(9)); // forward: anchor <= head
    let sel2 = Selection::with_sticky_display_col(9, 9, sticky(7));
    let mut set = SelectionSet::from_vec(vec![sel0, sel1, sel2], 0);

    // Replace "bbb" (positions 4..7) with "XY" — net -1 char, entirely
    // within line1.
    let mut b = ChangeSetBuilder::new(12);
    b.retain(4);
    b.delete(3);
    b.insert("XY");
    b.retain_rest();
    let cs = b.finish();

    set.translate_in_place(&cs, &buf_pre);

    assert_eq!(set.len(), 3, "no selection should merge here");

    // sel0: untouched line, position unchanged, sticky_display_col preserved.
    let s0 = set.iter_sorted().next().unwrap();
    assert_eq!((s0.anchor(), s0.head()), (1, 1));
    assert_eq!(s0.sticky_display_col(), Some(sticky(5)));

    // sel1: collapses onto the replacement point (old positions 5,6 both
    // fell inside the deleted "bbb"), direction preserved, sticky_display_col
    // reset because its line was edited.
    let s1 = set.iter_sorted().nth(1).unwrap();
    assert_eq!((s1.anchor(), s1.head()), (4, 4));
    assert_eq!(s1.sticky_display_col(), None);

    // sel2: untouched line, shifted back by 1 (net delta of the replace),
    // sticky_display_col preserved.
    let s2 = set.iter_sorted().nth(2).unwrap();
    assert_eq!((s2.anchor(), s2.head()), (8, 8));
    assert_eq!(s2.sticky_display_col(), Some(sticky(7)));
}

#[test]
fn translate_in_place_insert_exactly_at_line_start_touches_that_line() {
    // "aa\nbb\n" (6 chars): line0 = "aa\n" [0,3), line1 = "bb\n" [3,6).
    // Insert("X") at old position 3 — exactly line1's start — is a point
    // range [3,3). It must count as touching line1 (matching the
    // pre-batch `touches_line` behavior: `old >= line_start`), not line0.
    let buf_pre = BufferText::from("aa\nbb");
    let sel0 = Selection::with_sticky_display_col(1, 1, sticky(5)); // head=1, on line0
    let sel1 = Selection::with_sticky_display_col(4, 4, sticky(9)); // head=4, on line1
    let mut set = SelectionSet::from_vec(vec![sel0, sel1], 0);

    let mut b = ChangeSetBuilder::new(6);
    b.retain(3);
    b.insert("X");
    b.retain_rest();
    let cs = b.finish();

    set.translate_in_place(&cs, &buf_pre);

    let s0 = set.iter_sorted().next().unwrap();
    assert_eq!(
        s0.sticky_display_col(),
        Some(sticky(5)),
        "line0 wasn't touched — insert lands after it"
    );
    let s1 = set.iter_sorted().nth(1).unwrap();
    assert_eq!(
        s1.sticky_display_col(),
        None,
        "line1 touched — insert lands exactly at its start"
    );
}

#[test]
fn translate_in_place_backward_selection_keeps_direction() {
    // "abcde\n" (6 chars). A backward selection (anchor=4, head=1) sits
    // entirely after the edit; verify anchor/head land on the correct
    // (shifted) ends rather than being swapped.
    let buf_pre = BufferText::from("abcde");
    let sel = Selection::new(4, 1); // backward: head < anchor
    let mut set = SelectionSet::single(sel);

    // Insert "XX" at position 0 — shifts everything after it by 2.
    let mut b = ChangeSetBuilder::new(6);
    b.insert("XX");
    b.retain_rest();
    let cs = b.finish();

    set.translate_in_place(&cs, &buf_pre);

    let s = set.primary();
    assert_eq!(s.anchor(), 6); // was 4, shifted by 2
    assert_eq!(s.head(), 3); // was 1, shifted by 2
    assert!(s.head() < s.anchor(), "must stay backward");
}

#[test]
fn translate_in_place_merges_selections_collapsed_onto_same_point() {
    // "abcdef\n" (7 chars). Two collapsed selections inside a deletion
    // that removes the entire content ("abcdef") both collapse to
    // position 0 and must merge into a single selection.
    let buf_pre = BufferText::from("abcdef");
    let sel0 = Selection::with_sticky_display_col(1, 1, sticky(3));
    let sel1 = Selection::with_sticky_display_col(4, 4, sticky(8));
    let mut set = SelectionSet::from_vec(vec![sel0, sel1], 0);

    let mut b = ChangeSetBuilder::new(7);
    b.delete(6); // remove "abcdef"
    b.retain_rest(); // keep the structural trailing \n
    let cs = b.finish();

    set.translate_in_place(&cs, &buf_pre);

    assert_eq!(set.len(), 1, "both selections collapse onto the same point");
    let s = set.primary();
    assert_eq!((s.anchor(), s.head()), (0, 0));
    assert_eq!(
        s.sticky_display_col(),
        None,
        "merged selection's line was edited"
    );
}
