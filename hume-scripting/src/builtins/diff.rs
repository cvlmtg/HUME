//! `(diff-lines old-text new-text)` / `(diff-buffer-lines pane ref-text)` /
//! `(diff-words old-text new-text)`: native line and word diff, exposed to
//! Steel plugins.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use hume_editing::changeset::{ChangeHunk, LineSpan};

use crate::host::WordDiffHunk;
use crate::types::PaneHandle;

use super::SteelResult;
use super::args::{list_of, not_live_err, string_arg, string_list, symbol_hash};
use super::errors::{generic_err, require_cap};

/// `(diff-lines old-text new-text)` → list of hunk hashes, oldest side
/// first. Each hunk is `(hash 'old-start 'old-count 'new-start 'new-count
/// 'old-lines 'new-lines)`, 0-based; `Equal` runs are dropped. See [`DiffHost`]'s doc
/// for the exact contract (both texts normalized as buffer content).
/// `old-count`/`new-count` are `(length old-lines)`/`(length new-lines)`.
/// [`DiffHunk`] carries no separate count field, so the Steel hash derives
/// them at the boundary rather than duplicating state Rust-side.
///
/// [`DiffHost`]: crate::host::DiffHost
pub(crate) fn diff_lines(ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-lines old-text")?;
    let new = string_arg(new, "diff-lines new-text")?;
    let hunks = require_cap(ctx.host.diff(), "diff-lines")?.diff_lines(&old, &new);
    Ok(hunks_to_steel(hunks))
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
    // `bid`'s liveness is already checked at decode time (`LivePane`), so
    // `DiffHost::diff_buffer_lines` returning `None` here would mean the
    // host answered inconsistently with `buffer_exists`. Never observed,
    // but the trait still returns `Option`, so it's handled rather than
    // assumed.
    let hunks = require_cap(ctx.host.diff(), "diff-buffer-lines")?
        .diff_buffer_lines(bid, &ref_text)
        .ok_or_else(|| not_live_err("diff-buffer-lines", bid))?;
    Ok(hunks_to_steel(hunks))
}

/// `(buffer-revision-diff pane id)` → the hunks that separate `pane`'s buffer's
/// live text from revision `id` of its undo history (a number from
/// `(buffer-undo-tree pane)`), in the shape `diff-lines` returns plus `'words`.
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

/// `'words`, present only when the hunk has word spans: `(hash 'old (list
/// span …) 'new (list span …))`, each span `(hash 'line 'start 'end)`, `'line`
/// counted from the hunk's first line on that side and `'start`/`'end` char
/// columns in it, end exclusive.
fn hunk_to_steel(hunk: ChangeHunk) -> SteelVal {
    let old_count = hunk.old_lines.len();
    let new_count = hunk.new_lines.len();
    let mut fields = vec![
        ("old-start", SteelVal::IntV(hunk.old_start.index() as isize)),
        ("old-count", SteelVal::IntV(old_count as isize)),
        ("new-start", SteelVal::IntV(hunk.new_start.index() as isize)),
        ("new-count", SteelVal::IntV(new_count as isize)),
        ("old-lines", string_list(hunk.old_lines)),
        ("new-lines", string_list(hunk.new_lines)),
    ];
    if let Some(words) = hunk.words {
        fields.push((
            "words",
            symbol_hash([
                ("old", list_of(words.old.into_iter().map(span_to_steel))),
                ("new", list_of(words.new.into_iter().map(span_to_steel))),
            ]),
        ));
    }
    symbol_hash(fields)
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
/// 'old-text 'new-text)`, char offsets, `Equal` runs dropped. `'deadline-hit` is `#t` when
/// the underlying Myers pass timed out and returned a coarse result. See
/// [`DiffHost::diff_words`]'s doc for how a caller should react.
///
/// [`DiffHost::diff_words`]: crate::host::DiffHost::diff_words
pub(crate) fn diff_words(ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-words old-text")?;
    let new = string_arg(new, "diff-words new-text")?;
    let (hunks, deadline_hit) = require_cap(ctx.host.diff(), "diff-words")?.diff_words(&old, &new);
    let hunks = list_of(hunks.into_iter().map(word_hunk_to_steel));
    Ok(symbol_hash([
        ("hunks", hunks),
        ("deadline-hit", SteelVal::BoolV(deadline_hit)),
    ]))
}

fn word_hunk_to_steel(hunk: WordDiffHunk) -> SteelVal {
    symbol_hash([
        ("old-start", SteelVal::IntV(hunk.old_start as isize)),
        ("old-end", SteelVal::IntV(hunk.old_end as isize)),
        ("new-start", SteelVal::IntV(hunk.new_start as isize)),
        ("new-end", SteelVal::IntV(hunk.new_end as isize)),
        ("old-text", SteelVal::StringV(hunk.old_text.into())),
        ("new-text", SteelVal::StringV(hunk.new_text.into())),
    ])
}

#[cfg(test)]
mod tests;
