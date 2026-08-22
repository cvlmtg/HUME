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

#[cfg(test)]
mod tests;
