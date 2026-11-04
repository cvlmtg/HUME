//! Build a `ChangeSet` from a line-level diff.
//!
//! The line diff (`hume_editing::diff::diff_lines`) partitions both the old and
//! new inputs into `Equal` / `Delete` / `Insert` / `Replace` hunks whose line
//! ranges cover the inputs contiguously. [`changeset_from_line_diff`] walks
//! those hunks and emits a fine-grained `ChangeSet` whose operations cover
//! only the lines that actually changed; unchanged `Equal` runs become cheap
//! `Retain(n)` ops carrying no payload.
//!
//! Memory cost of the changeset, and of the inverse the undo tree derives
//! from it, ≈ size of the changed lines only, not the full buffer. This is
//! what lets `:e!` reload record a normal undo step without a coarse
//! delete-all + insert-all that doubles buffer memory.
//! Only what lies between the lines both texts start and end with is split
//! and diffed ([`text_line_diff`]): the lines around it are retained. Its
//! tokens borrow each line from the rope unless the line straddles chunks,
//! and the diff only ever compares and counts them, so a changed line's text
//! for the changeset is the slice [`build_changeset`] takes from `new`;
//! `LineHunkKind` carries no payload of its own, only the line-index ranges.
//! None of this affects what survives in the history tree afterwards, which
//! is just the changed lines.
//!
//! The helper takes `&BufferText` on both sides and returns the `ChangeSet`; it
//! does not mutate either buffer. The caller still owns the text swap.

use std::ops::Range;

use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::diff::{LineHunk, LineHunkKind};
use crate::line_diff::{text_line_diff, token_offsets};
use crate::text::BufferText;

use super::{ChangeSet, ChangeSetBuilder};

/// Build the `ChangeSet` describing the old → new text transformation at the
/// line level.
///
/// It is sized to `old.len_chars()`, and applying it to `old` yields `new`.
/// Memory cost ≈ size of the changed lines only (unchanged `Equal` hunks
/// become `Retain(n)` ops with no payload).
///
/// The caller still owns the `BufferText` mutation; this helper only produces the
/// `ChangeSet`, it does not touch either buffer.
pub fn changeset_from_line_diff(old: &BufferText, new: &BufferText) -> ChangeSet {
    let (old_content, new_content) = (old.content(), new.content());
    let diff = text_line_diff(&old_content, &new_content);
    let old_offsets = token_offsets(&diff.old, diff.ends.old_middle);
    let new_offsets = token_offsets(&diff.new, diff.ends.new_middle);
    build_changeset(old, new, &diff.hunks, &old_offsets, &new_offsets)
}

/// Walk the line-diff hunks and emit the `ChangeSet`.
///
/// `old_offsets` / `new_offsets` are the `token_offsets` of each side's middle:
/// the first entry is where the middle starts, the last where it ends. The
/// shared lines before the middle are retained first and the ones after it by
/// `finish`. Token-derived offsets keep a hunk ending at the middle's trailing
/// empty token in range and the cursor aligned with the rope.
fn build_changeset(
    old: &BufferText,
    new: &BufferText,
    hunks: &[LineHunk],
    old_offsets: &[CharOffset],
    new_offsets: &[CharOffset],
) -> ChangeSet {
    let mut fwd = ChangeSetBuilder::new(old.end());
    fwd.retain_to(old_offsets[0]);

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
            }
            LineHunkKind::Delete => {
                fwd.delete(span(old_offsets, &hunk.old));
            }
            LineHunkKind::Insert => {
                fwd.insert(&slice(new, new_offsets, &hunk.new));
            }
            LineHunkKind::Replace => {
                fwd.delete(span(old_offsets, &hunk.old));
                fwd.insert(&slice(new, new_offsets, &hunk.new));
            }
        }
    }
    fwd.finish()
}

//
// The core check: apply `forward` to a copy of `old` and assert it
// equals `new`. This catches any off-by-one in line → char offset translation, hunk-walk
// gaps, or `retain_rest` misuse, independent of the line-diff implementation.

#[cfg(test)]
mod tests;
