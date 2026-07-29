use std::ops::Range;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};
use crate::providers::{InlineInsert, VirtualLine};
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
    /// Pre-materialised text for the current buffer line. Written by
    /// `format_buffer_line`; read by `pipeline::render_buffer_line` as `line_str`.
    pub line_texts: String,
    /// Per-frame arena backing `CellContent::Virtual`/`Indicator` text ranges
    /// — inline-insert text, whitespace-indicator glyphs, and virtual-line
    /// text, none of which can be `&'static str` (LSP hints, Steel-configured
    /// icons). Cleared per buffer line in `FrameScratch::clear_line`, mirroring
    /// `line_texts` — compose for that line/virtual-row always runs before
    /// the clear.
    pub virtual_texts: String,
}

impl FormatScratch {
    pub fn new() -> Self {
        Self {
            display_rows: Vec::with_capacity(16),
            graphemes: Vec::with_capacity(512),
            virtual_lines: Vec::new(),
            line_texts: String::with_capacity(512),
            virtual_texts: String::with_capacity(256),
        }
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.clear_line_bufs();
        self.virtual_lines.clear();
        self.line_texts.clear();
    }

    /// Reset the per-buffer-line fields shared with [`FrameScratch::clear_line`]
    /// (`display_rows`, `graphemes`, `virtual_texts`).
    ///
    /// Excludes `line_texts`: the fused render pipeline clears it at its own
    /// point (right before appending that line's text), decoupled from
    /// `clear_line`'s cadence — see `line_texts`'s field doc.
    pub fn clear_line_bufs(&mut self) {
        self.display_rows.clear();
        self.graphemes.clear();
        self.virtual_texts.clear();
    }
}

impl Default for FormatScratch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Buffer line formatting
// ---------------------------------------------------------------------------

/// Format one buffer line, appending zero or more `DisplayRow`s.
///
/// `h_window` clips emitted graphemes to a horizontal column range — used only
/// by the fused render pipeline in `WrapMode::None`, where a single line can be
/// arbitrarily long (a 1MB minified-JS line is a real case). Once the scan
/// passes `h_window.end`, formatting stops early (bounding CPU cost to the
/// visible prefix instead of the whole line); graphemes left of `h_window.start`
/// are scanned (needed for tab-stop column arithmetic) but not pushed, since the
/// compose stage would discard them anyway. Pass `None` for wrapping modes
/// (already bounded by `wrap_width`) and for callers that need the whole line
/// (e.g. cursor-position lookups).
#[allow(clippy::too_many_arguments)]
pub fn format_buffer_line(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    h_window: Option<Range<u16>>,
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

    // Byte offset where trailing whitespace begins. A ws grapheme is
    // "trailing" iff its byte offset is at/after this point — this excludes
    // leading and interior whitespace in one check. On an all-whitespace line
    // `trim_end()` yields `""` (offset 0), so every ws char counts as trailing.
    let trailing_ws_start = line_str.trim_end().len();
    let indent_depth = compute_indent_depth(line_str, tab_width);

    let wrap_width = wrap_mode.wrap_width().unwrap_or(u16::MAX); // u16::MAX = sentinel for "no wrap"
    // For indent-wrap, continuation rows start at this column.
    let indent_cols: u16 = if matches!(wrap_mode, WrapMode::Indent { .. }) {
        (indent_depth as u16) * (tab_width as u16)
    } else {
        0
    };
    // Word/Indent backtrack to the last whitespace on overflow; Soft splits at
    // the exact wrap column.
    let word_break = matches!(wrap_mode, WrapMode::Word { .. } | WrapMode::Indent { .. });

    // ── Row / column state ────────────────────────────────────────────────
    // Aliases into the scratch buffers so the rest of the function can use
    // the original `rows_out` / `graphemes_out` names without further changes.
    let rows_out = &mut scratch.display_rows;
    let graphemes_out = &mut scratch.graphemes;
    let virtual_texts_out = &mut scratch.virtual_texts;

    let mut insert_idx = 0usize;
    let mut wrap = WrapState {
        current_col: 0,
        wrap_row: 0,
        row_g_start: graphemes_out.len(),
        // Word-wrap state: remember the last whitespace position in the current row.
        last_ws_g_idx: graphemes_out.len(), // grapheme index of last ws boundary
        last_ws_was_set: false,
        word_break,
    };

    // Push the first row.
    rows_out.push(DisplayRow {
        kind: RowKind::LineStart { line_idx },
        graphemes: wrap.row_g_start..0, // closed later
    });

    let mut in_leading_ws = true;

    // Running absolute char position within the buffer. Populated per grapheme
    // so the style stage can resolve selection positions without rope lookups.
    let mut char_pos = rope.line_to_char(line_idx);

    // Set when `h_window` bounds the scan and formatting stopped early because
    // `current_col` reached the window's right edge. Everything past that
    // point — the EOL sentinel, trailing inserts, the newline indicator — is
    // off-screen by definition, so it is skipped rather than emitted at the
    // wrong (clipped) column.
    let mut clipped = false;

    'lines: for (byte_offset, grapheme_str) in line_str.grapheme_indices(true) {
        // ── Inject inline inserts before this byte offset ─────────────────
        while insert_idx < inline_inserts.len()
            && inline_inserts[insert_idx].byte_offset <= byte_offset
        {
            if h_window.as_ref().is_some_and(|w| wrap.current_col >= w.end) {
                clipped = true;
                break 'lines;
            }
            let ins = &inline_inserts[insert_idx];
            let ins_width = unicode_display_width(&ins.text).min(255) as u8;
            if ins_width > 0 {
                wrap.maybe_wrap(
                    ins_width,
                    wrap_width,
                    indent_cols,
                    line_idx,
                    indent_depth,
                    rows_out,
                    graphemes_out,
                );
                let visible = h_window
                    .as_ref()
                    .is_none_or(|w| wrap.current_col + ins_width as u16 > w.start);
                if visible {
                    push_insert_cells(
                        virtual_texts_out,
                        graphemes_out,
                        ins,
                        byte_offset..byte_offset, // zero-length: virtual
                        char_pos,
                        indent_depth,
                        &mut wrap.current_col,
                    );
                } else {
                    wrap.current_col = wrap.current_col.saturating_add(ins_width as u16);
                }
            }
            insert_idx += 1;
        }

        if h_window.as_ref().is_some_and(|w| wrap.current_col >= w.end) {
            clipped = true;
            break 'lines;
        }

        // ── Skip newlines (line_str is already stripped; this guards edge cases) ──
        if grapheme_str == "\n" {
            continue;
        }
        // NOTE: newline indicator is emitted after the main loop, below.

        // ── Update leading-ws flag ─────────────────────────────────────────
        let is_ws = is_whitespace_grapheme(grapheme_str);
        if !is_ws && in_leading_ws {
            in_leading_ws = false;
        }
        let is_trailing = byte_offset >= trailing_ws_start;

        // ── Compute display width and content ─────────────────────────────
        let (width, content) = grapheme_display(
            grapheme_str,
            wrap.current_col,
            tab_width,
            whitespace,
            is_trailing,
            virtual_texts_out,
        );

        // ── Wrap if necessary ─────────────────────────────────────────────
        wrap.maybe_wrap(
            width,
            wrap_width,
            indent_cols,
            line_idx,
            indent_depth,
            rows_out,
            graphemes_out,
        );

        // A tab deferred whole to a continuation row expands from its new
        // (post-wrap) column, not the one `grapheme_display` computed it at —
        // tab width is column-dependent, unlike every other grapheme's.
        let width = if grapheme_str == "\t" {
            tab_display_width(wrap.current_col, tab_width)
        } else {
            width
        };

        // ── Track word-break position ─────────────────────────────────────
        if is_ws && !in_leading_ws {
            // `graphemes_out.len()` is the index the space itself is about to
            // occupy (it hasn't been pushed yet — that happens below). Record
            // one past it, so a wrap split at `last_ws_g_idx` starts the new
            // row after the space, leaving it as the previous row's last
            // cell instead of the continuation row's first.
            wrap.last_ws_g_idx = graphemes_out.len() + 1;
            wrap.last_ws_was_set = true;
        }

        // ── Emit grapheme ─────────────────────────────────────────────────
        let char_count = grapheme_str.chars().count();
        let visible = h_window
            .as_ref()
            .is_none_or(|w| wrap.current_col + width as u16 > w.start);
        if visible {
            graphemes_out.push(Grapheme {
                byte_range: byte_offset..byte_offset + grapheme_str.len(),
                char_offset: char_pos,
                col: wrap.current_col,
                width,
                content,
                indent_depth,
                scope: None,
            });
        }
        char_pos += char_count;
        wrap.current_col = wrap.current_col.saturating_add(width as u16);

        // For CJK (width == 2): emit a WidthContinuation placeholder so the
        // render stage knows not to write anything to the second cell.
        if width == 2 && visible {
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
                scope: None,
            });
        }
    }

    // ── End-of-line sentinel, trailing inserts, newline indicator ──────────
    // Skipped entirely when the h_window scan stopped early (`clipped`): all
    // three sit at or past the true end of line, which is off-screen by
    // definition once the window's right edge has been passed.
    if !clipped {
        // Emit an Empty grapheme at the char offset of the trailing `\n` whenever
        // the line has a trailing newline. This gives the cursor/selection-head a
        // cell to land on when positioned on the newline character (e.g. after `x`
        // selects the whole line). Without this, `char_offset_to_col` in the style
        // stage finds no grapheme at the `\n` position and leaves the cursor
        // invisible in block-cursor modes.
        //
        // For truly empty lines (just "\n") this is the only grapheme (col 0).
        // For non-empty lines it sits one column past the last visible character.
        if had_newline {
            graphemes_out.push(Grapheme {
                byte_range: line_str.len()..line_str.len(),
                char_offset: char_pos, // char offset of the `\n`
                col: wrap.current_col,
                width: 1,
                content: CellContent::Empty,
                indent_depth: 0,
                scope: None,
            });
        }

        // ── Emit any trailing inline inserts ────────────────────────────────
        for ins in &inline_inserts[insert_idx..] {
            push_insert_cells(
                virtual_texts_out,
                graphemes_out,
                ins,
                line_str.len()..line_str.len(),
                char_pos,
                indent_depth,
                &mut wrap.current_col,
            );
        }

        // ── Newline indicator ───────────────────────────────────────────────
        // Emitted at the end of the line (after all content and trailing inserts)
        // on the last wrap row. A newline is inherently always at end-of-line,
        // so there's no "trailing vs interior" distinction here — just on/off.
        if had_newline && whitespace.newline {
            let (start, len) = push_arena_text(virtual_texts_out, whitespace.newline_char);
            graphemes_out.push(Grapheme {
                byte_range: line_str.len()..line_str.len(),
                // Same offset as the EOL sentinel (the `\n` position). Style-stage
                // lookups resolve to the *first* grapheme at a given offset, which
                // is the EOL sentinel pushed earlier in this function — the
                // indicator itself is never the cursor-cell match.
                char_offset: char_pos,
                col: wrap.current_col,
                width: 1,
                content: CellContent::Indicator { start, len },
                indent_depth,
                scope: None,
            });
        }
    }

    // Close the last row.
    close_current_row(rows_out, graphemes_out, wrap.row_g_start);
}

// ---------------------------------------------------------------------------
// Wrap state
// ---------------------------------------------------------------------------

/// Mutable state for the word-wrap / soft-wrap pass inside `format_buffer_line`.
///
/// Grouping these five fields avoids threading them as separate `&mut`
/// parameters through `maybe_wrap`.
struct WrapState {
    current_col: u16,
    wrap_row: u16,
    /// Index into `graphemes_out` where the current display row began.
    row_g_start: usize,
    /// Grapheme index of the last seen whitespace boundary (for word-wrap backtracking).
    last_ws_g_idx: usize,
    last_ws_was_set: bool,
    /// Whether to backtrack to the last whitespace boundary on overflow.
    /// True for `Word`/`Indent`; false for `Soft`, which always splits at the
    /// exact wrap column even mid-word.
    word_break: bool,
}

impl WrapState {
    /// If adding `width` columns to `current_col` would overflow `wrap_width`,
    /// close the current row and start a new one. Implements word-wrap
    /// backtracking: when `word_break` is set and `last_ws_was_set`, the row
    /// splits at the last whitespace position; otherwise it splits at the
    /// current grapheme (soft break, may split a word).
    #[allow(clippy::too_many_arguments)]
    fn maybe_wrap(
        &mut self,
        width: u8,
        wrap_width: u16,
        indent_cols: u16,
        line_idx: usize,
        indent_depth: u8,
        rows_out: &mut Vec<DisplayRow>,
        graphemes_out: &mut [Grapheme],
    ) {
        if wrap_width == u16::MAX || self.current_col + width as u16 <= wrap_width {
            return;
        }
        if self.current_col == 0 {
            // Single grapheme wider than the viewport — emit it anyway to avoid
            // an infinite loop. (This can happen with very wide tab stops.)
            return;
        }

        // Determine split point: backtrack to last whitespace only when word
        // breaking is enabled (Word/Indent); Soft always splits at the current
        // grapheme, mid-word if necessary.
        let split_at =
            if self.word_break && self.last_ws_was_set && self.last_ws_g_idx > self.row_g_start {
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
            kind: RowKind::Wrap {
                line_idx,
                wrap_row: self.wrap_row,
            },
            graphemes: self.row_g_start..0, // closed later
        });
    }
}

// ---------------------------------------------------------------------------
// Row closing helpers
// ---------------------------------------------------------------------------

fn close_current_row(rows_out: &mut [DisplayRow], graphemes_out: &[Grapheme], row_g_start: usize) {
    if let Some(row) = rows_out.last_mut() {
        row.graphemes = row_g_start..graphemes_out.len();
    }
}

fn close_row_at(rows_out: &mut [DisplayRow], row_g_start: usize, split_at: usize) {
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

/// Display width of a tab starting at `col`: the distance to the next tab
/// stop. Column-dependent, so a wrap that moves a tab to a new starting
/// column (see `format_buffer_line`'s post-`maybe_wrap` recompute) requires
/// calling this again rather than reusing the pre-wrap width.
fn tab_display_width(col: u16, tab_width: u8) -> u8 {
    let tab_width = tab_width.max(1) as u16;
    let next_stop = (col / tab_width + 1).saturating_mul(tab_width);
    // saturating: at col ≈ u16::MAX the true next stop overflows u16; clamp
    // to a 1-wide tab instead of panicking (debug) or wrapping (release).
    next_stop.saturating_sub(col).clamp(1, 255) as u8
}

/// Compute the display `width` and `CellContent` for one grapheme cluster.
fn grapheme_display(
    grapheme_str: &str,
    current_col: u16,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    is_trailing: bool,
    virtual_texts: &mut String,
) -> (u8, CellContent) {
    // Tab: expand to next tab stop.
    //
    // This is the renderer's tab-stop arithmetic. It is intentionally
    // duplicated with `hume_editing::grapheme::display_col_in_line` (which
    // serves the editing ops) rather than shared: `hume-engine` deliberately
    // does not depend on `hume-editing` (engine has no knowledge of the text
    // model), and the two diverge on purpose — here non-tab graphemes use
    // `unicode-width` so wide CJK chars take 2 columns for rendering, while the
    // editing helper counts every non-tab grapheme as 1 (so CJK-plus-tab
    // mixtures may misalign a tab stop there; bounded and rare). See the doc on
    // `display_col_in_line` for the divergence rationale.
    if grapheme_str == "\t" {
        let display_width = tab_display_width(current_col, tab_width);
        let content = if should_render_whitespace(whitespace.tab, is_trailing) {
            let (start, len) = push_arena_text(virtual_texts, whitespace.tab_char);
            CellContent::Indicator { start, len }
        } else {
            // Tabs render as spaces when the indicator is off.
            let (start, len) = push_arena_text(virtual_texts, " ");
            CellContent::Indicator { start, len }
        };
        return (display_width, content);
    }

    // Space
    if grapheme_str == " " {
        let content = if should_render_whitespace(whitespace.space, is_trailing) {
            let (start, len) = push_arena_text(virtual_texts, whitespace.space_char);
            CellContent::Indicator { start, len }
        } else {
            CellContent::Grapheme
        };
        return (1, content);
    }

    // Invisible Unicode spaces (NBSP, ideographic space): gated by the same
    // `space` render mode but with a distinct glyph, so stray non-breaking
    // spaces stand out from ordinary ones. Width comes from unicode-width
    // (U+3000 is 2 columns), matching the regular-grapheme path so the wrap
    // math is identical whether the indicator is on or off.
    if grapheme_str == "\u{A0}" || grapheme_str == "\u{3000}" {
        let w = unicode_display_width(grapheme_str).clamp(1, 2) as u8;
        let content = if should_render_whitespace(whitespace.space, is_trailing) {
            let (start, len) = push_arena_text(virtual_texts, whitespace.nbsp_char);
            CellContent::Indicator { start, len }
        } else {
            CellContent::Grapheme
        };
        return (w, content);
    }

    // Regular grapheme: use unicode-width for display width.
    let w = unicode_display_width(grapheme_str).min(2) as u8;
    let w = w.max(1); // always at least 1 column
    (w, CellContent::Grapheme)
}

/// Push `text` into the per-frame arena (`FormatScratch::virtual_texts`),
/// returning a `(start, len)` range cheap enough to store in a `Copy`
/// `CellContent`. A single line's pushed text realistically never approaches
/// the `u32`/`u16` bounds; `debug_assert` catches an overflow in tests, while
/// release saturates rather than panicking (mirrors the `current_col`
/// saturation pattern in `format_buffer_line`).
pub(crate) fn push_arena_text(arena: &mut String, text: &str) -> (u32, u16) {
    let start = arena.len();
    arena.push_str(text);
    debug_assert!(
        u32::try_from(start).is_ok(),
        "frame arena start exceeds u32"
    );
    debug_assert!(
        u16::try_from(text.len()).is_ok(),
        "pushed text exceeds u16 length"
    );
    (
        u32::try_from(start).unwrap_or(u32::MAX),
        u16::try_from(text.len()).unwrap_or(u16::MAX),
    )
}

/// Push one `Grapheme`/cell per grapheme cluster of `ins.text`, not one wide
/// cell for the whole string: a ratatui `Cell` renders its `symbol` at
/// exactly one column, so packing a multi-character insert into a single
/// cell leaves the columns after the first unwritten by this insert —
/// whatever the compose stage puts there instead (real buffer content) then
/// wins when the backend paints cell-by-cell, clobbering everything past the
/// first character. `CellContent::Indicator` sidesteps this by keeping its
/// own symbol to one glyph and space-filling the rest; inline-insert text
/// (inlay hints, diagnostics) has no such luxury. Shared by both the
/// mid-line and end-of-line insert sites in `format_buffer_line`.
fn push_insert_cells(
    virtual_texts_out: &mut String,
    graphemes_out: &mut Vec<Grapheme>,
    ins: &InlineInsert,
    byte_range: Range<usize>,
    char_offset: usize,
    indent_depth: u8,
    current_col: &mut u16,
) {
    let (text_start, _) = push_arena_text(virtual_texts_out, &ins.text);
    for (g_byte_offset, g_str) in ins.text.grapheme_indices(true) {
        let g_width = unicode_display_width(g_str).min(255) as u8;
        if g_width == 0 {
            continue;
        }
        graphemes_out.push(Grapheme {
            byte_range: byte_range.clone(),
            // Char offset of the real grapheme this insert precedes (not MAX):
            // keeps the row non-decreasing in char_offset, which
            // `resolve_grapheme_col`'s partition_point requires. Mid-line
            // inserts are pushed before that grapheme, so ties resolve to the
            // insert first — `resolve_grapheme_col` skips forward past
            // `Virtual` cells to reach the real one. Trailing inserts share
            // the EOL sentinel's offset (the `\n` position) since there is no
            // later real grapheme on the row to precede.
            char_offset,
            col: *current_col,
            width: g_width,
            content: CellContent::Virtual {
                start: text_start + g_byte_offset as u32,
                len: g_str.len() as u16,
            },
            indent_depth,
            scope: Some(ins.scope),
        });
        *current_col = current_col.saturating_add(g_width as u16);
    }
}

/// Returns `true` if a whitespace indicator should be rendered for this cell.
///
/// `is_trailing`: this grapheme's byte offset is at/after the start of the
/// line's trailing whitespace run (see `trailing_ws_start` in `format_buffer_line`).
fn should_render_whitespace(render: WhitespaceRender, is_trailing: bool) -> bool {
    match render {
        WhitespaceRender::None => false,
        WhitespaceRender::All => true,
        WhitespaceRender::Trailing => is_trailing,
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
mod tests;
