//! `(diff-lines old-text new-text)` / `(diff-buffer-lines pane ref-text)` /
//! `(buffer-revision-diff pane id)` / `(buffer-changes-since pane generation)` /
//! `(diff-words old-text new-text)`:
//! native line and word diff, exposed to Steel plugins. `diff-lines` and
//! `diff-words` read no editor state, so they call `hume-editing` directly.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use hume_editing::diff::{WordHunk, WordHunkKind};
use hume_editing::hunk::{ChangeHunk, LineSpan, text_hunks};
use hume_editing::text::BufferText;

use hume_core::BufferId;

use super::SteelResult;
use super::args::{
    END_CHAR_COL, START_CHAR_COL, char_range_value, list_of, not_live_err, string_arg, string_list,
    symbol_hash,
};
use super::errors::{generic_err, require_cap};
use super::ids::BufferGeneration;

/// `(diff-lines old-text new-text)` → `(hash 'hunks (…))`: the hunks, oldest
/// side first. Each hunk is `(hash 'old-start-line 'old-end-line 'new-start-line
/// 'new-end-line 'old-lines 'new-lines 'words)`: 0-based lines, end-exclusive,
/// so a side with no lines has an empty range at the point the other side was
/// inserted at or deleted from. `Equal` runs are dropped. Both texts are read
/// as buffer content (see [`text_hunks`]). The end lines are derived here
/// from the line lists, since [`ChangeHunk`] stores no end.
pub(crate) fn diff_lines(old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "old-text")?;
    let new = string_arg(new, "new-text")?;
    Ok(hunks_to_steel(text_hunks(
        &BufferText::from(old.as_str()),
        &BufferText::from(new.as_str()),
    )))
}

/// `(diff-buffer-lines pane ref-text)` → same shape as `diff-lines`, diffing
/// `ref-text` (old) against `pane`'s buffer's live text (new) without
/// round-tripping the whole buffer through Steel.
pub(crate) fn diff_buffer_lines(
    ctx: &mut SteelCtx,
    bid: BufferId,
    ref_text: SteelVal,
) -> SteelResult {
    let ref_text = string_arg(ref_text, "ref-text")?;
    // `LiveBuffer` already checked liveness; `None` would be a host disagreeing with itself.
    let diff = require_cap(ctx.host.diff())?
        .diff_buffer_lines(bid, &ref_text)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(hunks_to_steel(diff))
}

/// `(buffer-revision-diff pane id)` → the `(hash 'hunks)` that
/// separates `pane`'s buffer's live text from revision `id` of its undo
/// history (a number from `(buffer-undo-tree pane)`), as `diff-lines` returns.
/// The revision's text is the old side and the live text the new. Raises when
/// the buffer has no such revision.
pub(crate) fn buffer_revision_diff(
    ctx: &mut SteelCtx,
    bid: BufferId,
    revision: usize,
) -> SteelResult {
    let diff = require_cap(ctx.host.diff())?
        .revision_diff(bid, revision)
        .map_err(generic_err)?;
    Ok(hunks_to_steel(diff))
}

/// `(buffer-changes-since pane generation)` → the hunks separating the text
/// of `generation`, a `buffer-generation` value of `pane`'s buffer, from the
/// live text, as [`hunk_list_value`] gives them: the old side is the text at
/// `generation`. `#f` when the buffer cannot say. Raises for a generation
/// of another buffer.
pub(crate) fn buffer_changes_since(
    ctx: &mut SteelCtx,
    bid: BufferId,
    generation: SteelVal,
) -> SteelResult {
    let generation = BufferGeneration::of(&generation, bid, "generation").map_err(generic_err)?;
    let hunks = require_cap(ctx.host.diff())?
        .changes_since(bid, generation)
        .map_err(generic_err)?;
    Ok(hunk_list_value(hunks))
}

/// `hunks` as a list of the hunks `diff-lines` returns, or `#f` for `None`:
/// the `on-text-changed` payload and what `buffer-changes-since` returns.
pub fn hunk_list_value(hunks: Option<Vec<ChangeHunk>>) -> SteelVal {
    hunks.map_or(SteelVal::BoolV(false), hunk_list)
}

fn hunks_to_steel(hunks: Vec<ChangeHunk>) -> SteelVal {
    symbol_hash([("hunks", hunk_list(hunks))])
}

fn hunk_list(hunks: Vec<ChangeHunk>) -> SteelVal {
    list_of(hunks.into_iter().map(hunk_to_steel))
}

/// `'words`: `(hash 'old (list span …) 'new (list span …))`, or `#f` when the
/// hunk's word diff was skipped for the word budget. An old-side span is
/// `(hash 'line 'start-char-col 'end-char-col)`: a line of the old text and
/// char columns in it, end exclusive. A new-side span is `(hash 'start-char
/// 'end-char)`: char offsets into the new text, end exclusive. For
/// `diff-lines` that text is `new-text` with LF line endings, so each CRLF
/// counts as one char.
fn hunk_to_steel(hunk: ChangeHunk) -> SteelVal {
    let old_end = hunk.old_start.advance(hunk.old_lines.len());
    let new_end = hunk.new_start.advance(hunk.new_lines.len());
    symbol_hash([
        (
            "old-start-line",
            SteelVal::IntV(hunk.old_start.index() as isize),
        ),
        ("old-end-line", SteelVal::IntV(old_end.index() as isize)),
        (
            "new-start-line",
            SteelVal::IntV(hunk.new_start.index() as isize),
        ),
        ("new-end-line", SteelVal::IntV(new_end.index() as isize)),
        ("old-lines", string_list(hunk.old_lines)),
        ("new-lines", string_list(hunk.new_lines)),
        (
            "words",
            hunk.words.map_or(SteelVal::BoolV(false), |words| {
                symbol_hash([
                    (
                        "old",
                        list_of(words.old.into_iter().map(line_span_to_steel)),
                    ),
                    (
                        "new",
                        list_of(
                            words.new.into_iter().map(|range| {
                                char_range_value(range.start.index(), range.end.index())
                            }),
                        ),
                    ),
                ])
            }),
        ),
    ])
}

fn line_span_to_steel(span: LineSpan) -> SteelVal {
    symbol_hash([
        ("line", SteelVal::IntV(span.line.index() as isize)),
        (START_CHAR_COL, SteelVal::IntV(span.start.index() as isize)),
        (END_CHAR_COL, SteelVal::IntV(span.end.index() as isize)),
    ])
}

/// `(diff-words old-text new-text)` → `(hash 'hunks …)`. `'hunks` is a list
/// of `(hash 'old-start-char 'old-end-char 'new-start-char 'new-end-char
/// 'old-text 'new-text)`, char offsets, `Equal` runs dropped; a pure insert or
/// delete has a zero-width side with empty text.
pub(crate) fn diff_words(old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "old-text")?;
    let new = string_arg(new, "new-text")?;
    let hunks = hume_editing::diff::diff_words(&old, &new)
        .into_iter()
        .filter_map(word_hunk_to_steel);
    Ok(symbol_hash([("hunks", list_of(hunks))]))
}

fn word_hunk_to_steel(hunk: WordHunk) -> Option<SteelVal> {
    let (old_text, new_text) = match hunk.kind {
        WordHunkKind::Equal => return None,
        WordHunkKind::Delete(old) => (old, String::new()),
        WordHunkKind::Insert(new) => (String::new(), new),
        WordHunkKind::Replace { old, new } => (old, new),
    };
    Some(symbol_hash([
        ("old-start-char", SteelVal::IntV(hunk.old.start as isize)),
        ("old-end-char", SteelVal::IntV(hunk.old.end as isize)),
        ("new-start-char", SteelVal::IntV(hunk.new.start as isize)),
        ("new-end-char", SteelVal::IntV(hunk.new.end as isize)),
        ("old-text", SteelVal::StringV(old_text.into())),
        ("new-text", SteelVal::StringV(new_text.into())),
    ]))
}

#[cfg(test)]
mod tests;
