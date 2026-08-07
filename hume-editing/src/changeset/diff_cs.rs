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
//! the full buffer — this is what lets `:e!` reload record a normal undo step
//! without a coarse delete-all + insert-all that doubles buffer memory.
//! [`Text::line_tokens`] borrows its tokens from the rope (owning only where
//! a line straddles a chunk boundary), so building the diff no longer pays a
//! full-buffer `String` copy on either side; changed lines still get
//! materialized once, when [`build_changesets`] re-slices them from
//! `old`/`new` — `LineHunkKind` carries no payload of its own, only the
//! line-index ranges. None of this affects what survives in the history tree
//! afterwards, which is just the changed lines.
//!
//! The helper takes `&Text` on both sides and returns the two `ChangeSet`s; it
//! does not mutate either buffer. The caller still owns the text swap.

use std::borrow::Cow;
use std::ops::Range;
use std::time::Duration;

use crate::diff::{LineHunk, LineHunkKind, diff_lines_with_deadline};
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
    let (old_tokens, old_offsets) = tokens_with_offsets(old);
    let (new_tokens, new_offsets) = tokens_with_offsets(new);

    let diff = diff_lines_with_deadline(&old_tokens, &new_tokens, deadline);

    debug_assert_eq!(
        old_offsets
            .last()
            .copied()
            .expect("tokens_with_offsets always pushes the trailing sentinel"),
        old.len_chars(),
        "old token char count must equal rope len_chars",
    );
    debug_assert_eq!(
        new_offsets
            .last()
            .copied()
            .expect("tokens_with_offsets always pushes the trailing sentinel"),
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
        text.slice(offsets[range.start]..offsets[range.end])
            .to_string()
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
            LineHunkKind::Delete => {
                fwd.delete(span(old_offsets, &hunk.old));
                inv.insert(&slice(old, old_offsets, &hunk.old));
            }
            LineHunkKind::Insert => {
                fwd.insert(&slice(new, new_offsets, &hunk.new));
                inv.delete(span(new_offsets, &hunk.new));
            }
            LineHunkKind::Replace => {
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

/// [`Text::line_tokens`] plus the cumulative char offset of each token (with
/// a trailing sentinel): `offsets[i]` is the char offset where token `i`
/// starts and `offsets[tokens.len()]` is the total char count, matching
/// `text.len_chars()`. `build_changesets` needs both — the tokens to diff,
/// the offsets to translate a hunk's line-index range back to a char range
/// into the rope.
fn tokens_with_offsets(text: &Text) -> (Vec<Cow<'_, str>>, Vec<usize>) {
    let mut tokens = Vec::with_capacity(text.len_lines());
    let mut offsets = Vec::with_capacity(text.len_lines() + 1);
    offsets.push(0);
    let mut char_acc = 0usize;
    for token in text.line_tokens() {
        char_acc += token.chars().count();
        offsets.push(char_acc);
        tokens.push(token);
    }
    (tokens, offsets)
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The load-bearing oracle: apply `forward` to a copy of `old` and assert it
// equals `new`; apply `inverse` to a copy of `new` and assert it equals `old`.
// This catches any off-by-one in line → char offset translation, hunk-walk
// gaps, or `retain_rest` misuse — independent of the line-diff implementation.

#[cfg(test)]
mod tests;
