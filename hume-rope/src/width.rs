//! Display-column arithmetic — the single source of truth for "how many
//! terminal cells does this text occupy", shared by every crate that renders
//! or aligns text: `hume-engine` (buffer lines, virtual decoration rows),
//! `hume-ops` (tab insert/dedent), and `hume-editor` (popups, pickers, the
//! statusline). A caller measuring or drawing display width goes through
//! this module rather than re-deriving its own tab/placeholder rules —
//! two independent conventions can silently disagree at the exact cells
//! where a tab stop or an unrenderable cluster falls.
//!
//! Distinct from grapheme *indexing* (`crate::grapheme`, which counts
//! clusters, not cells) and from LSP wire positions
//! (`crate::position_encoding`, which counts UTF-16 code units or bytes).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The `tab_width` every caller measuring UI chrome text (a popup, a picker,
/// a menu, the statusline, the minibuffer) passes — chrome has no tab stops
/// of its own, so a tab there is pinned to exactly one cell, same as any
/// other character. Named so that convention is stated once rather than
/// repeated as a bare `1` at every chrome measurement/draw call site.
pub const CHROME_TAB_WIDTH: u8 = 1;

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

/// How a grapheme cluster renders — tab, unrenderable placeholder, or plain
/// text — decided once by [`classify`] and carrying each variant's own
/// display width, so every caller that needs to know not just *how wide* a
/// cluster is but *what to draw* for it reads that off one decision instead
/// of re-deriving it: `format::grapheme_display`, `format::push_virtual_cells`,
/// and `render::write_text_run` all need `cluster == "\t"` tested before
/// [`needs_placeholder`], in that order ([`classify`]'s own doc) — one
/// ordering hazard, checked once instead of at each call site, with no
/// [`Placeholder`] rebuilt after `grapheme_width` already discarded it.
pub enum Cluster {
    /// A tab, expanding to the next `tab_width` stop.
    Tab { width: usize },
    /// A cluster the terminal must not be shown as itself — see
    /// [`needs_placeholder`]. Carries its own [`Placeholder`] so a caller
    /// never has to build it twice.
    Placeholder(Placeholder),
    /// Every other cluster, measured with `unicode-width`. The cluster text
    /// itself isn't carried — every caller already holds it (it's what was
    /// passed to [`classify`]) and reads it from there instead.
    Plain { width: usize },
}

impl Cluster {
    /// Display columns this cluster occupies. Never zero — see
    /// [`grapheme_width`]'s own doc for why.
    pub fn width(&self) -> usize {
        match self {
            Cluster::Tab { width } | Cluster::Plain { width, .. } => *width,
            // A placeholder is ASCII-only ([`Placeholder::as_str`]), so its
            // byte length is also its column count — read directly rather
            // than through `as_str`, which re-validates the bytes as UTF-8.
            Cluster::Placeholder(p) => p.len,
        }
    }
}

/// Classifies `cluster` for rendering at display column `display_col` and
/// measures its width in the same pass — the one decision every caller
/// that draws text (not just measures it) must consume rather than
/// re-derive. Re-deriving it independently is not just duplicated work but
/// a duplicated *ordering* hazard: a tab is also a control character, so
/// testing [`needs_placeholder`] before ruling out `"\t"` would draw a
/// multi-cell placeholder into the single cell [`tab_advance`] reserves
/// for it.
pub fn classify(cluster: &str, display_col: usize, tab_width: u8) -> Cluster {
    if cluster == "\t" {
        return Cluster::Tab {
            width: tab_advance(display_col, tab_width),
        };
    }
    // Printable ASCII — the overwhelming majority of what a source file is,
    // and the one case decidable without measuring: never a control
    // character, never zero-width, always one column. `unicode-width` has no
    // ASCII shortcut of its own (every char walks its lookup tables carrying
    // an emoji-sequence state machine), so this branch is what keeps the
    // common cluster off them.
    if let [0x20..=0x7e] = cluster.as_bytes() {
        return Cluster::Plain { width: 1 };
    }
    // Measured once and shared by both the placeholder test and the plain
    // width, rather than through `needs_placeholder`, which would measure a
    // second time for the plain case.
    let w = cluster.width();
    if placeholder_at_width(cluster, w) {
        Cluster::Placeholder(placeholder(cluster))
    } else {
        Cluster::Plain { width: w.min(2) }
    }
}

/// Display columns one grapheme cluster occupies when rendered starting at
/// display column `display_col`. A tab advances to the next `tab_width`
/// stop; a cluster the terminal must not be shown ([`needs_placeholder`])
/// occupies its [`placeholder`]; every other cluster is measured with
/// `unicode-width`, capped at the two-cell layout the renderer gives a wide
/// grapheme. Nothing measures zero — a cluster that would have needs a
/// placeholder instead, which is never empty.
///
/// A measure-only caller wants this; a caller that also draws the cluster
/// wants [`classify`] instead, so it doesn't re-decide what this function
/// already decided.
pub fn grapheme_width(cluster: &str, display_col: usize, tab_width: u8) -> usize {
    classify(cluster, display_col, tab_width).width()
}

/// True when `cluster` must not be written to the terminal as itself.
///
/// Two disjoint reasons, and a writer has to test for both — they do not
/// imply each other:
///
/// - **It holds a control character.** The backend writes a cell's symbol
///   verbatim, so a literal `\t` would move the terminal's own cursor to its
///   next hardware tab stop and an `ESC` would start an escape sequence out
///   of file content. `unicode-width` measures these as *1* (its rule 7,
///   "all other characters have width 1"), so a zero measure does not catch
///   them.
/// - **It measures zero columns** — a zero-width space, a bare ZWJ, a
///   combining mark with no base character, or a bidi override. Written as
///   itself the terminal advances nothing and the rest of the row slides
///   left of where every display-column computation says it is.
///
/// The second group is `Default_Ignorable_Code_Point`, which is why it also
/// covers the bidi overrides behind Trojan Source (CVE-2021-42574) — the
/// reason these are shown as their codepoint rather than as a blank: a
/// U+202E that renders like a space is exactly the attack.
pub fn needs_placeholder(cluster: &str) -> bool {
    placeholder_at_width(cluster, cluster.width())
}

/// [`needs_placeholder`] for a caller that has already measured `cluster` —
/// `w` must be `cluster.width()`. Split out so [`classify`] can decide from
/// one measurement instead of taking a second one through the predicate.
fn placeholder_at_width(cluster: &str, w: usize) -> bool {
    w == 0 || cluster.chars().any(char::is_control)
}

/// Longest [`placeholder`], `<10ffff>`.
const MAX_PLACEHOLDER: usize = 8;

/// The visible stand-in for a cluster [`needs_placeholder`] rejects: its
/// codepoint in angle-bracket hex, `<200b>`.
///
/// Matches what Vim and Neovim show (`<200b>`, highlighted as `SpecialKey`)
/// and what Emacs's `glyphless-char-display` shows on a text terminal
/// (`[200B]`). Showing the codepoint rather than a generic marker is what
/// lets a reader tell a harmless zero-width space from a bidi override.
///
/// Inline stack storage, no allocation — this is on the per-cell render
/// path. A degenerate cluster of more than one such character reports its
/// first; nothing in practice produces one.
pub struct Placeholder {
    buf: [u8; MAX_PLACEHOLDER],
    len: usize,
}

impl Placeholder {
    pub fn as_str(&self) -> &str {
        // Only ASCII is ever written below, so the slice is valid UTF-8 and
        // its byte length is also its display width.
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("<?>")
    }
}

impl std::fmt::Write for Placeholder {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        if end > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// The [`Placeholder`] for `cluster`. Meaningful only when
/// [`needs_placeholder`] is true of it.
pub fn placeholder(cluster: &str) -> Placeholder {
    use std::fmt::Write as _;
    let codepoint = cluster.chars().next().map_or(0, u32::from);
    let mut out = Placeholder {
        buf: [0; MAX_PLACEHOLDER],
        len: 0,
    };
    // Cannot overflow the buffer: a codepoint is at most `10ffff`, six hex
    // digits between the two brackets.
    let _ = write!(out, "<{codepoint:x}>");
    out
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

/// Display column of the `k`-th indent stop (`k` indent levels of
/// `tab_width` display columns each) — the inverse of [`indent_depth`].
/// `tab_width < 1` is clamped to 1, the same clamp `indent_depth` applies, so
/// the two can never disagree about what one indent level is worth.
pub fn indent_stop(k: u32, tab_width: u8) -> u32 {
    k * (tab_width as u32).max(1)
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
/// from `s`'s own start — exact for tab-free text, which is what every
/// caller of this function has: chrome text, always measured at
/// [`CHROME_TAB_WIDTH`], the only tab width this function knows. A tab's
/// true expansion depends on what precedes it on screen, which a suffix
/// alone can't know; this doesn't attempt to model that, which is why the
/// convention is fixed rather than a parameter a caller could pass some
/// other width to and get a silently wrong answer.
pub fn truncate_suffix_to_width(s: &str, max_display_width: usize) -> (&str, usize) {
    let mut display_col = 0usize;
    let mut start = s.len();
    for (byte_idx, g) in s.grapheme_indices(true).rev() {
        let w = grapheme_width(g, display_col, CHROME_TAB_WIDTH);
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
