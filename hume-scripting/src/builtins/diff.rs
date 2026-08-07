//! `(diff-lines old-text new-text)` / `(diff-buffer-lines bid ref-text)` /
//! `(diff-words old-text new-text)` — native line and word diff, exposed to
//! Steel plugins (Phase 2a/2b, `docs/GIT-DIFF.md`).

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{DiffHunk, WordDiffHunk};

use super::SteelResult;
use super::args::{BidArg, cons_pair, string_arg};
use super::errors::require_cap;

/// `(diff-lines old-text new-text)` → list of hunk tuples, oldest side
/// first. Each hunk is `(old-start old-count new-start new-count old-lines
/// new-lines)`, 0-based; `Equal` runs are dropped. See [`DiffHost`]'s doc
/// for the exact contract (both texts normalized as buffer content).
/// `old-count`/`new-count` are `(length old-lines)`/`(length new-lines)` —
/// [`DiffHunk`] carries no separate count field, so the Steel tuple derives
/// them at the boundary rather than duplicating state Rust-side.
///
/// [`DiffHost`]: crate::host::DiffHost
pub(crate) fn diff_lines(ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-lines old-text")?;
    let new = string_arg(new, "diff-lines new-text")?;
    let hunks = require_cap(ctx.host.diff(), "diff-lines")?.diff_lines(&old, &new);
    Ok(hunks_to_steel(hunks))
}

/// `(diff-buffer-lines bid ref-text)` → same shape as `diff-lines`, diffing
/// `ref-text` (old) against `bid`'s live text (new) without round-tripping
/// the whole buffer through Steel.
pub(crate) fn diff_buffer_lines(
    ctx: &mut SteelCtx,
    bid: BidArg,
    ref_text: SteelVal,
) -> SteelResult {
    let ref_text = string_arg(ref_text, "diff-buffer-lines ref-text")?;
    // `DiffHost::diff_buffer_lines` already does the liveness lookup it
    // needs to fetch the buffer's text, so this skips a second one —
    // `not_live_err` keeps the error wording identical to `require_live`'s.
    let hunks = require_cap(ctx.host.diff(), "diff-buffer-lines")?
        .diff_buffer_lines(bid.0, &ref_text)
        .ok_or_else(|| bid.not_live_err("diff-buffer-lines"))?;
    Ok(hunks_to_steel(hunks))
}

fn list_of(items: impl IntoIterator<Item = SteelVal>) -> SteelVal {
    SteelVal::ListV(items.into_iter().collect::<Vec<_>>().into())
}

fn hunks_to_steel(hunks: Vec<DiffHunk>) -> SteelVal {
    list_of(hunks.into_iter().map(hunk_to_steel))
}

fn hunk_to_steel(hunk: DiffHunk) -> SteelVal {
    let old_count = hunk.old_lines.len();
    let new_count = hunk.new_lines.len();
    list_of([
        SteelVal::IntV(hunk.old_start as isize),
        SteelVal::IntV(old_count as isize),
        SteelVal::IntV(hunk.new_start as isize),
        SteelVal::IntV(new_count as isize),
        string_list(hunk.old_lines),
        string_list(hunk.new_lines),
    ])
}

fn string_list(lines: Vec<String>) -> SteelVal {
    list_of(lines.into_iter().map(|s| SteelVal::StringV(s.into())))
}

/// `(diff-words old-text new-text)` → `(hunks . deadline-hit?)`. `hunks` is
/// a list of `(old-start old-end new-start new-end old-text new-text)`
/// tuples, char offsets, `Equal` runs dropped. `deadline-hit?` is `#t` when
/// the underlying Myers pass timed out and returned a coarse result — see
/// [`DiffHost::diff_words`]'s doc for how a caller should react.
///
/// [`DiffHost::diff_words`]: crate::host::DiffHost::diff_words
pub(crate) fn diff_words(ctx: &mut SteelCtx, old: SteelVal, new: SteelVal) -> SteelResult {
    let old = string_arg(old, "diff-words old-text")?;
    let new = string_arg(new, "diff-words new-text")?;
    let (hunks, deadline_hit) = require_cap(ctx.host.diff(), "diff-words")?.diff_words(&old, &new);
    let hunks = list_of(hunks.into_iter().map(word_hunk_to_steel));
    cons_pair(hunks, SteelVal::BoolV(deadline_hit))
}

fn word_hunk_to_steel(hunk: WordDiffHunk) -> SteelVal {
    list_of([
        SteelVal::IntV(hunk.old_start as isize),
        SteelVal::IntV(hunk.old_end as isize),
        SteelVal::IntV(hunk.new_start as isize),
        SteelVal::IntV(hunk.new_end as isize),
        SteelVal::StringV(hunk.old_text.into()),
        SteelVal::StringV(hunk.new_text.into()),
    ])
}

#[cfg(test)]
mod tests;
