//! `(diff-lines old-text new-text)` / `(diff-buffer-lines bid ref-text)` /
//! `(diff-words old-text new-text)` — native line and word diff, exposed to
//! Steel plugins (Phase 2a/2b, `docs/GIT-DIFF.md`).

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{DiffHunk, WordDiffHunk};

use super::SteelResult;
use super::args::{BidArg, cons_pair, string_arg};
use super::errors::{generic_err, require_cap};

/// `(diff-lines old-text new-text)` → list of hunk tuples, oldest side
/// first. Each hunk is `(old-start old-count new-start new-count old-lines
/// new-lines)`, 0-based; `Equal` runs are dropped. See [`DiffHost`]'s doc
/// for the exact contract (both texts normalized as buffer content).
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
    let id = bid.require_live(ctx, "diff-buffer-lines")?;
    let ref_text = string_arg(ref_text, "diff-buffer-lines ref-text")?;
    let hunks = require_cap(ctx.host.diff(), "diff-buffer-lines")?
        .diff_buffer_lines(id, &ref_text)
        .ok_or_else(|| generic_err("diff-buffer-lines: unknown buffer"))?;
    Ok(hunks_to_steel(hunks))
}

fn hunks_to_steel(hunks: Vec<DiffHunk>) -> SteelVal {
    let list: Vec<SteelVal> = hunks.into_iter().map(hunk_to_steel).collect();
    SteelVal::ListV(list.into())
}

fn hunk_to_steel(hunk: DiffHunk) -> SteelVal {
    let fields = vec![
        SteelVal::IntV(hunk.old_start as isize),
        SteelVal::IntV(hunk.old_count as isize),
        SteelVal::IntV(hunk.new_start as isize),
        SteelVal::IntV(hunk.new_count as isize),
        string_list(hunk.old_lines),
        string_list(hunk.new_lines),
    ];
    SteelVal::ListV(fields.into())
}

fn string_list(lines: Vec<String>) -> SteelVal {
    let list: Vec<SteelVal> = lines
        .into_iter()
        .map(|s| SteelVal::StringV(s.into()))
        .collect();
    SteelVal::ListV(list.into())
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
    cons_pair(word_hunks_to_steel(hunks), SteelVal::BoolV(deadline_hit))
}

fn word_hunks_to_steel(hunks: Vec<WordDiffHunk>) -> SteelVal {
    let list: Vec<SteelVal> = hunks.into_iter().map(word_hunk_to_steel).collect();
    SteelVal::ListV(list.into())
}

fn word_hunk_to_steel(hunk: WordDiffHunk) -> SteelVal {
    let fields = vec![
        SteelVal::IntV(hunk.old_start as isize),
        SteelVal::IntV(hunk.old_end as isize),
        SteelVal::IntV(hunk.new_start as isize),
        SteelVal::IntV(hunk.new_end as isize),
        SteelVal::StringV(hunk.old_text.into()),
        SteelVal::StringV(hunk.new_text.into()),
    ];
    SteelVal::ListV(fields.into())
}

#[cfg(test)]
mod tests;
