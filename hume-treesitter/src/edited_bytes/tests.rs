use std::ops::RangeInclusive;

use proptest::prelude::*;

use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;
use test_fixtures::unicode;
use tree_sitter::{InputEdit, Point, Range};

use super::EditedBytes;
use crate::edits::input_edits_from_changeset;

/// An edit of one-row text, where a byte's column is its offset.
fn edit(start: usize, old_end: usize, new_end: usize) -> InputEdit {
    let point = |column| Point { row: 0, column };
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(start),
        old_end_position: point(old_end),
        new_end_position: point(new_end),
    }
}

fn recorded(edits: &[InputEdit]) -> Vec<RangeInclusive<usize>> {
    let mut edited = EditedBytes::default();
    for edit in edits {
        edited.record(edit);
    }
    edited.regions
}

/// A range of `bytes` whose start sits at `column` of its row.
fn range(bytes: std::ops::Range<usize>, column: usize) -> Range {
    let point = |column| Point { row: 1, column };
    Range {
        start_byte: bytes.start,
        end_byte: bytes.end,
        start_point: point(column),
        end_point: point(column + bytes.len()),
    }
}

fn touched(edits: &[InputEdit], ranges: &[Range]) -> bool {
    let mut edited = EditedBytes::default();
    for edit in edits {
        edited.record(edit);
    }
    edited.touches(ranges)
}

// ── record ───────────────────────────────────────────────────────────────

#[test]
fn an_insertion_covers_its_bytes_and_the_byte_after_them() {
    assert_eq!(recorded(&[edit(4, 4, 7)]), vec![4..=7]);
}

#[test]
fn a_deletion_covers_the_byte_now_at_its_start() {
    assert_eq!(recorded(&[edit(5, 9, 5)]), vec![5..=5]);
}

#[test]
fn an_insertion_before_a_region_shifts_it() {
    // Three bytes inserted at 2 move bytes 10..=12 to 13..=15.
    assert_eq!(
        recorded(&[edit(10, 10, 12), edit(2, 2, 5)]),
        vec![2..=5, 13..=15]
    );
}

#[test]
fn an_insertion_inside_a_region_widens_it() {
    // Four bytes at 15 move the region's end from 20 to 24.
    assert_eq!(
        recorded(&[edit(10, 10, 20), edit(15, 15, 19)]),
        vec![10..=24]
    );
}

#[test]
fn an_insertion_after_a_region_leaves_it() {
    assert_eq!(
        recorded(&[edit(10, 10, 12), edit(30, 30, 32)]),
        vec![10..=12, 30..=32]
    );
}

#[test]
fn a_deletion_spanning_two_regions_merges_them() {
    // Bytes 3..11 become one byte: 2..=4 and 10..=12 meet the deletion,
    // and 12, past it, moves to 12 - 11 + 4 = 5.
    assert_eq!(
        recorded(&[edit(2, 2, 4), edit(10, 10, 12), edit(3, 11, 4)]),
        vec![2..=5]
    );
}

#[test]
fn a_region_ending_right_before_an_edit_joins_it() {
    assert_eq!(recorded(&[edit(2, 2, 4), edit(5, 5, 6)]), vec![2..=6]);
}

#[test]
fn a_region_one_byte_short_of_an_edit_stays_apart() {
    assert_eq!(
        recorded(&[edit(2, 2, 4), edit(6, 6, 7)]),
        vec![2..=4, 6..=7]
    );
}

#[test]
fn the_edits_of_one_change_land_where_the_text_puts_them() {
    // `ab` at char 2 and `c` at char 10 of the old text: `ab` takes bytes
    // 2..4 and `c` byte 10 + 2 = 12 of the new one.
    let text = BufferText::from("0123456789abcdef");
    let mut builder = ChangeSetBuilder::new(text.end());
    builder.retain_to(CharOffset::new(2));
    builder.insert("ab");
    builder.retain_to(CharOffset::new(10));
    builder.insert("c");
    let edits = input_edits_from_changeset(&builder.finish(), text.rope());
    assert_eq!(recorded(&edits), vec![2..=4, 12..=13]);
}

// ── touches ──────────────────────────────────────────────────────────────
//
// The text is `abcd\nef  XXXXXXXX\n...`: row 1 starts at byte 5, and the
// range `XXXXXXXX` covers bytes 9..17, from column 4.

#[test]
fn an_edit_on_the_range_row_before_the_range_touches_it() {
    assert!(touched(&[edit(6, 6, 7)], &[range(9..17, 4)]));
}

#[test]
fn an_edit_on_an_earlier_row_does_not_touch_the_range() {
    assert!(!touched(&[edit(1, 1, 2)], &[range(9..17, 4)]));
}

#[test]
fn the_byte_after_the_range_touches_it_and_the_next_one_does_not() {
    assert!(touched(&[edit(17, 18, 17)], &[range(9..17, 4)]));
    assert!(!touched(&[edit(18, 18, 19)], &[range(9..17, 4)]));
}

#[test]
fn joining_the_range_row_with_the_previous_one_touches_the_range() {
    // Deleting the newline at 4 makes `abcdef  XXXXXXXX` one row: the
    // range covers bytes 8..16 from column 8.
    assert!(touched(&[edit(4, 5, 4)], &[range(8..16, 8)]));
}

#[test]
fn a_line_break_inserted_before_the_range_touches_it() {
    // A newline after `ef` puts `  XXXXXXXX` on a row of its own,
    // starting at byte 8: the range covers bytes 10..18 from column 2.
    assert!(touched(&[edit(7, 7, 8)], &[range(10..18, 2)]));
}

#[test]
fn an_edit_in_a_gap_row_between_two_ranges_touches_neither() {
    // Ranges on rows 1 and 3 with row 2, bytes 10..18, between them.
    assert!(!touched(
        &[edit(12, 12, 13)],
        &[range(5..9, 0), range(20..25, 2)]
    ));
}

// ── within ───────────────────────────────────────────────────────────────

#[test]
fn within_clips_the_regions_meeting_the_hull_to_it() {
    let mut edited = EditedBytes::default();
    for edit in [edit(2, 2, 4), edit(10, 10, 12), edit(20, 20, 22)] {
        edited.record(&edit);
    }
    assert_eq!(
        edited.within(3..=11).collect::<Vec<_>>(),
        vec![3..=4, 10..=11]
    );
    assert_eq!(edited.within(13..=19).count(), 0);
}

// ── Random edits ─────────────────────────────────────────────────────────

/// Where a byte of the current text comes from.
#[derive(Clone, Copy, Debug)]
struct Origin {
    inserted: bool,
    /// The byte right after inserted text.
    after_insertion: bool,
    /// The byte now at the point a deletion removed text.
    seam: bool,
}

/// What the random texts and replacements are made of: the shared
/// unicode samples, a newline and an ASCII letter.
fn atom(index: usize) -> &'static str {
    unicode::ALL
        .get(index)
        .copied()
        .unwrap_or_else(|| ["\n", "x"][index - unicode::ALL.len()])
}

fn atom_index() -> impl Strategy<Value = usize> {
    0..unicode::ALL.len() + 2
}

/// Replacements of one change: a char position (reduced to the text),
/// chars deleted there and the atom inserted.
type Change = Vec<(usize, usize, Option<usize>)>;

fn change() -> impl Strategy<Value = Change> {
    proptest::collection::vec(
        (
            any::<usize>(),
            0..=3usize,
            proptest::option::of(atom_index()),
        ),
        1..=3,
    )
}

proptest! {
    #[test]
    fn every_inserted_byte_the_byte_after_it_and_each_deletion_seam_lie_in_a_region(
        atoms in proptest::collection::vec(atom_index(), 0..12),
        changes in proptest::collection::vec(change(), 1..=4),
    ) {
        let mut source: String = atoms.into_iter().map(atom).collect();
        source.push('\n');
        let original = Origin { inserted: false, after_insertion: false, seam: false };
        let mut origins = vec![original; source.len()];
        let mut edited = EditedBytes::default();
        for change in changes {
            let text = BufferText::from(source.as_str());
            let final_newline = text.len_chars() - 1;
            let mut replaces: Vec<(usize, usize, &str)> = change
                .into_iter()
                .map(|(at, deleted, inserted)| {
                    let at = at % (final_newline + 1);
                    (at, deleted.min(final_newline - at), inserted.map_or("", atom))
                })
                .collect();
            replaces.sort_by_key(|&(at, ..)| at);
            let mut kept_end = 0;
            replaces.retain(|&(at, deleted, inserted)| {
                let keep = at >= kept_end && (deleted > 0 || !inserted.is_empty());
                if keep {
                    kept_end = at + deleted;
                }
                keep
            });
            if replaces.is_empty() {
                continue;
            }

            let byte = |char_index: usize| text.rope().char_to_byte(char_index);
            let mut builder = ChangeSetBuilder::new(text.end());
            let mut next_source = String::new();
            let mut next_origins = Vec::new();
            let mut copied = 0;
            for &(at, deleted, inserted) in &replaces {
                builder.retain_to(CharOffset::new(at));
                builder.delete_to(CharOffset::new(at + deleted));
                builder.insert(inserted);
                next_source.push_str(&source[copied..byte(at)]);
                next_origins.extend_from_slice(&origins[copied..byte(at)]);
                next_source.push_str(inserted);
                next_origins.extend(
                    inserted
                        .bytes()
                        .map(|_| Origin { inserted: true, after_insertion: false, seam: false }),
                );
                copied = byte(at + deleted);
                if deleted > 0 {
                    origins[copied].seam = true;
                }
                if !inserted.is_empty() {
                    origins[copied].after_insertion = true;
                }
            }
            next_source.push_str(&source[copied..]);
            next_origins.extend_from_slice(&origins[copied..]);

            let change = builder.finish();
            prop_assert_eq!(&change.apply(&text).expect("the change applies").to_string(), &next_source);
            for edit in input_edits_from_changeset(&change, text.rope()) {
                edited.record(&edit);
            }
            source = next_source;
            origins = next_origins;
        }

        for (byte, origin) in origins.iter().enumerate() {
            if origin.inserted || origin.after_insertion || origin.seam {
                prop_assert!(
                    edited.regions.iter().any(|region| region.contains(&byte)),
                    "byte {} ({:?}) of {:?} lies in none of {:?}",
                    byte, origin, source, edited.regions
                );
            }
        }
        for pair in edited.regions.windows(2) {
            prop_assert!(pair[0].end() + 1 < *pair[1].start(), "{:?} are sorted and apart", edited.regions);
        }
    }
}
