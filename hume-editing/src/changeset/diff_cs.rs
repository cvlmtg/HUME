//! Build a `(forward, inverse)` `ChangeSet` pair from a line-level diff.
//!
//! The line diff (`hume_editing::diff::diff_lines`) partitions both the old and
//! new inputs into `Equal` / `Delete` / `Insert` / `Replace` hunks whose line
//! ranges cover the inputs contiguously. [`changesets_from_line_diff`] walks
//! those hunks and emits a fine-grained `ChangeSet` whose operations cover
//! only the lines that actually changed — unchanged `Equal` runs become cheap
//! `Retain(n)` ops carrying no payload.
//!
//! Memory cost of the *stored* inverse ≈ size of the changed lines only, not
//! the full buffer. This is what lets `:e!` reload record a normal undo step
//! without forcing a coarse delete-all + insert-all that doubles buffer memory.
//!
//! Note this is about the retained undo step, not peak cost: building the diff
//! materializes both sides into contiguous `String`s, so the transient working
//! set during a reload is ~2× the buffer. The win is in what survives in the
//! history tree afterwards.
//!
//! The helper takes `&Text` on both sides and returns the two `ChangeSet`s; it
//! does not mutate either buffer. The caller still owns the text swap.

use std::ops::Range;
use std::time::Duration;

use crate::diff::{diff_lines_with_deadline, LineHunk, LineHunkKind};
use crate::text::Text;

use super::{ChangeSet, ChangeSetBuilder};

/// Build `(forward, inverse)` `ChangeSet`s describing the old → new text
/// transformation at the line level, using the default line-diff deadline.
///
/// `forward` is sized to `old.len_chars()`; `inverse` to `new.len_chars()`.
/// Applying `forward` to `old` yields `new`; applying `inverse` to `new` yields
/// `old`. Memory cost ≈ size of the changed lines only (unchanged `Equal` hunks
/// become `Retain(n)` ops with no payload).
///
/// The caller still owns the `Text` mutation — this helper only produces the
/// `ChangeSet`s, it does not touch either buffer.
pub fn changesets_from_line_diff(old: &Text, new: &Text) -> (ChangeSet, ChangeSet) {
    changesets_from_line_diff_with_deadline(old, new, crate::diff::DIFF_LINE_DEADLINE)
}

/// Like [`changesets_from_line_diff`] but with an explicit line-diff deadline.
/// Exposed primarily for tests that need to force the Myers fallback
/// (`Duration::ZERO`) to exercise the coarse single-Replace path.
pub fn changesets_from_line_diff_with_deadline(
    old: &Text,
    new: &Text,
    deadline: Duration,
) -> (ChangeSet, ChangeSet) {
    // Materialize each side into one contiguous `String`, then tokenise keeping
    // the trailing `\n` with each line. Keeping the `\n` in the token is
    // load-bearing: an `Equal` hunk then means byte-identical token slices, so
    // equal char spans on both sides — including across the trailing-empty-line
    // boundary, where the bare-`split('\n')` approach would align a 0-char
    // trailing line with a 1-char internal `"\n"` line and desynchronise the
    // forward/inverse char cursors by exactly one `\n`.
    let old_str = old.to_string();
    let new_str = new.to_string();
    let (old_tokens, old_offsets) = lines_keep_newline(&old_str);
    let (new_tokens, new_offsets) = lines_keep_newline(&new_str);

    let diff = diff_lines_with_deadline(&old_tokens, &new_tokens, deadline);

    debug_assert_eq!(
        *old_offsets.last().unwrap(),
        old.len_chars(),
        "old token char count must equal rope len_chars",
    );
    debug_assert_eq!(
        *new_offsets.last().unwrap(),
        new.len_chars(),
        "new token char count must equal rope len_chars",
    );

    build_changesets(old, new, &diff.hunks, &old_offsets, &new_offsets)
}

/// Walk the line-diff hunks and emit the `(forward, inverse)` `ChangeSet` pair.
///
/// `old_offsets` / `new_offsets` are the cumulative char-offset tables over
/// each side's line tokens (length `tokens.len() + 1`, last entry == buffer
/// `len_chars`). Using token-derived offsets — not `Text::line_to_char` — keeps
/// the hunk end at `len_lines()` (the trailing empty token) panic-free and
/// keeps the forward/inverse cursors byte-for-byte aligned with the rope.
fn build_changesets(
    old: &Text,
    new: &Text,
    hunks: &[LineHunk],
    old_offsets: &[usize],
    new_offsets: &[usize],
) -> (ChangeSet, ChangeSet) {
    let mut fwd = ChangeSetBuilder::new(old.len_chars());
    let mut inv = ChangeSetBuilder::new(new.len_chars());

    let span = |offsets: &[usize], range: &Range<usize>| offsets[range.end] - offsets[range.start];
    let slice = |text: &Text, offsets: &[usize], range: &Range<usize>| {
        text.slice(offsets[range.start]..offsets[range.end]).to_string()
    };

    for hunk in hunks {
        match &hunk.kind {
            LineHunkKind::Equal => {
                debug_assert_eq!(
                    span(old_offsets, &hunk.old),
                    span(new_offsets, &hunk.new),
                    "Equal hunk must cover equal char counts (tokens keep \\n, so this holds)",
                );
                fwd.retain(span(old_offsets, &hunk.old));
                inv.retain(span(new_offsets, &hunk.new));
            }
            LineHunkKind::Delete(_) => {
                fwd.delete(span(old_offsets, &hunk.old));
                inv.insert(&slice(old, old_offsets, &hunk.old));
            }
            LineHunkKind::Insert(_) => {
                fwd.insert(&slice(new, new_offsets, &hunk.new));
                inv.delete(span(new_offsets, &hunk.new));
            }
            LineHunkKind::Replace { .. } => {
                fwd.delete(span(old_offsets, &hunk.old));
                fwd.insert(&slice(new, new_offsets, &hunk.new));
                inv.delete(span(new_offsets, &hunk.new));
                inv.insert(&slice(old, old_offsets, &hunk.old));
            }
        }
    }
    fwd.retain_rest();
    inv.retain_rest();
    (fwd.finish(), inv.finish())
}

/// Tokenise `s` into line slices, keeping the trailing `\n` with each line, and
/// return the cumulative char offset of each token (with a trailing sentinel).
///
/// `"a\nb\n"` → tokens `["a\n", "b\n", ""]`, offsets `[0, 2, 4, 4]`. The final
/// token after the last `\n` is the empty trailing line (`""`, no `\n` of its
/// own) — HUME's buffer invariant guarantees `s` ends with `\n`, so this token
/// is always present and empty. `offsets[i]` is the char offset where token `i`
/// starts and `offsets[tokens.len()]` is the total char count, so the two
/// `Vec`s are built in a single pass.
///
/// Keeping the `\n` in the token makes an `Equal` hunk correspond to
/// byte-identical slices on both sides, so the forward/inverse char spans agree
/// even when one side's trailing-empty line aligns with the other side's
/// internal empty line.
fn lines_keep_newline(s: &str) -> (Vec<&str>, Vec<usize>) {
    let mut tokens = Vec::new();
    let mut offsets = vec![0usize];
    let mut start = 0;
    let mut char_acc = 0usize;
    for (i, ch) in s.char_indices() {
        char_acc += 1;
        if ch == '\n' {
            tokens.push(&s[start..=i]);
            offsets.push(char_acc);
            start = i + 1;
        }
    }
    tokens.push(&s[start..]);
    offsets.push(char_acc);
    (tokens, offsets)
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The load-bearing oracle: apply `forward` to a copy of `old` and assert it
// equals `new`; apply `inverse` to a copy of `new` and assert it equals `old`.
// This catches any off-by-one in line → char offset translation, hunk-walk
// gaps, or `retain_rest` misuse — independent of the line-diff implementation.

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Independent oracle: forward(old) == new and inverse(new) == old.
    fn assert_round_trip(old: &str, new: &str) {
        let old_t = Text::from(old);
        let new_t = Text::from(new);
        let (fwd, inv) = changesets_from_line_diff(&old_t, &new_t);

        let applied_fwd = fwd.apply(&old_t).expect("forward apply");
        assert_eq!(
            applied_fwd.to_string(),
            new_t.to_string(),
            "forward(old) must equal new",
        );
        let applied_inv = inv.apply(&new_t).expect("inverse apply");
        assert_eq!(
            applied_inv.to_string(),
            old_t.to_string(),
            "inverse(new) must equal old",
        );

        // Length invariants: forward sized to old, inverse sized to new.
        assert_eq!(fwd.len_before, old_t.len_chars());
        assert_eq!(fwd.len_after, new_t.len_chars());
        assert_eq!(inv.len_before, new_t.len_chars());
        assert_eq!(inv.len_after, old_t.len_chars());
    }

    #[test]
    fn identical_inputs() {
        assert_round_trip("hello\nworld\n", "hello\nworld\n");
        assert_round_trip("a\n", "a\n");
    }

    #[test]
    fn pure_insert_at_head() {
        assert_round_trip("b\nc\n", "a\nb\nc\n");
    }

    #[test]
    fn pure_insert_in_middle() {
        assert_round_trip("a\nc\n", "a\nb\nc\n");
    }

    #[test]
    fn pure_insert_at_tail() {
        assert_round_trip("a\nb\n", "a\nb\nc\n");
    }

    #[test]
    fn pure_delete_at_head() {
        assert_round_trip("a\nb\nc\n", "b\nc\n");
    }

    #[test]
    fn pure_delete_in_middle() {
        assert_round_trip("a\nb\nc\n", "a\nc\n");
    }

    #[test]
    fn pure_delete_at_tail() {
        assert_round_trip("a\nb\nc\n", "a\nb\n");
    }

    #[test]
    fn replace_block_in_middle() {
        assert_round_trip("a\n1\n2\n3\nz\n", "a\nX\nY\nz\n");
    }

    #[test]
    fn internal_empty_lines() {
        // Internal empty lines carry their own `\n`; the trailing-empty token
        // is `""` on both sides. Round-trips across all three boundaries.
        assert_round_trip("a\n\nb\n", "a\n\n\nc\n");
        assert_round_trip("a\n\nb\n", "a\nb\n");
        assert_round_trip("\n\na\n", "\na\n");
    }

    #[test]
    fn trailing_newline_buffer() {
        // The trailing-empty token is `""`; the line before it is `"a\n"`.
        assert_round_trip("a\n", "a\n");
        assert_round_trip("a\n", "b\n");
        assert_round_trip("a\nb\n", "a\n");
        // Regression for the original oracle failure: a single-content-line
        // buffer vs a single-`\n` buffer. The bare-`split('\n')` approach
        // desynced the cursors by one `\n` here.
        assert_round_trip("a\n", "\n");
        assert_round_trip("\n", "a\n");
    }

    #[test]
    fn zwj_emoji_inside_equal_run() {
        // The family emoji is a ZWJ sequence of 5 chars. It lives inside an
        // Equal line so the line diff must treat it as opaque content; the
        // char-offset walk carries it through unchanged.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let old = format!("x {family} y\nshared\n");
        let new = format!("x {family} z\nshared\n");
        assert_round_trip(&old, &new);
    }

    #[test]
    fn completely_different_inputs() {
        assert_round_trip("aaa\nbbb\n", "xxx\nyyy\nzzz\n");
    }

    #[test]
    fn myers_coarse_replace_still_round_trips() {
        // Duration::ZERO forces the histogram pass to bail and Myers to
        // return the coarsest single-Replace result. The hunk partition
        // invariant still holds, so the round-trip must succeed regardless.
        let old: String = (0..40).map(|i| format!("line-{i}-pad-{}", i % 3)).collect();
        let new: String = (0..40).map(|i| format!("line-{i}-pad-{}", (i + 1) % 5)).collect();
        let old_t = Text::from(old.as_str());
        let new_t = Text::from(new.as_str());
        let (fwd, inv) = changesets_from_line_diff_with_deadline(&old_t, &new_t, Duration::ZERO);

        let applied_fwd = fwd.apply(&old_t).expect("forward apply");
        assert_eq!(applied_fwd.to_string(), new_t.to_string());
        let applied_inv = inv.apply(&new_t).expect("inverse apply");
        assert_eq!(applied_inv.to_string(), old_t.to_string());
    }

    #[test]
    fn inverse_is_fine_grained_for_single_line_change() {
        // Pins the "fine-grained, not coarse" property: a single-line edit's
        // inverse must carry only the changed line, not a full-buffer delete.
        let old = Text::from("alpha\nbeta\ngamma\n");
        let new = Text::from("alpha\nBETA\ngamma\n");
        let (_fwd, inv) = changesets_from_line_diff(&old, &new);

        let inv_text = inv.apply(&new).expect("inverse apply");
        assert_eq!(inv_text.to_string(), "alpha\nbeta\ngamma\n");
        // Coarse inverse would be Delete(18) | Insert("alpha\nbeta\ngamma\n").
        // The fine-grained inverse re-inserts only the changed line.
        let has_small_insert = inv.ops().iter().any(|op| match op {
            super::super::Operation::Insert(s) => s == "beta\n",
            _ => false,
        });
        assert!(
            has_small_insert,
            "inverse should re-insert only the changed line, got {:?}",
            inv.ops(),
        );
    }

    // ── property-based round-trip ────────────────────────────────────────────

    use proptest::prelude::*;

    fn arb_small_text(max_len: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec!['a', 'b', 'c', '\n', '\u{0301}']),
            0..max_len,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        #[test]
        fn prop_round_trip(old in arb_small_text(16), new in arb_small_text(16)) {
            let old_t = Text::from(old.as_str());
            let new_t = Text::from(new.as_str());
            let (fwd, inv) = changesets_from_line_diff(&old_t, &new_t);

            let applied_fwd = fwd.apply(&old_t).expect("forward apply");
            prop_assert_eq!(applied_fwd.to_string(), new_t.to_string());

            let applied_inv = inv.apply(&new_t).expect("inverse apply");
            prop_assert_eq!(applied_inv.to_string(), old_t.to_string());
        }
    }
}