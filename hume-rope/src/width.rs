//! Display-column arithmetic — the single source of truth for "how many
//! terminal cells does this text occupy", shared by every crate that renders
//! or aligns text: `hume-engine` (buffer lines, virtual decoration rows),
//! `hume-ops` (tab insert/dedent), and `hume-editor` (popups, pickers, the
//! statusline). Before this module existed each of those forked its own
//! column-counting convention; see the git history around this module's
//! introduction for the drift that caused.
//!
//! Distinct from grapheme *indexing* (`crate::grapheme`, which counts
//! clusters, not cells) and from LSP wire positions
//! (`crate::position_encoding`, which counts UTF-16 code units or bytes).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Columns a `\t` at display column `display_col` occupies — the distance to
/// the next tab stop of width `tw`. Always in `[1, tw]`: a tab already
/// sitting on a stop advances a full `tw` rather than zero. `tw < 1` is
/// clamped to 1 (a zero-width tab stop is meaningless) so callers don't each
/// have to guard it themselves.
pub fn tab_advance(display_col: usize, tw: u8) -> usize {
    let tw = (tw as usize).max(1);
    tw - display_col % tw
}

/// Largest tab stop of width `tw` strictly before `display_col` — the
/// dedent-on-Backspace sibling of [`tab_advance`]. A `display_col` already
/// sitting exactly on a stop still steps back to the *previous* one (never a
/// no-op), matching how `tab_advance` never advances by zero. `0` for
/// `display_col == 0` or anything within the first stop. `tw < 1` is clamped
/// to 1.
pub fn prev_tab_stop(display_col: usize, tw: u8) -> usize {
    let tw = (tw as usize).max(1);
    (display_col.saturating_sub(1) / tw) * tw
}

/// Display columns one grapheme cluster occupies when rendered starting at
/// display column `display_col`. A tab advances to the next `tab_width`
/// stop; every other cluster is measured with `unicode-width` and clamped to
/// `[1, 2]` — the lower bound keeps every cluster occupying at least one
/// cell (so it stays addressable by column even for a degenerate cluster
/// with no base character, e.g. a lone combining mark), the upper bound
/// matches the two-cell layout the renderer gives every wide grapheme.
pub fn grapheme_width(cluster: &str, display_col: usize, tab_width: u8) -> usize {
    if cluster == "\t" {
        tab_advance(display_col, tab_width)
    } else {
        cluster.width().clamp(1, 2)
    }
}

/// Display columns `s` occupies when rendered starting at display column
/// `start_display_col` — the sum of its grapheme clusters' [`grapheme_width`].
pub fn str_width(s: &str, start_display_col: usize, tab_width: u8) -> usize {
    let mut display_col = start_display_col;
    for g in s.graphemes(true) {
        display_col += grapheme_width(g, display_col, tab_width);
    }
    display_col - start_display_col
}

/// Number of indent levels in `line`'s leading whitespace. One indent level
/// is `tab_width` display columns — a run of spaces or a tab stop.
/// `tab_width < 1` is clamped to 1. Leading whitespace is always ASCII
/// (space/tab), so a byte scan is safe and faster than grapheme iteration.
pub fn indent_depth(line: &str, tab_width: u8) -> u8 {
    let tw = (tab_width as usize).max(1);
    let mut display_col = 0usize;
    for b in line.bytes() {
        match b {
            b' ' => display_col += 1,
            b'\t' => display_col += tab_advance(display_col, tab_width),
            _ => break,
        }
    }
    (display_col / tw).min(u8::MAX as usize) as u8
}

/// Longest prefix of `s` whose display width fits within `max_display_width`
/// display columns, and that prefix's width. Never splits a grapheme
/// cluster: one that would overshoot the budget is dropped whole, so the
/// returned width can be strictly less than `max_display_width`.
pub fn truncate_to_width(s: &str, max_display_width: usize, tab_width: u8) -> (&str, usize) {
    let mut display_col = 0usize;
    for (byte_idx, g) in s.grapheme_indices(true) {
        let w = grapheme_width(g, display_col, tab_width);
        if display_col + w > max_display_width {
            return (&s[..byte_idx], display_col);
        }
        display_col += w;
    }
    // Reaching here means every cluster fit, so the whole string is the answer.
    (s, display_col)
}

/// Longest suffix of `s` (kept from the end, dropping leading graphemes)
/// whose display width fits within `max_display_width` display columns, and
/// that suffix's width. Never splits a grapheme cluster.
///
/// Measures back-to-front, accumulating width from the kept end rather than
/// from `s`'s own start — exact for tab-free text or `tab_width == 1` (the
/// UI-chrome convention every caller of this variant uses today). A tab's
/// true expansion depends on what precedes it on screen, which a suffix
/// alone can't know; this doesn't attempt to model that.
pub fn truncate_suffix_to_width(s: &str, max_display_width: usize, tab_width: u8) -> (&str, usize) {
    let mut display_col = 0usize;
    let mut start = s.len();
    for (byte_idx, g) in s.grapheme_indices(true).rev() {
        let w = grapheme_width(g, display_col, tab_width);
        if display_col + w > max_display_width {
            break;
        }
        display_col += w;
        start = byte_idx;
    }
    (&s[start..], display_col)
}

#[cfg(test)]
mod tests;
