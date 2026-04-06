use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::layout::VisibleRange;
use crate::pane::{WrapMode, WhitespaceConfig, WhitespaceRender};
use crate::providers::{InlineDecoration, InlineInsert, VirtualLine, VirtualLineAnchor, VirtualLineSource};
use crate::types::{CellContent, DisplayRow, Grapheme, RowKind};

// ---------------------------------------------------------------------------
// Scratch storage
// ---------------------------------------------------------------------------

/// Reusable scratch buffers for the Format stage (Stage 2).
///
/// Owned by [`crate::pipeline::FrameScratch`] so capacity is retained across
/// frames — no heap allocation after the first frame warms up the `Vec`s.
pub struct FormatScratch {
    /// `DisplayRow`s produced for the current buffer line (or all visible lines
    /// in the batch path). Cleared per line (fused) or per frame (batch).
    pub display_rows: Vec<DisplayRow>,
    /// `Grapheme`s for the current buffer line; rows index into this.
    pub graphemes: Vec<Grapheme>,
    /// Virtual lines collected from all providers for the visible range.
    pub virtual_lines: Vec<VirtualLine>,
    /// Pre-materialised line text. In the fused path this holds one line at a
    /// time; in the batch path it accumulates all visible lines end-to-end.
    pub line_texts: String,
    /// Start offset of each line within `line_texts` (batch path only).
    pub line_text_offsets: Vec<usize>,
}

impl FormatScratch {
    pub fn new() -> Self {
        Self {
            display_rows: Vec::with_capacity(16),
            graphemes: Vec::with_capacity(512),
            virtual_lines: Vec::new(),
            line_texts: String::with_capacity(512),
            line_text_offsets: Vec::new(),
        }
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.display_rows.clear();
        self.graphemes.clear();
        self.virtual_lines.clear();
        self.line_texts.clear();
        self.line_text_offsets.clear();
    }
}

impl Default for FormatScratch {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Format all visible buffer lines into `DisplayRow`s and `Grapheme`s.
///
/// `inline_inserts` is a caller-supplied scratch buffer reused across lines.
/// `scratch` accumulates all output; call `scratch.clear()` before each frame.
/// No heap allocations occur after the first frame warms up the `Vec`s.
#[allow(clippy::too_many_arguments)]
pub fn format_lines(
    rope: &Rope,
    visible: &VisibleRange,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    inline_providers: &[Box<dyn InlineDecoration>],
    virtual_providers: &[Box<dyn VirtualLineSource>],
    inline_inserts: &mut Vec<InlineInsert>,
    scratch: &mut FormatScratch,
) {
    // Pre-collect virtual lines from all providers so we can splice them in
    // order during the line loop without re-querying each iteration.
    scratch.virtual_lines.clear();
    for provider in virtual_providers {
        provider.virtual_lines(visible.line_range.clone(), visible.content_width, &mut scratch.virtual_lines);
    }
    // Sort by anchor for O(n) merging during the line loop.
    scratch.virtual_lines.sort_by_key(|vl| vl.anchor.sort_key());

    let mut vl_cursor = 0usize;

    for line_idx in visible.line_range.clone() {
        // ── Inject Before-anchored virtual lines ──────────────────────────
        while vl_cursor < scratch.virtual_lines.len() {
            if scratch.virtual_lines[vl_cursor].anchor == VirtualLineAnchor::Before(line_idx) {
                let vl = &scratch.virtual_lines[vl_cursor];
                emit_virtual_row(vl, line_idx, &mut scratch.display_rows, &mut scratch.graphemes);
                vl_cursor += 1;
            } else {
                break;
            }
        }

        // ── Format the buffer line (or emit a Filler row past EOF) ────────
        // Record where this line's text starts in `line_texts` — even for filler
        // rows the offset is pushed so `line_text_offsets[i]` always corresponds
        // to `visible.line_range.start + i` and filler entries have zero length.
        scratch.line_text_offsets.push(scratch.line_texts.len());

        if line_idx >= rope.len_lines() {
            scratch.display_rows.push(DisplayRow {
                kind: RowKind::Filler,
                graphemes: scratch.graphemes.len()..scratch.graphemes.len(),
            });
        } else {
            // Collect inline inserts for this line.
            inline_inserts.clear();
            for provider in inline_providers {
                provider.decorations_for_line(line_idx, inline_inserts);
            }
            inline_inserts.sort_by_key(|i| i.byte_offset);

            format_buffer_line(
                rope,
                line_idx,
                tab_width,
                whitespace,
                wrap_mode,
                inline_inserts,
                scratch,
            );
        }

        // ── Inject After-anchored virtual lines ───────────────────────────
        while vl_cursor < scratch.virtual_lines.len() {
            if scratch.virtual_lines[vl_cursor].anchor == VirtualLineAnchor::After(line_idx) {
                let vl = &scratch.virtual_lines[vl_cursor];
                emit_virtual_row(vl, line_idx, &mut scratch.display_rows, &mut scratch.graphemes);
                vl_cursor += 1;
            } else {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Virtual row emission
// ---------------------------------------------------------------------------

fn emit_virtual_row(
    vl: &VirtualLine,
    anchor_line: usize,
    display_rows: &mut Vec<DisplayRow>,
    graphemes: &mut Vec<Grapheme>,
) {
    let g_start = graphemes.len();
    graphemes.extend_from_slice(&vl.graphemes);
    display_rows.push(DisplayRow {
        kind: RowKind::Virtual { provider_id: vl.provider_id, anchor_line },
        graphemes: g_start..graphemes.len(),
    });
}

// ---------------------------------------------------------------------------
// Buffer line formatting
// ---------------------------------------------------------------------------

/// Return the number of display rows that `line_idx` occupies when formatted.
///
/// Convenience wrapper for external crates (e.g. the editor's scroll logic)
/// that need to count visual rows without using `FormatScratch` directly for
/// all four pipeline stages.
///
/// The scratch buffers are cleared before use; the caller may treat `scratch`
/// as dirty after this call.
pub fn count_visual_rows(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    scratch: &mut FormatScratch,
) -> usize {
    scratch.display_rows.clear();
    scratch.graphemes.clear();
    scratch.line_texts.clear();
    format_buffer_line(rope, line_idx, tab_width, whitespace, wrap_mode, &[], scratch);
    scratch.display_rows.len()
}

/// Format one buffer line, appending zero or more `DisplayRow`s.
pub fn format_buffer_line(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    inline_inserts: &[InlineInsert],
    scratch: &mut FormatScratch,
) {
    // Append this rope line's text to the persistent `line_texts` buffer.
    // The caller already recorded `line_texts.len()` as the start offset for
    // this line, so we just extend from here. Rope chunks are valid UTF-8.
    let text_start = scratch.line_texts.len();
    let line_slice = rope.line(line_idx);
    for chunk in line_slice.chunks() {
        scratch.line_texts.push_str(chunk);
    }
    // Strip the trailing newline that ropey includes for non-final lines.
    let had_newline = scratch.line_texts.ends_with('\n');
    strip_line_ending(&mut scratch.line_texts);

    let line_str = &scratch.line_texts[text_start..];

    // Detect whether the entire line is whitespace (used for Trailing rendering).
    let line_is_blank = line_str.trim().is_empty();
    let indent_depth = compute_indent_depth(line_str, tab_width);

    let wrap_width = wrap_mode.wrap_width().unwrap_or(u16::MAX); // u16::MAX = sentinel for "no wrap"
    // For indent-wrap, continuation rows start at this column.
    let indent_cols: u16 = if matches!(wrap_mode, WrapMode::Indent { .. }) {
        (indent_depth as u16) * (tab_width as u16)
    } else {
        0
    };

    // ── Row / column state ────────────────────────────────────────────────
    // Aliases into the scratch buffers so the rest of the function can use
    // the original `rows_out` / `graphemes_out` names without further changes.
    let rows_out = &mut scratch.display_rows;
    let graphemes_out = &mut scratch.graphemes;

    let mut insert_idx = 0usize;
    let mut wrap = WrapState {
        current_col: 0,
        wrap_row: 0,
        row_g_start: graphemes_out.len(),
        // Word-wrap state: remember the last whitespace position in the current row.
        last_ws_g_idx: graphemes_out.len(), // grapheme index of last ws boundary
        last_ws_was_set: false,
    };

    // Push the first row.
    rows_out.push(DisplayRow {
        kind: RowKind::LineStart { line_idx },
        graphemes: wrap.row_g_start..0, // closed later
    });

    let mut in_leading_ws = true;
    let mut had_non_ws = false;

    // Running absolute char position within the buffer. Populated per grapheme
    // so the style stage can resolve selection positions without rope lookups.
    let mut char_pos = rope.line_to_char(line_idx);

    for (byte_offset, grapheme_str) in line_str.grapheme_indices(true) {
        // ── Inject inline inserts before this byte offset ─────────────────
        while insert_idx < inline_inserts.len()
            && inline_inserts[insert_idx].byte_offset <= byte_offset
        {
            let ins = &inline_inserts[insert_idx];
            let ins_width = unicode_display_width(ins.text) as u8;
            if ins_width > 0 {
                wrap.maybe_wrap(ins_width, wrap_width, indent_cols, line_idx, indent_depth, rows_out, graphemes_out);
                graphemes_out.push(Grapheme {
                    byte_range: byte_offset..byte_offset, // zero-length: virtual
                    // Inline inserts have no buffer char; use sentinel so style stage skips them.
                    char_offset: usize::MAX,
                    col: wrap.current_col,
                    width: ins_width,
                    content: CellContent::Virtual(ins.text),
                    indent_depth,
                });
                wrap.current_col += ins_width as u16;
            }
            insert_idx += 1;
        }

        // ── Skip newlines (line_str is already stripped; this guards edge cases) ──
        if grapheme_str == "\n" {
            continue;
        }
        // NOTE: newline indicator is emitted after the main loop, below.

        // ── Update leading-ws flag ─────────────────────────────────────────
        let is_ws = is_whitespace_grapheme(grapheme_str);
        if !is_ws {
            if in_leading_ws {
                in_leading_ws = false;
            }
            had_non_ws = true;
        }

        // ── Compute display width and content ─────────────────────────────
        let (width, content) = grapheme_display(
            grapheme_str,
            wrap.current_col,
            tab_width,
            whitespace,
            in_leading_ws,
            had_non_ws,
            line_is_blank,
        );

        // ── Wrap if necessary ─────────────────────────────────────────────
        wrap.maybe_wrap(width, wrap_width, indent_cols, line_idx, indent_depth, rows_out, graphemes_out);

        // ── Track word-break position ─────────────────────────────────────
        if is_ws && !in_leading_ws {
            wrap.last_ws_g_idx = graphemes_out.len(); // next grapheme will be after the ws
            wrap.last_ws_was_set = true;
        }

        // ── Emit grapheme ─────────────────────────────────────────────────
        let char_count = grapheme_str.chars().count();
        graphemes_out.push(Grapheme {
            byte_range: byte_offset..byte_offset + grapheme_str.len(),
            char_offset: char_pos,
            col: wrap.current_col,
            width,
            content,
            indent_depth,
        });
        char_pos += char_count;
        wrap.current_col += width as u16;

        // For CJK (width == 2): emit a WidthContinuation placeholder so the
        // render stage knows not to write anything to the second cell.
        if width == 2 {
            // Both cells of a double-wide char always stay on the same row.
            // Backing up the primary to avoid overflow is not yet implemented.
            graphemes_out.push(Grapheme {
                byte_range: byte_offset..byte_offset + grapheme_str.len(),
                // Same char as the primary cell — this is not a distinct buffer position.
                char_offset: char_pos - char_count,
                col: wrap.current_col,
                width: 0, // zero — does not consume columns
                content: CellContent::WidthContinuation,
                indent_depth,
            });
        }
    }

    // ── Empty-line sentinel ────────────────────────────────────────────────
    // If no graphemes were emitted (truly empty line, e.g. just "\n"), push a
    // width-1 Empty cell at col 0. This gives the selection head a grapheme to land on
    // in the Style stage — without it the selection head is invisible on empty lines.
    if graphemes_out.len() == wrap.row_g_start {
        graphemes_out.push(Grapheme {
            byte_range: 0..0,
            // char_pos is line_to_char(line_idx) — the cursor sits on the newline char.
            char_offset: char_pos,
            col: 0,
            width: 1,
            content: CellContent::Empty,
            indent_depth: 0,
        });
    }

    // ── Emit any trailing inline inserts ──────────────────────────────────
    for ins in &inline_inserts[insert_idx..] {
        let ins_width = unicode_display_width(ins.text) as u8;
        if ins_width > 0 {
            graphemes_out.push(Grapheme {
                byte_range: line_str.len()..line_str.len(),
                char_offset: usize::MAX, // virtual, no buffer char
                col: wrap.current_col,
                width: ins_width,
                content: CellContent::Virtual(ins.text),
                indent_depth,
            });
            wrap.current_col += ins_width as u16;
        }
    }

    // ── Newline indicator ──────────────────────────────────────────────────
    // Emitted at the end of the line (after all content and trailing inserts)
    // on the last wrap row. `in_leading_ws` and `had_non_ws` reflect the
    // state after iterating all graphemes, so `should_render_whitespace` sees
    // the correct context for Trailing vs All rules.
    if had_newline && should_render_whitespace(&whitespace.newline, in_leading_ws, had_non_ws, line_is_blank) {
        graphemes_out.push(Grapheme {
            byte_range: line_str.len()..line_str.len(),
            char_offset: usize::MAX, // whitespace indicator, no buffer char
            col: wrap.current_col,
            width: 1,
            content: CellContent::Indicator(whitespace.newline_char),
            indent_depth,
        });
    }

    // Close the last row.
    close_current_row(rows_out, graphemes_out, wrap.row_g_start);
}

// ---------------------------------------------------------------------------
// Wrap state
// ---------------------------------------------------------------------------

/// Mutable state for the word-wrap / soft-wrap pass inside `format_buffer_line`.
///
/// Grouping these five fields avoids passing them as separate `&mut` parameters
/// through `maybe_wrap`, which previously required 12 parameters total.
struct WrapState {
    current_col: u16,
    wrap_row: u16,
    /// Index into `graphemes_out` where the current display row began.
    row_g_start: usize,
    /// Grapheme index of the last seen whitespace boundary (for word-wrap backtracking).
    last_ws_g_idx: usize,
    last_ws_was_set: bool,
}

impl WrapState {
    /// If adding `width` columns to `current_col` would overflow `wrap_width`,
    /// close the current row and start a new one. Implements word-wrap
    /// backtracking: if `last_ws_was_set`, the row splits at the last whitespace
    /// position.
    #[allow(clippy::too_many_arguments)]
    fn maybe_wrap(
        &mut self,
        width: u8,
        wrap_width: u16,
        indent_cols: u16,
        line_idx: usize,
        indent_depth: u8,
        rows_out: &mut Vec<DisplayRow>,
        graphemes_out: &mut Vec<Grapheme>,
    ) {
        if wrap_width == u16::MAX || self.current_col + width as u16 <= wrap_width {
            return;
        }
        if self.current_col == 0 {
            // Single grapheme wider than the viewport — emit it anyway to avoid
            // an infinite loop. (This can happen with very wide tab stops.)
            return;
        }

        // Determine split point for word wrap.
        let split_at = if self.last_ws_was_set && self.last_ws_g_idx > self.row_g_start {
            self.last_ws_g_idx
        } else {
            graphemes_out.len() // soft break: split here
        };

        // Close current row at split_at.
        close_row_at(rows_out, self.row_g_start, split_at);

        // Start new row.
        self.wrap_row += 1;
        self.row_g_start = split_at;
        self.last_ws_was_set = false;

        // Recalculate `current_col` for graphemes in [split_at..] on the new row.
        let mut new_col = indent_cols;
        for g in &mut graphemes_out[split_at..] {
            g.col = new_col;
            g.indent_depth = indent_depth;
            new_col += g.width as u16;
        }
        self.current_col = new_col;
        self.last_ws_g_idx = split_at;

        rows_out.push(DisplayRow {
            kind: RowKind::Wrap { line_idx, wrap_row: self.wrap_row },
            graphemes: self.row_g_start..0, // closed later
        });
    }
}

// ---------------------------------------------------------------------------
// Row closing helpers
// ---------------------------------------------------------------------------

fn close_current_row(rows_out: &mut Vec<DisplayRow>, graphemes_out: &[Grapheme], row_g_start: usize) {
    if let Some(row) = rows_out.last_mut() {
        row.graphemes = row_g_start..graphemes_out.len();
    }
}

fn close_row_at(rows_out: &mut Vec<DisplayRow>, row_g_start: usize, split_at: usize) {
    if let Some(row) = rows_out.last_mut() {
        row.graphemes = row_g_start..split_at;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn is_whitespace_grapheme(s: &str) -> bool {
    s == " " || s == "\t"
}

// ---------------------------------------------------------------------------
// Grapheme display computation
// ---------------------------------------------------------------------------

/// Compute the display `width` and `CellContent` for one grapheme cluster.
fn grapheme_display(
    grapheme_str: &str,
    current_col: u16,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    in_leading_ws: bool,
    had_non_ws: bool,
    line_is_blank: bool,
) -> (u8, CellContent) {
    // Tab: expand to next tab stop.
    if grapheme_str == "\t" {
        let tab_width = tab_width.max(1) as u16;
        let next_stop = (current_col / tab_width + 1) * tab_width;
        let display_width = (next_stop - current_col).min(255) as u8;
        let content = if should_render_whitespace(&whitespace.tab, in_leading_ws, had_non_ws, line_is_blank) {
            CellContent::Indicator(whitespace.tab_char)
        } else {
            CellContent::Indicator(" ") // tabs render as spaces when indicator is off
        };
        return (display_width, content);
    }

    // Space
    if grapheme_str == " " {
        let content = if should_render_whitespace(&whitespace.space, in_leading_ws, had_non_ws, line_is_blank) {
            CellContent::Indicator(whitespace.space_char)
        } else {
            CellContent::Grapheme
        };
        return (1, content);
    }

    // Regular grapheme: use unicode-width for display width.
    let w = unicode_display_width(grapheme_str).min(2) as u8;
    let w = w.max(1); // always at least 1 column
    (w, CellContent::Grapheme)
}

/// Returns `true` if a whitespace indicator should be rendered for this cell.
///
/// `in_leading_ws`: still in the line's leading whitespace (before any non-ws char).
/// `had_non_ws`:    at least one non-ws grapheme has already been emitted.
fn should_render_whitespace(
    render: &WhitespaceRender,
    in_leading_ws: bool,
    had_non_ws: bool,
    line_is_blank: bool,
) -> bool {
    match render {
        WhitespaceRender::None => false,
        WhitespaceRender::All => true,
        // Trailing: ws that comes after content but before end-of-line.
        // Leading whitespace and blank lines are never "trailing".
        WhitespaceRender::Trailing => !in_leading_ws && had_non_ws && !line_is_blank,
    }
}

/// Unicode display width for a grapheme cluster, using unicode-width.
pub(crate) fn unicode_display_width(s: &str) -> usize {
    s.width()
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Count the number of indent levels in a line's leading whitespace.
/// One indent level = `tab_width` columns (spaces) or one tab stop.
pub(crate) fn compute_indent_depth(line_str: &str, tab_width: u8) -> u8 {
    let tw = tab_width.max(1) as usize;
    let mut col = 0usize;
    // Leading whitespace is always ASCII (space/tab), so byte iteration is safe and faster.
    for b in line_str.bytes() {
        match b {
            b' ' => col += 1,
            b'\t' => col = (col / tw + 1) * tw,
            _ => break,
        }
    }
    (col / tw).min(u8::MAX as usize) as u8
}

/// Remove a trailing `\n` from a string buffer in-place.
pub(crate) fn strip_line_ending(buf: &mut String) {
    if buf.ends_with('\n') {
        buf.pop();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::{WhitespaceConfig, WrapMode};

    fn do_format(text: &str, wrap_mode: WrapMode) -> (Vec<DisplayRow>, Vec<Grapheme>) {
        let rope = Rope::from_str(text);
        let visible = crate::layout::VisibleRange {
            line_range: 0..rope.len_lines(),
            top_skip_rows: 0,
            content_height: 50,
            content_width: 80,
            gutter_width: 0,
            total_lines: rope.len_lines(),
        };
        let ws = WhitespaceConfig::default();
        let mut inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        format_lines(&rope, &visible, 4, &ws, &wrap_mode, &[], &[], &mut inserts, &mut scratch);
        (scratch.display_rows, scratch.graphemes)
    }

    #[test]
    fn single_line_no_wrap() {
        // No trailing newline → ropey sees exactly 1 line.
        let (rows, graphemes) = do_format("hello", WrapMode::None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
        assert_eq!(graphemes.len(), 5); // 'h','e','l','l','o'
    }

    #[test]
    fn empty_line_produces_empty_sentinel_grapheme() {
        // "a\n\nb" has 3 lines: "a", "", "b".
        // The middle empty line must produce exactly 1 sentinel grapheme with
        // CellContent::Empty so the selection head has something to render on.
        let (rows, graphemes) = do_format("a\n\nb", WrapMode::None);
        assert_eq!(rows.len(), 3, "three lines");
        let empty_row = &rows[1];
        assert_eq!(empty_row.kind, RowKind::LineStart { line_idx: 1 });
        let row_gs = &graphemes[empty_row.graphemes.clone()];
        assert_eq!(row_gs.len(), 1, "exactly one sentinel grapheme");
        assert!(matches!(row_gs[0].content, CellContent::Empty), "sentinel must be Empty");
        assert_eq!(row_gs[0].col, 0);
        assert_eq!(row_gs[0].width, 1);
    }

    #[test]
    fn two_lines_no_wrap() {
        // No trailing newline → ropey sees exactly 2 lines.
        let (rows, graphemes) = do_format("ab\ncd", WrapMode::None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].graphemes.len(), 2);
        assert_eq!(rows[1].graphemes.len(), 2);
        assert_eq!(graphemes.len(), 4);
    }

    #[test]
    fn soft_wrap_produces_continuation_rows() {
        // 10 chars, wrapped at width 4: rows "hell", "o wo", "rld"
        let (rows, _) = do_format("hello world\n", WrapMode::Soft { width: 4 });
        assert!(rows.len() >= 2, "expected at least 2 rows, got {}", rows.len());
        assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
        assert!(matches!(rows[1].kind, RowKind::Wrap { line_idx: 0, .. }));
    }

    #[test]
    fn tab_expansion_advances_to_tabstop() {
        let rope = Rope::from_str("\t");
        let visible = crate::layout::VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 10,
            content_width: 80,
            gutter_width: 0,
            total_lines: 1,
        };
        let mut inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        format_lines(&rope, &visible, 4, &WhitespaceConfig::default(), &WrapMode::None, &[], &[], &mut inserts, &mut scratch);
        assert_eq!(scratch.graphemes[0].width, 4); // tab at col 0 → 4 wide
    }

    #[test]
    fn indent_depth_two_spaces() {
        assert_eq!(compute_indent_depth("  foo", 2), 1);
        assert_eq!(compute_indent_depth("    foo", 2), 2);
        assert_eq!(compute_indent_depth("foo", 2), 0);
    }

    #[test]
    fn past_eof_produces_filler() {
        // "a" has 1 line (index 0). Line indices 1+ are past EOF → Filler.
        let rope = Rope::from_str("a");
        let visible = crate::layout::VisibleRange {
            line_range: 0..4,
            top_skip_rows: 0,
            content_height: 10,
            content_width: 80,
            gutter_width: 0,
            total_lines: 1,
        };
        let mut inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        format_lines(&rope, &visible, 4, &WhitespaceConfig::default(), &WrapMode::None, &[], &[], &mut inserts, &mut scratch);
        // Row 0: real line; rows 1..3: Filler
        assert_eq!(scratch.display_rows[0].kind, RowKind::LineStart { line_idx: 0 });
        assert!(scratch.display_rows[1..].iter().all(|r| r.kind == RowKind::Filler));
    }

    #[test]
    fn grapheme_cols_are_correct() {
        let (_, graphemes) = do_format("abc\n", WrapMode::None);
        assert_eq!(graphemes[0].col, 0);
        assert_eq!(graphemes[1].col, 1);
        assert_eq!(graphemes[2].col, 2);
    }

    // ── Whitespace indicators ─────────────────────────────────────────────

    fn do_format_ws(text: &str, ws: WhitespaceConfig) -> (Vec<DisplayRow>, Vec<Grapheme>) {
        let rope = Rope::from_str(text);
        let visible = crate::layout::VisibleRange {
            line_range: 0..rope.len_lines(),
            top_skip_rows: 0,
            content_height: 50,
            content_width: 80,
            gutter_width: 0,
            total_lines: rope.len_lines(),
        };
        let mut inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        format_lines(&rope, &visible, 4, &ws, &WrapMode::None, &[], &[], &mut inserts, &mut scratch);
        (scratch.display_rows, scratch.graphemes)
    }

    #[test]
    fn newline_indicator_all_mode() {
        let mut ws = WhitespaceConfig::default();
        ws.newline = crate::pane::WhitespaceRender::All;
        ws.newline_char = "⏎";
        let (rows, graphemes) = do_format_ws("abc\n", ws);
        // "abc\n" has 2 ropey lines: "abc\n" (line 0) and "" (line 1, trailing).
        // Line 0: 3 content graphemes + 1 newline indicator.
        // Line 1: 1 Empty sentinel (added so the selection head is visible on empty lines).
        assert_eq!(rows.len(), 2);
        let row0_gs = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(row0_gs.len(), 4, "line 0: 3 content + 1 newline indicator");
        let nl_indicator = &row0_gs[3];
        assert!(matches!(&nl_indicator.content, CellContent::Indicator(s) if *s == "⏎"));
        assert_eq!(nl_indicator.col, 3);
    }

    #[test]
    fn newline_indicator_trailing_mode_shows_after_content() {
        let mut ws = WhitespaceConfig::default();
        ws.newline = crate::pane::WhitespaceRender::Trailing;
        let (_, graphemes) = do_format_ws("abc\n", ws);
        // Trailing: shows after non-ws content, so it appears.
        assert!(graphemes.iter().any(|g| matches!(&g.content, CellContent::Indicator(_))));
    }

    #[test]
    fn newline_indicator_trailing_mode_blank_line() {
        // Blank lines (only spaces) must NOT get a trailing newline indicator.
        let mut ws = WhitespaceConfig::default();
        ws.newline = crate::pane::WhitespaceRender::Trailing;
        let (_, graphemes) = do_format_ws("   \n", ws);
        assert!(!graphemes.iter().any(|g| matches!(&g.content, CellContent::Indicator(_))));
    }

    #[test]
    fn newline_indicator_none_mode() {
        let mut ws = WhitespaceConfig::default();
        ws.newline = crate::pane::WhitespaceRender::None;
        let (_, graphemes) = do_format_ws("abc\n", ws);
        assert!(!graphemes.iter().any(|g| matches!(&g.content, CellContent::Indicator(_))));
    }

    #[test]
    fn space_indicator_all_mode() {
        let mut ws = WhitespaceConfig::default();
        ws.space = crate::pane::WhitespaceRender::All;
        ws.space_char = "·";
        let (_, graphemes) = do_format_ws("a b\n", ws);
        // Space at index 1 should be Indicator
        let space_g = graphemes.iter().find(|g| g.col == 1).unwrap();
        assert!(matches!(&space_g.content, CellContent::Indicator(s) if *s == "·"));
    }

    #[test]
    fn tab_indicator_all_mode() {
        let mut ws = WhitespaceConfig::default();
        ws.tab = crate::pane::WhitespaceRender::All;
        ws.tab_char = "→";
        let rope = Rope::from_str("\t");
        let visible = crate::layout::VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 10,
            content_width: 80,
            gutter_width: 0,
            total_lines: 1,
        };
        let mut inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        format_lines(&rope, &visible, 4, &ws, &WrapMode::None, &[], &[], &mut inserts, &mut scratch);
        assert!(matches!(&scratch.graphemes[0].content, CellContent::Indicator(s) if *s == "→"));
        assert_eq!(scratch.graphemes[0].width, 4);
    }

    // ── Wrap modes ────────────────────────────────────────────────────────

    #[test]
    fn word_wrap_breaks_at_whitespace() {
        // "ab cd ef" with width 5: "ab cd" fits, then "ef" on next row.
        let (rows, graphemes) = do_format("ab cd ef", WrapMode::Word { width: 5 });
        assert!(rows.len() >= 2);
        assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
        assert!(matches!(rows[1].kind, RowKind::Wrap { line_idx: 0, .. }));
        // The first row must not contain 'e' or 'f'.
        let row0_graphemes = &graphemes[rows[0].graphemes.clone()];
        assert!(row0_graphemes.len() <= 5);
    }

    #[test]
    fn indent_wrap_continuation_starts_at_indent_col() {
        // "    long" with 4 spaces of indent (depth=1, tab_width=4), width=6.
        // First row: "    lo", continuation row starts at col 4.
        let (rows, graphemes) = do_format("    long text here", WrapMode::Indent { width: 6 });
        assert!(rows.len() >= 2);
        let wrap_row_graphemes = &graphemes[rows[1].graphemes.clone()];
        // The first grapheme on the continuation row should be at col 4 (indent level).
        assert_eq!(wrap_row_graphemes[0].col, 4);
    }

    // ── CJK double-width ─────────────────────────────────────────────────

    #[test]
    fn cjk_character_produces_width_continuation() {
        // '中' is a CJK character, display width 2.
        let (_, graphemes) = do_format("中", WrapMode::None);
        assert_eq!(graphemes.len(), 2);
        assert_eq!(graphemes[0].width, 2);
        assert_eq!(graphemes[0].col, 0);
        assert!(matches!(graphemes[1].content, CellContent::WidthContinuation));
        assert_eq!(graphemes[1].col, 2);
    }

    // ── indent_depth helpers ─────────────────────────────────────────────

    #[test]
    fn indent_depth_with_tabs() {
        // Two tabs with tab_width=4 => 2 indent levels.
        assert_eq!(compute_indent_depth("\t\tfoo", 4), 2);
        // Mixed: tab (0→4) then space (4→5), depth = 5/4 = 1.
        assert_eq!(compute_indent_depth("\t foo", 4), 1);
    }

    #[test]
    fn indent_depth_zero_tab_width_no_panic() {
        // tab_width=0 should be clamped to 1 internally.
        let depth = compute_indent_depth("  foo", 0);
        assert_eq!(depth, 2); // tw=1, col=2, depth=2
    }

    // ── strip_line_ending ─────────────────────────────────────────────────

    #[test]
    fn strip_line_ending_removes_newline() {
        let mut buf = "hello\n".to_string();
        strip_line_ending(&mut buf);
        assert_eq!(buf, "hello");
    }

    #[test]
    fn strip_line_ending_no_newline_unchanged() {
        let mut buf = "hello".to_string();
        strip_line_ending(&mut buf);
        assert_eq!(buf, "hello");
    }

    #[test]
    fn strip_line_ending_cr_not_stripped() {
        // Engine assumes Unix line endings; \r is left in place.
        let mut buf = "hello\r\n".to_string();
        strip_line_ending(&mut buf);
        assert_eq!(buf, "hello\r");
    }
}
