//! Build a `(forward, inverse)` `ChangeSet` pair from a line-level diff.
//!
//! The line diff (`hume_editing::diff::diff_lines`) partitions both the old and
//! new inputs into `Equal` / `Delete` / `Insert` / `Replace` hunks whose line
//! ranges cover the inputs contiguously. [`changesets_from_line_diff`] walks
//! those hunks and emits a fine-grained `ChangeSet` whose operations cover
//! only the lines that actually changed; unchanged `Equal` runs become cheap
//! `Retain(n)` ops carrying no payload.
//!
//! Memory cost of the *stored* inverse ≈ size of the changed lines only, not
//! the full buffer. This is what lets `:e!` reload record a normal undo step
//! without a coarse delete-all + insert-all that doubles buffer memory.
//! Only what lies between the lines both texts start and end with is split
//! and diffed ([`shared_line_ends`]): the lines around it are retained.
//! [`BufferText::line_tokens_in`] borrows each line from the rope wherever it
//! can, and the diff only ever compares and counts them, so changed lines get materialized
//! once, when [`build_changesets`] re-slices them from `old`/`new`;
//! `LineHunkKind` carries no payload of its own, only the line-index ranges.
//! None of this affects what survives in the history tree afterwards, which
//! is just the changed lines.
//!
//! The helper takes `&BufferText` on both sides and returns the two `ChangeSet`s; it
//! does not mutate either buffer. The caller still owns the text swap.

use std::ops::Range;

use std::borrow::Cow;

use hume_rope::lines::shared_line_ends;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::diff::{LineHunk, LineHunkKind, diff_lines};
use crate::text::BufferText;

use super::{ChangeSet, ChangeSetBuilder};

/// Build `(forward, inverse)` `ChangeSet`s describing the old → new text
/// transformation at the line level.
///
/// `forward` is sized to `old.len_chars()`; `inverse` to `new.len_chars()`.
/// Applying `forward` to `old` yields `new`; applying `inverse` to `new` yields
/// `old`. Memory cost ≈ size of the changed lines only (unchanged `Equal` hunks
/// become `Retain(n)` ops with no payload).
///
/// The caller still owns the `BufferText` mutation; this helper only produces the
/// `ChangeSet`s, it does not touch either buffer.
pub fn changesets_from_line_diff(old: &BufferText, new: &BufferText) -> (ChangeSet, ChangeSet) {
    let ends = shared_line_ends(old.rope(), new.rope());
    let (old_tokens, old_offsets) = tokens_with_offsets(old, ends.old_middle.clone());
    let (new_tokens, new_offsets) = tokens_with_offsets(new, ends.new_middle.clone());

    let hunks = diff_lines(&old_tokens, &new_tokens);

    debug_assert_eq!(
        old_offsets
            .last()
            .copied()
            .expect("tokens_with_offsets always pushes the trailing sentinel"),
        char_at_byte(old, ends.old_middle.end),
        "old token char count must end at the old middle's end",
    );
    debug_assert_eq!(
        new_offsets
            .last()
            .copied()
            .expect("tokens_with_offsets always pushes the trailing sentinel"),
        char_at_byte(new, ends.new_middle.end),
        "new token char count must end at the new middle's end",
    );

    build_changesets(old, new, &hunks, &old_offsets, &new_offsets)
}

/// Walk the line-diff hunks and emit the `(forward, inverse)` `ChangeSet` pair.
///
/// `old_offsets` / `new_offsets` are the cumulative char-offset tables over
/// each side's middle line tokens (length `tokens.len() + 1`): the first
/// entry is where the middle starts, the last where it ends. The shared lines
/// before the middle are retained first and the ones after it by `finish`. Using token-derived offsets (not `BufferText::line_to_char`) keeps
/// the hunk end at `ropey_line_count()` (the trailing empty token) panic-free
/// and keeps the forward/inverse cursors byte-for-byte aligned with the rope.
fn build_changesets(
    old: &BufferText,
    new: &BufferText,
    hunks: &[LineHunk],
    old_offsets: &[CharOffset],
    new_offsets: &[CharOffset],
) -> (ChangeSet, ChangeSet) {
    let mut fwd = ChangeSetBuilder::new(old.end());
    let mut inv = ChangeSetBuilder::new(new.end());
    fwd.retain_to(old_offsets[0]);
    inv.retain_to(new_offsets[0]);

    let span = |offsets: &[CharOffset], range: &Range<usize>| {
        offsets[range.end].chars_since(offsets[range.start])
    };
    let slice = |text: &BufferText, offsets: &[CharOffset], range: &Range<usize>| {
        text.slice(ExclusiveRange::new(
            offsets[range.start],
            offsets[range.end],
        ))
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
    (fwd.finish(), inv.finish())
}

/// The line tokens of `text`'s bytes `middle` plus the char offset in `text`
/// of each token (with a trailing sentinel): `offsets[i]` is where token `i`
/// starts and `offsets[tokens.len()]` where the middle ends.
/// `build_changesets` needs both: the tokens to diff, the offsets to
/// translate a hunk's line-index range back to a char range into the rope.
fn tokens_with_offsets(
    text: &BufferText,
    middle: Range<usize>,
) -> (Vec<Cow<'_, str>>, Vec<CharOffset>) {
    let start = char_at_byte(text, middle.start);
    let mut tokens = Vec::new();
    let mut offsets = vec![start];
    // Raw `usize` accumulator, minted to `CharOffset` only once each running
    // total is complete, the same "trusted mint" shape as an ASCII delimiter
    // scan's own loop variable: `char_acc` never appears anywhere as a
    // position in its own right until it's pushed.
    let mut char_acc = start.index();
    for token in text.line_tokens_in(middle) {
        char_acc += token.chars().count();
        offsets.push(CharOffset::new(char_acc));
        tokens.push(token);
    }
    (tokens, offsets)
}

/// The char offset of byte `byte`, a line start or the text's end.
fn char_at_byte(text: &BufferText, byte: usize) -> CharOffset {
    CharOffset::new(text.rope().byte_to_char(byte))
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The core check: apply `forward` to a copy of `old` and assert it
// equals `new`; apply `inverse` to a copy of `new` and assert it equals `old`.
// This catches any off-by-one in line → char offset translation, hunk-walk
// gaps, or `retain_rest` misuse, independent of the line-diff implementation.

#[cfg(test)]
mod tests;
