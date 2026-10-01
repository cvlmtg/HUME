use regex_cursor::engines::meta::Regex;

use crate::MotionMode;
use crate::search::find_matches_in_range;
use crate::text_object::trim_blank;
use hume_editing::selection::{Facing, Selection};
use hume_editing::state::EditState;
use hume_rope::cluster::ClusterRange;
use hume_rope::offset::ExclusiveRange;

// ── Split on newlines ─────────────────────────────────────────────────────────

/// Split each multi-line selection into one selection per line.
///
/// Single-line selections are left unchanged. For a selection spanning lines
/// L1..L2:
/// - Line L1: from the selection's start to the last non-`\n` char on L1
///   (or the `\n` itself if the line is empty).
/// - Lines L1+1..L2-1: full lines from start to last non-`\n` char.
/// - Line L2: from the line start to the selection's end.
///
/// The direction (forward/backward) of the original selection is preserved on
/// every piece. The primary becomes the first piece of the original primary.
pub fn cmd_split_selection_on_newlines(
    state: EditState,
    _count: usize,
    _mode: MotionMode,
) -> EditState {
    state.flat_map(|sel| {
        let text = sel.text();
        let lines = sel.lines();
        if lines.start == lines.end {
            return vec![sel.selection()];
        }
        let facing = sel.facing();
        let piece = |first, last| {
            let range = ClusterRange::through(text.full_slice(), first, last)
                .expect("a piece's first cluster precedes its last");
            Selection::covering(range, facing)
        };

        // First line piece: from the selection start to the end of the line
        // content, or the start alone when it is the line's `\n`.
        let mut pieces = vec![piece(
            sel.start(),
            text.lines().content_end(lines.start).max(sel.start()),
        )];
        // Middle lines: full lines.
        pieces.extend(
            ExclusiveRange::new(lines.start.advance(1), lines.end)
                .iter()
                .map(|line| piece(text.lines().start(line), text.lines().content_end(line))),
        );
        // Last line piece: from the line start to the selection's last cluster.
        pieces.push(piece(text.lines().start(lines.end), sel.last()));
        pieces
    })
}

// ── Sift matches within ────────────────────────────────────────────────────────

/// Replace each selection with the regex matches found within it.
///
/// For every selection in `state`, finds all non-overlapping matches of `regex`
/// bounded to that selection's range. Each match becomes a new forward
/// `Selection`. The new primary is the first match within the original primary
/// selection's range.
///
/// Returns `None` when no matches are found in any selection; the caller
/// should keep the original selections unchanged.
pub fn sift_matches_within(state: &EditState, regex: &Regex) -> Option<EditState> {
    let mut matches: Vec<Vec<Selection>> = state
        .view()
        .iter()
        .map(|sel| {
            find_matches_in_range(sel.text(), regex, sel.covered())
                .into_iter()
                .map(|span| Selection::covering(span, Facing::Forward))
                .collect()
        })
        .collect();
    if matches.iter().all(Vec::is_empty) {
        return None;
    }
    Some(
        state
            .clone()
            .flat_map(|sel| std::mem::take(&mut matches[sel.index()])),
    )
}

// ── Trim whitespace ───────────────────────────────────────────────────────────

/// Trim leading and trailing whitespace from every selection's range.
///
/// "Whitespace" here means space (` `), tab (`\t`), and newline (`\n`). The
/// range shrinks inward until both ends sit on non-whitespace characters. If
/// the entire selection is whitespace the selection collapses to a cursor at
/// the original `head`.
pub fn cmd_trim_selection_whitespace(
    state: EditState,
    _count: usize,
    _mode: MotionMode,
) -> EditState {
    state.map(|sel| match trim_blank(sel.text(), sel.covered()) {
        Some(range) => Selection::covering(range, sel.facing()),
        None => sel.selection().to_head(),
    })
}

#[cfg(test)]
mod tests;
