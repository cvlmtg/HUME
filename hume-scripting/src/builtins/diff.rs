//! `(diff-lines old-text new-text)` / `(diff-buffer-lines pane ref-text)` /
//! `(buffer-revision-diff pane id)` / `(diff-words old-text new-text)`:
//! native line and word diff, exposed to Steel plugins. `diff-lines` and
//! `diff-words` read no editor state, so they call `hume-editing` directly.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use hume_editing::diff::{WordHunk, WordHunkKind};
use hume_editing::hunk::{ChangeHunk, LineSpan, text_hunks};
use hume_editing::text::BufferText;

use crate::types::PaneHandle;

use super::SteelResult;
use super::args::{list_of, not_live_err, string_arg, string_list, symbol_hash};
use super::errors::{generic_err, require_cap};

/// `(diff-lines old-text new-text)` → list of hunk hashes, oldest side
/// first. Each hunk is `(hash 'old-start 'old-count 'new-start 'new-count
/// 'old-lines 'new-lines 'words)`, 0-based; `Equal` runs are dropped. Both
/// texts are read as buffer content (see [`text_hunks`]).
/// `old-count`/`new-count` are `(length old-lines)`/`(length new-lines)`,
/// derived here since [`ChangeHunk`] stores no count.
pub(crate) fn diff_lines(_ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-lines old-text")?;
    let new = string_arg(new, "diff-lines new-text")?;
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
    pane: PaneHandle,
    ref_text: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let ref_text = string_arg(ref_text, "diff-buffer-lines ref-text")?;
    // `LivePane` already checked liveness; `None` would be a host disagreeing with itself.
    let hunks = require_cap(ctx.host.diff(), "diff-buffer-lines")?
        .diff_buffer_lines(bid, &ref_text)
        .ok_or_else(|| not_live_err("diff-buffer-lines", bid))?;
    Ok(hunks_to_steel(hunks))
}

/// `(buffer-revision-diff pane id)` → the hunks that separate `pane`'s buffer's
/// live text from revision `id` of its undo history (a number from
/// `(buffer-undo-tree pane)`), in the shape `diff-lines` returns.
/// The revision's text is the old side and the live text the new. Raises when
/// the buffer has no such revision.
pub(crate) fn buffer_revision_diff(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    revision: usize,
) -> SteelResult {
    let hunks = require_cap(ctx.host.diff(), "buffer-revision-diff")?
        .revision_diff(pane.buffer(), revision)
        .map_err(generic_err)?;
    Ok(hunks_to_steel(hunks))
}

fn hunks_to_steel(hunks: Vec<ChangeHunk>) -> SteelVal {
    list_of(hunks.into_iter().map(hunk_to_steel))
}

/// `'words`: `(hash 'old (list span …) 'new (list span …))`, each span `(hash 'line 'start 'end)`, `'line`
/// counted from the hunk's first line on that side and `'start`/`'end` char
/// columns in it, end exclusive.
fn hunk_to_steel(hunk: ChangeHunk) -> SteelVal {
    let old_count = hunk.old_lines.len();
    let new_count = hunk.new_lines.len();
    symbol_hash([
        ("old-start", SteelVal::IntV(hunk.old_start.index() as isize)),
        ("old-count", SteelVal::IntV(old_count as isize)),
        ("new-start", SteelVal::IntV(hunk.new_start.index() as isize)),
        ("new-count", SteelVal::IntV(new_count as isize)),
        ("old-lines", string_list(hunk.old_lines)),
        ("new-lines", string_list(hunk.new_lines)),
        (
            "words",
            symbol_hash([
                (
                    "old",
                    list_of(hunk.words.old.into_iter().map(span_to_steel)),
                ),
                (
                    "new",
                    list_of(hunk.words.new.into_iter().map(span_to_steel)),
                ),
            ]),
        ),
    ])
}

fn span_to_steel(span: LineSpan) -> SteelVal {
    symbol_hash([
        ("line", SteelVal::IntV(span.line as isize)),
        ("start", SteelVal::IntV(span.start.index() as isize)),
        ("end", SteelVal::IntV(span.end.index() as isize)),
    ])
}

/// `(diff-words old-text new-text)` → `(hash 'hunks … 'deadline-hit …)`.
/// `'hunks` is a list of `(hash 'old-start 'old-end 'new-start 'new-end
/// 'old-text 'new-text)`, char offsets, `Equal` runs dropped; a pure insert
/// or delete has a zero-width side with empty text. `'deadline-hit` is `#t`
/// when Myers timed out and returned a coarse result, which a caller should
/// treat as a fallback rather than a precise diff.
pub(crate) fn diff_words(_ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-words old-text")?;
    let new = string_arg(new, "diff-words new-text")?;
    let diff = hume_editing::diff::diff_words(&old, &new);
    let deadline_hit = diff.deadline_hit();
    let hunks = list_of(diff.hunks.into_iter().filter_map(word_hunk_to_steel));
    Ok(symbol_hash([
        ("hunks", hunks),
        ("deadline-hit", SteelVal::BoolV(deadline_hit)),
    ]))
}

fn word_hunk_to_steel(hunk: WordHunk) -> Option<SteelVal> {
    let (old_text, new_text) = match hunk.kind {
        WordHunkKind::Equal => return None,
        WordHunkKind::Delete(old) => (old, String::new()),
        WordHunkKind::Insert(new) => (String::new(), new),
        WordHunkKind::Replace { old, new } => (old, new),
    };
    Some(symbol_hash([
        ("old-start", SteelVal::IntV(hunk.old.start as isize)),
        ("old-end", SteelVal::IntV(hunk.old.end as isize)),
        ("new-start", SteelVal::IntV(hunk.new.start as isize)),
        ("new-end", SteelVal::IntV(hunk.new.end as isize)),
        ("old-text", SteelVal::StringV(old_text.into())),
        ("new-text", SteelVal::StringV(new_text.into())),
    ]))
}

#[cfg(test)]
mod tests;
