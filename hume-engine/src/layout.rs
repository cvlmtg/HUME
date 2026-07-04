use std::ops::Range;

use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::pane::{ViewportState, WrapMode};
use crate::providers::GutterColumn;

// ---------------------------------------------------------------------------
// Visible range — output of Stage 1
// ---------------------------------------------------------------------------

/// The output of the Layout stage: which buffer lines are visible and what
/// geometry the Format/Render stages should use.
#[derive(Debug, Clone)]
pub struct VisibleRange {
    /// Buffer lines to format (may extend slightly past the visible area for
    /// smooth-scroll look-ahead; the Render stage clips to `content_height`).
    pub line_range: Range<usize>,
    /// How many display rows to skip from the top of `line_range.start` when
    /// the viewport begins partway through a wrapped line.
    pub top_skip_rows: u16,
    /// Available display rows in the content area.
    pub content_height: u16,
    /// Available columns in the content area (viewport width − gutter width).
    pub content_width: u16,
    /// Total gutter width in columns.
    pub gutter_width: u16,
    /// 0-based index of the last buffer line (`rope.len_lines() - 1`).
    /// This is the correct value to pass to `GutterColumn::width()`.
    pub last_line_idx: usize,
}

// ---------------------------------------------------------------------------
// Stage 1: compute_viewport
// ---------------------------------------------------------------------------

/// Sum of all gutter column widths for the given `max_line` (0-based last line index).
///
/// Pass the last line index of the entire file so gutter width is stable across scrolling.
///
/// Takes an iterator (rather than a slice) so callers can feed it
/// `ProviderSet::gutter_columns()` directly — `ProviderSet` stores
/// `(ProviderId, Box<dyn GutterColumn>)` pairs internally (to support
/// `ProviderSet::remove`), which isn't a shape this purely-arithmetic
/// function needs to know about.
pub fn gutter_width_for_line<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    max_line: usize,
) -> u16 {
    gutter_columns.map(|c| c.width(max_line) as u16).sum()
}

/// Compute the `VisibleRange` for a pane given its current state.
///
/// `tab_width` feeds the wrapping-mode line-row estimate (see
/// `estimate_line_rows`) — it has no effect in `WrapMode::None`.
///
/// **Precondition**: `rope` must end with a structural `\n` (the editor's
/// buffer invariant — see project `CLAUDE.md`), except when it is empty.
/// The phantom-trailing-line exclusion below assumes ropey's one extra
/// reported line past `last_line_idx` is that trailing newline's empty
/// remainder; without it, the rope's actual last content line is dropped
/// from `line_range` entirely. Not enforced with a hard error — this
/// crate's own unit tests build ropes without the invariant to test
/// formatting in isolation — but checked with a `debug_assert` so a
/// violation is caught in debug builds instead of silently rendering
/// short.
///
/// This is purely arithmetic — no heap allocations.
pub fn compute_viewport<'a>(
    rope: &Rope,
    viewport: &ViewportState,
    wrap_mode: &WrapMode,
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    tab_width: u8,
) -> VisibleRange {
    debug_assert!(
        rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) == '\n',
        "compute_viewport requires a trailing '\\n' (the buffer invariant) — \
         without it the rope's last content line is dropped from line_range"
    );

    let total_lines = rope.len_lines();
    // 0-based index of the last line — the single source of truth for GutterColumn::width().
    // Using the whole-file last line (not just the visible range) keeps gutter width stable
    // as the user scrolls.
    let last_line_idx = total_lines.saturating_sub(1);
    let gutter_width = gutter_width_for_line(gutter_columns, last_line_idx);

    let content_width = viewport.width.saturating_sub(gutter_width).max(1);

    // Compute buffer line range that fills the viewport.
    let top_line = viewport.top_line.min(last_line_idx.saturating_sub(1));
    let top_skip = viewport.top_row_offset;

    // Exclude the phantom trailing line. The buffer invariant guarantees a
    // trailing `\n`, so ropey always reports one extra empty line at index
    // `last_line_idx`. Real content is lines 0..last_line_idx (exclusive), so
    // `last_line_idx` is the correct exclusive upper bound for the range.
    let line_range = compute_line_range(
        rope,
        top_line,
        top_skip,
        viewport.height,
        content_width,
        wrap_mode,
        last_line_idx,
        tab_width,
    );

    VisibleRange {
        line_range,
        top_skip_rows: top_skip,
        content_height: viewport.height,
        content_width,
        gutter_width,
        last_line_idx,
    }
}

/// Determine which buffer lines need to be formatted to fill `viewport_height`
/// rows, starting from `top_line` with `top_skip` rows already scrolled past.
///
/// `last_line_idx` is the 0-based index of the last real content line (i.e.
/// `rope.len_lines() - 1`). It is used as an exclusive upper bound for the
/// returned range because the phantom trailing line at that index must not be
/// included.
#[allow(clippy::too_many_arguments)]
fn compute_line_range(
    rope: &Rope,
    top_line: usize,
    top_skip: u16,
    viewport_height: u16,
    content_width: u16,
    wrap_mode: &WrapMode,
    last_line_idx: usize,
    tab_width: u8,
) -> Range<usize> {
    // For non-wrapping mode each buffer line is exactly one display row.
    if !wrap_mode.is_wrapping() {
        // top_skip is always 0 for non-wrapping (no wrapped lines).
        let end = (top_line + viewport_height as usize).min(last_line_idx);
        return top_line..end;
    }

    // For wrapping modes: count rows per line until we have filled the viewport.
    // `top_skip` rows have been consumed from `top_line` already.
    let mut rows_needed = viewport_height as usize + top_skip as usize;
    let mut end = top_line;

    while end < last_line_idx && rows_needed > 0 {
        let line_rows = estimate_line_rows(rope, end, content_width, tab_width);
        rows_needed = rows_needed.saturating_sub(line_rows);
        end += 1;
    }

    // Add a small look-ahead so smooth scrolling has room. Word/Indent wrap
    // can still exceed this estimate (a short word wastes columns) — that
    // residual error is what the look-ahead buffers against. Exact counting
    // would mean running the formatter per line, rejected: this stage is
    // documented as purely arithmetic, no allocations.
    const LOOKAHEAD_LINES: usize = 4;
    let end = (end + LOOKAHEAD_LINES).min(last_line_idx);
    top_line..end
}

/// Cheaply estimate how many display rows a buffer line occupies when wrapped
/// to `content_width`.
///
/// Width-aware: sums `unicode_width` display width over the line's chunks,
/// swapping each tab's 1-column placeholder width for a full `tab_width`
/// expansion — a deliberate upper bound, since it ignores the tab's actual
/// column position, so it can only over-count, never under-count (safe for a
/// look-ahead bound). Character count alone (the previous proxy) undercounts
/// CJK (2 columns) and tabs (up to `tab_width` columns); overestimates here
/// are harmless — the extra lines get formatted, then clipped by the Render
/// stage.
fn estimate_line_rows(rope: &Rope, line_idx: usize, content_width: u16, tab_width: u8) -> usize {
    if content_width == 0 {
        return 1;
    }
    let tab_width = tab_width.max(1) as usize;
    let line = rope.line(line_idx);
    let mut total_width = 0usize;
    for chunk in line.chunks() {
        let tab_count = chunk.matches('\t').count();
        // `unicode_width` reports width 1 (not 0) for '\t' in this crate
        // version — subtract that placeholder count back out before adding
        // the real tab-stop expansion, so each tab contributes exactly
        // `tab_width` columns to the estimate (a deliberate upper bound: it
        // ignores the tab's actual column position, so it can only
        // over-count, never under-count).
        total_width += chunk.width() - tab_count + tab_count * tab_width;
    }
    if total_width == 0 {
        1
    } else {
        total_width.div_ceil(content_width as usize).max(1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::GutterCell;
    use crate::providers::GutterColumn;
    use crate::providers::GutterRowCtx;
    use crate::types::{RowKind, Scope};

    struct _NoGutter;
    impl GutterColumn for _NoGutter {
        fn width(&self, _: usize) -> u8 {
            0
        }
        fn render_row(&self, _: RowKind, _: &GutterRowCtx) -> GutterCell {
            GutterCell::blank(Scope("ui.linenr"))
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn no_wrap_basic_range() {
        let rope = Rope::from_str("line1\nline2\nline3\nline4\nline5\n");
        let viewport = ViewportState::new(80, 3);
        let visible = compute_viewport(&rope, &viewport, &WrapMode::None, std::iter::empty(), 4);
        assert_eq!(visible.line_range.start, 0);
        assert!(visible.line_range.end <= 5);
        assert_eq!(visible.gutter_width, 0);
        assert_eq!(visible.content_width, 80);
    }

    #[test]
    fn no_wrap_clamped_to_total_lines() {
        // Trailing '\n' per the buffer invariant (see compute_viewport's doc
        // comment, B12) — without it the rope's one real content line would
        // be excluded from line_range as if it were the phantom trailing line.
        let rope = Rope::from_str("only one line\n");
        let viewport = ViewportState::new(80, 50);
        let visible = compute_viewport(&rope, &viewport, &WrapMode::None, std::iter::empty(), 4);
        // 2 ropey lines (content + phantom trailing); line_range excludes
        // the phantom, so it must stop at 1 despite the 50-row viewport.
        assert_eq!(visible.line_range, 0..1);
    }

    #[test]
    fn soft_wrap_includes_lookahead() {
        let rope = Rope::from_str("a\nb\nc\nd\ne\nf\ng\n");
        let viewport = ViewportState::new(80, 3);
        let visible = compute_viewport(
            &rope,
            &viewport,
            &WrapMode::Soft { width: 80 },
            std::iter::empty(),
            4,
        );
        // Should have at least 3 + lookahead lines
        assert!(visible.line_range.len() >= 3);
    }

    // ── estimate_line_rows width-awareness (B6) ─────────────────────────

    #[test]
    fn estimate_line_rows_cjk_width_aware() {
        // 40 '中' chars, each display width 2 → true width 80. At
        // content_width 40 that's exactly 2 rows. Char-count-only (the old
        // proxy) would see 40 chars / 40 cols = 1 row — half the true count.
        let text: String = "中".repeat(40);
        let rope = Rope::from_str(&text);
        assert_eq!(estimate_line_rows(&rope, 0, 40, 4), 2);
    }

    #[test]
    fn estimate_line_rows_tabs_width_aware() {
        // 10 tabs, tab_width 4 → each tab contributes a full 4-column
        // expansion (its 1-column `unicode_width` placeholder is swapped for
        // the real tab-stop width) → true width 40. At content_width 20
        // that's exactly 2 rows. Char-count-only would see 10 chars / 20
        // cols = 1 row.
        let text = "\t".repeat(10);
        let rope = Rope::from_str(&text);
        assert_eq!(estimate_line_rows(&rope, 0, 20, 4), 2);
    }

    #[test]
    fn estimate_line_rows_empty_line_is_one() {
        let rope = Rope::from_str("");
        assert_eq!(estimate_line_rows(&rope, 0, 40, 4), 1);
    }

    #[test]
    fn estimate_line_rows_zero_content_width_is_one() {
        let rope = Rope::from_str("abc");
        assert_eq!(estimate_line_rows(&rope, 0, 0, 4), 1);
    }
}
