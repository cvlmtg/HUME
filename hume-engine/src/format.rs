use std::ops::Range;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};
use crate::providers::{InlineInsert, ProviderSet, VirtualLine, VirtualLineAnchor};
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
    scratch.clear_line_bufs();
    scratch.line_texts.clear();
    format_buffer_line(
        rope,
        line_idx,
        tab_width,
        whitespace,
        wrap_mode,
        None,
        &[],
        scratch,
    );
    scratch.display_rows.len()
}

/// Display-row breakdown for a single buffer line: virtual rows anchored
/// `Before` it, its own wrap/content rows, and virtual rows anchored `After`
/// it. `total()` is the number of screen rows the line's whole visual block
/// (virtual rows included) occupies.
///
/// This is the single source of truth for "how many display rows does line
/// N occupy" once any `VirtualLineSource` exists — editor scroll/cursor math
/// must stop counting `content` alone (via `count_visual_rows`) and use this
/// instead, or a virtual block above/below a line throws off both scrolling
/// and cursor placement by the number of virtual rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowsBreakdown {
    pub before: usize,
    pub content: usize,
    pub after: usize,
}

impl RowsBreakdown {
    /// Total screen rows this line's visual block occupies.
    pub fn total(&self) -> usize {
        self.before + self.content + self.after
    }
}

/// Compute the `RowsBreakdown` for `line_idx`.
///
/// Queries every registered `VirtualLineSource` for `line_idx..line_idx + 1`
/// — same per-line-lookup cost contract as `SignSource`/`VirtualLineSource`'s
/// render-time use (cheap; no allocation-heavy work), since this now runs
/// during scroll/cursor math too, not just render. `scratch.virtual_lines`
/// is used as scratch storage for the query and left empty on return.
#[allow(clippy::too_many_arguments)]
pub fn display_rows_for_line(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) -> RowsBreakdown {
    let content = count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, scratch);

    scratch.virtual_lines.clear();
    for (_, provider) in &providers.virtual_lines {
        provider.virtual_lines(
            line_idx..line_idx + 1,
            content_width,
            &mut scratch.virtual_lines,
        );
    }
    let mut before = 0usize;
    let mut after = 0usize;
    for vl in &scratch.virtual_lines {
        match vl.anchor {
            VirtualLineAnchor::Before(n) if n == line_idx => before += 1,
            VirtualLineAnchor::After(n) if n == line_idx => after += 1,
            // A provider returning an anchor for a line outside the queried
            // range would be a provider bug; ignore rather than panic (same
            // "never trust provider output blindly" stance as G3's id stamping).
            _ => {}
        }
    }
    scratch.virtual_lines.clear();

    RowsBreakdown {
        before,
        content,
        after,
    }
}

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
                    let (start, len) = push_arena_text(virtual_texts_out, &ins.text);
                    graphemes_out.push(Grapheme {
                        byte_range: byte_offset..byte_offset, // zero-length: virtual
                        // Char offset of the real grapheme this insert precedes (not
                        // MAX): keeps the row non-decreasing in char_offset, which
                        // `resolve_grapheme_col`'s partition_point requires. Inserts
                        // are pushed before that grapheme, so ties resolve to the
                        // insert first — `resolve_grapheme_col` skips forward past
                        // `Virtual` cells to reach the real one.
                        char_offset: char_pos,
                        col: wrap.current_col,
                        width: ins_width,
                        content: CellContent::Virtual { start, len },
                        indent_depth,
                        scope: Some(ins.scope),
                    });
                }
                wrap.current_col = wrap.current_col.saturating_add(ins_width as u16);
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
            let ins_width = unicode_display_width(&ins.text).min(255) as u8;
            if ins_width > 0 {
                let (start, len) = push_arena_text(virtual_texts_out, &ins.text);
                graphemes_out.push(Grapheme {
                    byte_range: line_str.len()..line_str.len(),
                    // Same offset as the EOL sentinel above (the `\n` position) —
                    // there is no later real grapheme on this row for a trailing
                    // insert to precede. Keeps char_offset non-decreasing.
                    char_offset: char_pos,
                    col: wrap.current_col,
                    width: ins_width,
                    content: CellContent::Virtual { start, len },
                    indent_depth,
                    scope: Some(ins.scope),
                });
                wrap.current_col = wrap.current_col.saturating_add(ins_width as u16);
            }
        }

        // ── Newline indicator ───────────────────────────────────────────────
        // Emitted at the end of the line (after all content and trailing inserts)
        // on the last wrap row. A newline is inherently always at end-of-line,
        // so there's no "trailing vs interior" distinction here — just on/off.
        if had_newline && whitespace.newline {
            let (start, len) = push_arena_text(virtual_texts_out, &whitespace.newline_char);
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
            let (start, len) = push_arena_text(virtual_texts, &whitespace.tab_char);
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
            let (start, len) = push_arena_text(virtual_texts, &whitespace.space_char);
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
            let (start, len) = push_arena_text(virtual_texts, &whitespace.nbsp_char);
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
mod tests {
    use super::*;
    use crate::pane::{WhitespaceConfig, WrapMode};

    #[test]
    fn tab_display_width_normal_range() {
        assert_eq!(
            tab_display_width(0, 4),
            4,
            "tab at col 0, width 4 → full stop"
        );
        assert_eq!(
            tab_display_width(2, 4),
            2,
            "tab at col 2, width 4 → half stop"
        );
        assert_eq!(
            tab_display_width(4, 4),
            4,
            "tab exactly on a stop → full width"
        );
    }

    #[test]
    fn tab_display_width_saturates_near_u16_max() {
        // col=65535, tab_width=4: true next stop is 65536, which overflows u16.
        // Without the saturating_mul/clamp fix this panics in debug builds.
        assert_eq!(tab_display_width(u16::MAX, 4), 1);
    }

    fn do_format(text: &str, wrap_mode: WrapMode) -> (Vec<DisplayRow>, Vec<Grapheme>) {
        let rope = Rope::from_str(text);
        let ws = WhitespaceConfig::default();
        let inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        for line_idx in 0..rope.len_lines() {
            format_buffer_line(
                &rope,
                line_idx,
                4,
                &ws,
                &wrap_mode,
                None,
                &inserts,
                &mut scratch,
            );
        }
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
    fn eol_sentinel_emitted_on_non_empty_line() {
        // "hello\n" — the non-empty line must get an eol sentinel at the `\n`
        // position so the cursor is visible when a line-selection head lands on `\n`.
        let (rows, graphemes) = do_format("hello\n", WrapMode::None);
        // "hello\n" has two ropey lines: "hello\n" and "" (trailing).
        assert_eq!(rows.len(), 2);
        let row0_gs = &graphemes[rows[0].graphemes.clone()];
        // 5 content graphemes + 1 eol sentinel.
        assert_eq!(row0_gs.len(), 6, "5 content + eol sentinel");
        let sentinel = &row0_gs[5];
        assert!(
            matches!(sentinel.content, CellContent::Empty),
            "sentinel must be Empty"
        );
        assert_eq!(sentinel.col, 5, "sentinel one past last char");
        assert_eq!(sentinel.char_offset, 5, "sentinel at \\n char offset");
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
        assert!(
            matches!(row_gs[0].content, CellContent::Empty),
            "sentinel must be Empty"
        );
        assert_eq!(row_gs[0].col, 0);
        assert_eq!(row_gs[0].width, 1);
    }

    #[test]
    fn two_lines_no_wrap() {
        // No trailing newline → ropey sees exactly 2 lines.
        // "ab\n" has a trailing \n so its row gets the eol sentinel (3 graphemes).
        // "cd" has no trailing \n, so no sentinel (2 graphemes).
        let (rows, graphemes) = do_format("ab\ncd", WrapMode::None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].graphemes.len(), 3); // 'a', 'b', eol sentinel
        assert_eq!(rows[1].graphemes.len(), 2); // 'c', 'd'
        assert_eq!(graphemes.len(), 5);
    }

    #[test]
    fn soft_wrap_produces_continuation_rows() {
        // 10 chars, wrapped at width 4: rows "hell", "o wo", "rld"
        let (rows, _) = do_format("hello world\n", WrapMode::Soft { width: 4 });
        assert!(
            rows.len() >= 2,
            "expected at least 2 rows, got {}",
            rows.len()
        );
        assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
        assert!(matches!(rows[1].kind, RowKind::Wrap { line_idx: 0, .. }));
    }

    #[test]
    fn soft_wrap_splits_at_exact_column_not_whitespace() {
        // "hello world" (11 chars) wrapped at width 7. Soft must split mid-word
        // at column 7 → "hello w" (the 'w' is the 7th grapheme), NOT backtrack
        // to the space at index 5 ("hello").
        let (rows, graphemes) = do_format("hello world", WrapMode::Soft { width: 7 });
        assert!(rows.len() >= 2);
        let row0 = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(
            row0.len(),
            7,
            "soft wrap must split at the exact wrap column, got {} graphemes",
            row0.len()
        );
        // The 7th grapheme (index 6) must be 'w', proving the split is mid-word.
        assert_eq!(
            row0[6].char_offset, 6,
            "last grapheme of row 0 is 'w' at char 6"
        );
    }

    #[test]
    fn soft_and_word_differ_at_same_width() {
        // Same input/width as above; Word backtracks to the space, keeping it
        // as row0's last cell ("hello ", 6 graphemes — B11: the space ends
        // the row it was seen on, not the continuation row's first cell),
        // while Soft splits mid-word ("hello w", 7 graphemes). This is the
        // regression guard: before the fix both produced identical output.
        let (soft_rows, soft_graphemes) = do_format("hello world", WrapMode::Soft { width: 7 });
        let (word_rows, word_graphemes) = do_format("hello world", WrapMode::Word { width: 7 });
        let soft_row0 = &soft_graphemes[soft_rows[0].graphemes.clone()];
        let word_row0 = &word_graphemes[word_rows[0].graphemes.clone()];
        assert_ne!(
            soft_row0.len(),
            word_row0.len(),
            "soft and word must differ; soft row0 = {}, word row0 = {}",
            soft_row0.len(),
            word_row0.len()
        );
        assert_eq!(
            word_row0.len(),
            6,
            "word wrap backtracks to the space, which stays on row0 → \"hello \""
        );
        assert_eq!(
            soft_row0.len(),
            7,
            "soft wrap splits mid-word → \"hello w\""
        );
    }

    #[test]
    fn soft_wrap_defers_wide_char_whole_to_next_row_when_it_would_straddle_column() {
        // width=5: "abcd" fills cols 0..4 (current_col=4). The next grapheme
        // '中' (CJK, display width 2) would need cols 4..6, straddling the
        // wrap column — `maybe_wrap` checks *before* placing a grapheme, so
        // it must defer '中' whole to the next row rather than splitting its
        // two display cells across rows.
        let (rows, graphemes) = do_format("abcd\u{4e2d}ef", WrapMode::Soft { width: 5 });
        assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");

        let row0 = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(row0.len(), 4, "row 0 holds only \"abcd\", not a split '中'");
        assert_eq!(row0[3].char_offset, 3, "row 0's last grapheme is 'd'");

        let row1 = &graphemes[rows[1].graphemes.clone()];
        assert_eq!(row1.len(), 4, "'中' + its width continuation + 'e' + 'f'");
        assert_eq!(row1[0].char_offset, 4, "row 1 starts with '中'");
        assert_eq!(row1[0].width, 2, "'中' keeps its full display width");
        assert_eq!(row1[0].col, 0, "'中' starts at column 0 of the new row");
        assert!(
            matches!(row1[1].content, CellContent::WidthContinuation),
            "second cell of '中' stays paired with it on the same row"
        );
        assert_eq!(row1[2].char_offset, 5, "'e' follows on row 1");
        assert_eq!(row1[3].char_offset, 6, "'f' follows on row 1");
    }

    #[test]
    fn soft_wrap_defers_tab_whole_to_next_row_when_it_would_straddle_column() {
        // tab_width=4 (do_format's fixed value). "abcd" fills cols 0..4
        // (current_col=4, already tab-stop-aligned), so the tab needs cols
        // 4..8 (its full 4-column expansion) — straddling wrap_width=6. Soft
        // wrap must defer the whole tab to the next row rather than
        // truncating its expansion mid-tab.
        //
        // Column 4 is chosen so the tab's expansion is congruent whether
        // measured from its original column (4) or its post-wrap column (0)
        // — both are tab-stop-aligned, so this test doesn't also exercise
        // the (separate) post-wrap width recompute; see
        // `soft_wrap_recomputes_tab_width_at_post_wrap_column` for that case.
        let (rows, graphemes) = do_format("abcd\tef", WrapMode::Soft { width: 6 });
        assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");

        let row0 = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(row0.len(), 4, "row 0 holds only \"abcd\"");

        let row1 = &graphemes[rows[1].graphemes.clone()];
        assert_eq!(row1.len(), 3, "tab + 'e' + 'f'");
        assert_eq!(row1[0].col, 0, "tab starts at column 0 of the new row");
        assert_eq!(row1[0].width, 4, "tab keeps its full 4-column expansion");
        assert_eq!(row1[1].char_offset, 5, "'e' follows the tab");
        assert_eq!(row1[2].char_offset, 6, "'f' follows 'e'");
    }

    #[test]
    fn soft_wrap_recomputes_tab_width_at_post_wrap_column() {
        // Pre-wrap col=2 ("ab"): the tab would need cols 2..4 there (width 2,
        // its distance to the next tab stop from col 2). Deferred to a new
        // row, it starts at col 0 instead and must expand its full 4-column
        // tab stop — not keep the stale pre-wrap width of 2.
        let (rows, graphemes) = do_format("ab\tc", WrapMode::Soft { width: 3 });
        assert!(rows.len() >= 2, "tab must overflow onto a new row");
        let row1 = &graphemes[rows[1].graphemes.clone()];
        assert_eq!(row1[0].col, 0, "tab starts at column 0 of the new row");
        assert_eq!(
            row1[0].width, 4,
            "tab must expand its full post-wrap tab stop (4), not the stale pre-wrap width (2)"
        );
    }

    #[test]
    fn soft_wrap_exact_fit_row_keeps_eol_sentinel_on_same_row() {
        // "abcde\n" wrapped at width 5 fits exactly (current_col reaches 5,
        // never exceeding it mid-loop, so no content wrap triggers). The EOL
        // sentinel emitted after the main loop bypasses `maybe_wrap` entirely
        // (see the "End-of-line sentinel" comment), so it lands at col 5 —
        // one column past the wrap boundary — without pushing a *wrap*
        // continuation row for line 0. This pins that behavior: a cursor on
        // the trailing '\n' of an exactly-full soft-wrapped row still renders
        // on that row, not a phantom wrap row.
        //
        // "abcde\n" is still 2 ropey lines ("abcde\n" + a trailing empty
        // line, same as plain "hello\n" in `eol_sentinel_emitted_on_non_empty_line`)
        // — row 1 here is that phantom trailing line's own sentinel, not a
        // continuation of line 0.
        let (rows, graphemes) = do_format("abcde\n", WrapMode::Soft { width: 5 });
        assert_eq!(rows.len(), 2, "line 0's row + the phantom trailing line");
        assert_eq!(
            rows[0].kind,
            RowKind::LineStart { line_idx: 0 },
            "line 0 must not have wrapped into a second row of its own"
        );
        assert_eq!(rows[1].kind, RowKind::LineStart { line_idx: 1 });

        let row0 = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(row0.len(), 6, "5 content graphemes + 1 eol sentinel");
        let sentinel = &row0[5];
        assert!(
            matches!(sentinel.content, CellContent::Empty),
            "sentinel must be Empty"
        );
        assert_eq!(
            sentinel.col, 5,
            "sentinel sits one column past the wrap width"
        );
        assert_eq!(sentinel.char_offset, 5, "sentinel at the \\n char offset");
    }

    #[test]
    fn tab_expansion_advances_to_tabstop() {
        let (_, graphemes) = do_format("\t", WrapMode::None);
        assert_eq!(graphemes[0].width, 4); // tab at col 0 → 4 wide
    }

    #[test]
    fn indent_depth_two_spaces() {
        assert_eq!(compute_indent_depth("  foo", 2), 1);
        assert_eq!(compute_indent_depth("    foo", 2), 2);
        assert_eq!(compute_indent_depth("foo", 2), 0);
    }

    #[test]
    fn grapheme_cols_are_correct() {
        let (_, graphemes) = do_format("abc\n", WrapMode::None);
        assert_eq!(graphemes[0].col, 0);
        assert_eq!(graphemes[1].col, 1);
        assert_eq!(graphemes[2].col, 2);
    }

    // ── Whitespace indicators ─────────────────────────────────────────────

    fn do_format_ws(text: &str, ws: WhitespaceConfig) -> (Vec<DisplayRow>, Vec<Grapheme>, String) {
        let rope = Rope::from_str(text);
        let inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        for line_idx in 0..rope.len_lines() {
            format_buffer_line(
                &rope,
                line_idx,
                4,
                &ws,
                &WrapMode::None,
                None,
                &inserts,
                &mut scratch,
            );
        }
        (
            scratch.display_rows,
            scratch.graphemes,
            scratch.virtual_texts,
        )
    }

    /// Slice the arena text backing an `Indicator`/`Virtual` cell — panics if
    /// `content` isn't one of those variants (test-only helper).
    fn cell_text<'a>(arena: &'a str, content: &CellContent) -> &'a str {
        match content {
            CellContent::Indicator { start, len } | CellContent::Virtual { start, len } => {
                &arena[*start as usize..*start as usize + *len as usize]
            }
            other => panic!("expected Indicator or Virtual content, got {other:?}"),
        }
    }

    #[test]
    fn newline_indicator_all_mode() {
        let ws = WhitespaceConfig {
            newline: true,
            newline_char: "⏎".into(),
            ..WhitespaceConfig::default()
        };
        let (rows, graphemes, arena) = do_format_ws("abc\n", ws);
        // "abc\n" has 2 ropey lines: "abc\n" (line 0) and "" (line 1, trailing).
        // Line 0: 3 content graphemes + 1 eol sentinel + 1 newline indicator = 5.
        // Line 1: 1 Empty sentinel (eol sentinel for empty trailing line).
        assert_eq!(rows.len(), 2);
        let row0_gs = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(
            row0_gs.len(),
            5,
            "line 0: 3 content + eol sentinel + newline indicator"
        );
        // Sentinel is at index 3, newline indicator at index 4.
        let sentinel = &row0_gs[3];
        assert!(
            matches!(sentinel.content, CellContent::Empty),
            "index 3 is the eol sentinel"
        );
        assert_eq!(sentinel.col, 3);
        assert_eq!(sentinel.char_offset, 3); // char offset of the '\n'
        let nl_indicator = &row0_gs[4];
        assert_eq!(cell_text(&arena, &nl_indicator.content), "⏎");
        assert_eq!(nl_indicator.col, 3);
    }

    #[test]
    fn newline_indicator_all_mode_blank_line() {
        // Newline is inherently always at end-of-line, so `all` shows it even
        // on a whitespace-only line — there's no "trailing" axis to exempt it.
        let ws = WhitespaceConfig {
            newline: true,
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, _) = do_format_ws("   \n", ws);
        assert!(
            graphemes
                .iter()
                .any(|g| matches!(&g.content, CellContent::Indicator { .. }))
        );
    }

    #[test]
    fn newline_indicator_none_mode() {
        let ws = WhitespaceConfig {
            newline: false,
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, _) = do_format_ws("abc\n", ws);
        assert!(
            !graphemes
                .iter()
                .any(|g| matches!(&g.content, CellContent::Indicator { .. }))
        );
    }

    #[test]
    fn space_indicator_all_mode() {
        let ws = WhitespaceConfig {
            space: crate::pane::WhitespaceRender::All,
            space_char: "·".into(),
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, arena) = do_format_ws("a b\n", ws);
        // Space at index 1 should be Indicator
        let space_g = graphemes.iter().find(|g| g.col == 1).unwrap();
        assert_eq!(cell_text(&arena, &space_g.content), "·");
    }

    #[test]
    fn nbsp_indicator_all_mode() {
        // NBSP (U+00A0, width 1) and ideographic space (U+3000, width 2) are
        // gated by the `space` render mode but use the distinct nbsp glyph.
        let ws = WhitespaceConfig {
            space: crate::pane::WhitespaceRender::All,
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, arena) = do_format_ws("a\u{A0}b\u{3000}c\n", ws);
        let nbsp_g = graphemes.iter().find(|g| g.col == 1).unwrap();
        assert_eq!(cell_text(&arena, &nbsp_g.content), "⍽");
        assert_eq!(nbsp_g.width, 1);
        let ideo_g = graphemes.iter().find(|g| g.col == 3).unwrap();
        assert_eq!(cell_text(&arena, &ideo_g.content), "⍽");
        assert_eq!(ideo_g.width, 2, "ideographic space keeps its 2-col width");
    }

    #[test]
    fn nbsp_renders_as_itself_when_off() {
        // With space rendering off, invisible spaces stay CellContent::Grapheme
        // (rendered as themselves) and keep their unicode widths.
        let (_, graphemes, _) = do_format_ws("a\u{A0}b\u{3000}c\n", WhitespaceConfig::default());
        let nbsp_g = graphemes.iter().find(|g| g.col == 1).unwrap();
        assert!(matches!(nbsp_g.content, CellContent::Grapheme));
        assert_eq!(nbsp_g.width, 1);
        let ideo_g = graphemes.iter().find(|g| g.col == 3).unwrap();
        assert!(matches!(ideo_g.content, CellContent::Grapheme));
        assert_eq!(ideo_g.width, 2);
    }

    #[test]
    fn tab_indicator_all_mode() {
        let ws = WhitespaceConfig {
            tab: crate::pane::WhitespaceRender::All,
            tab_char: "→".into(),
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, arena) = do_format_ws("\t", ws);
        assert_eq!(cell_text(&arena, &graphemes[0].content), "→");
        assert_eq!(graphemes[0].width, 4);
    }

    #[test]
    fn space_indicator_trailing_mode_interior() {
        // Regression test: only true trailing whitespace (nothing but
        // whitespace follows it on the line) renders as an indicator.
        // Leading and interior spaces must stay plain even though they come
        // after some earlier non-ws content — the bug was classifying any ws
        // following *some* non-ws grapheme as trailing, regardless of
        // whether more content followed.
        let ws = WhitespaceConfig {
            space: crate::pane::WhitespaceRender::Trailing,
            space_char: "·".into(),
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, _) = do_format_ws("  A  B  \n", ws);
        let is_indicator = |col: u16| {
            matches!(
                graphemes.iter().find(|g| g.col == col).unwrap().content,
                CellContent::Indicator { .. }
            )
        };
        // Leading spaces (cols 0-1): plain.
        assert!(!is_indicator(0));
        assert!(!is_indicator(1));
        // Interior spaces (cols 3-4, between 'A' and 'B'): plain.
        assert!(!is_indicator(3));
        assert!(!is_indicator(4));
        // Trailing spaces (cols 6-7): indicators.
        assert!(is_indicator(6));
        assert!(is_indicator(7));
    }

    #[test]
    fn space_indicator_trailing_mode_blank_line() {
        // A whitespace-only line renders all its spaces as trailing
        // indicators — there's no separate content to be "before".
        let ws = WhitespaceConfig {
            space: crate::pane::WhitespaceRender::Trailing,
            space_char: "·".into(),
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, arena) = do_format_ws("   \n", ws);
        for col in 0..3u16 {
            let g = graphemes.iter().find(|g| g.col == col).unwrap();
            assert_eq!(
                cell_text(&arena, &g.content),
                "·",
                "col {col} should be a trailing indicator"
            );
        }
    }

    #[test]
    fn tab_indicator_trailing_mode_interior() {
        // Same interior-whitespace bug, for tabs. Both the glyph and the
        // off-state fallback are `CellContent::Indicator` (tabs always
        // render through the arena — see `grapheme_display`), so the glyph
        // text itself is the only way to distinguish "shown" from "hidden".
        let ws = WhitespaceConfig {
            tab: crate::pane::WhitespaceRender::Trailing,
            tab_char: "→".into(),
            ..WhitespaceConfig::default()
        };
        let (_, graphemes, arena) = do_format_ws("\tA\tB\t\n", ws);
        let glyph_at_offset = |byte_offset: usize| {
            let g = graphemes
                .iter()
                .find(|g| g.byte_range.start == byte_offset)
                .unwrap();
            cell_text(&arena, &g.content).to_string()
        };
        assert_eq!(
            glyph_at_offset(0),
            " ",
            "leading tab renders as plain space"
        );
        assert_eq!(
            glyph_at_offset(2),
            " ",
            "interior tab renders as plain space"
        );
        assert_eq!(glyph_at_offset(4), "→", "trailing tab renders as the glyph");
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
    fn word_wrap_space_ends_previous_row_not_starts_continuation() {
        // "a b" at width 2 (B11 boundary case): 'a' fits at col0; the space
        // fits exactly at col1 (current_col becomes 2); 'b' then overflows
        // (2+1>2), backtracking to the space. The space (char offset 1) must
        // end row0 ("a "), not become row1's leading cell — splitting so the
        // new row would start with the space, rather than after it, was the
        // bug. Independent oracle: char_offset is the input's own char index,
        // computed by hand from "a b" (a=0, space=1, b=2), not derived from
        // any wrap-logic internals.
        let (rows, graphemes) = do_format("a b", WrapMode::Word { width: 2 });
        assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");
        let row0 = &graphemes[rows[0].graphemes.clone()];
        let row1 = &graphemes[rows[1].graphemes.clone()];
        assert_eq!(row0.len(), 2, "row0 is \"a \" (a + trailing space)");
        assert_eq!(row0[0].char_offset, 0, "row0[0] is 'a'");
        assert_eq!(row0[1].char_offset, 1, "row0[1] is the space");
        assert_eq!(row1.len(), 1, "row1 is \"b\" only");
        assert_eq!(row1[0].char_offset, 2, "row1[0] is 'b'");
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
        assert!(matches!(
            graphemes[1].content,
            CellContent::WidthContinuation
        ));
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

    // ── h_window clipping (B1) ───────────────────────────────────────────

    fn do_format_windowed(
        text: &str,
        wrap_mode: WrapMode,
        h_window: Option<Range<u16>>,
    ) -> (Vec<DisplayRow>, Vec<Grapheme>) {
        let rope = Rope::from_str(text);
        let ws = WhitespaceConfig::default();
        let inserts = Vec::new();
        let mut scratch = FormatScratch::new();
        for line_idx in 0..rope.len_lines() {
            format_buffer_line(
                &rope,
                line_idx,
                4,
                &ws,
                &wrap_mode,
                h_window.clone(),
                &inserts,
                &mut scratch,
            );
        }
        (scratch.display_rows, scratch.graphemes)
    }

    #[test]
    fn long_line_no_wrap_clips_to_window_without_panic() {
        // 70,000 ASCII chars — without clipping this would overflow `u16`
        // (`current_col`) long before reaching the end. With a window of
        // [0, 80+slack) only a small prefix should be pushed.
        let text: String = "a".repeat(70_000);
        let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, Some(0..80));
        assert_eq!(rows.len(), 1);
        assert!(
            graphemes.len() <= 90,
            "expected a small clipped prefix, got {} graphemes",
            graphemes.len()
        );
        // Every emitted grapheme must fall within (or just at) the window.
        assert!(graphemes.iter().all(|g| g.col < 90));
    }

    #[test]
    fn long_line_no_wrap_window_scrolled_right_has_correct_cols() {
        // Same 70,000-char ASCII line, scrolled to h_offset = 65,000. Since
        // every char is 1 column wide, col must equal char index (independent
        // oracle) for every grapheme actually emitted around the window.
        let text: String = "a".repeat(70_000);
        let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, Some(65_000..65_080));
        assert_eq!(rows.len(), 1);
        assert!(!graphemes.is_empty(), "window should still emit graphemes");
        for g in &graphemes {
            assert_eq!(
                g.col as usize, g.char_offset,
                "pure-ASCII line: col must equal char index"
            );
        }
        // Nothing before the window's left edge should appear.
        assert!(graphemes.iter().all(|g| g.col >= 65_000));
    }

    // ── Inline-insert char_offset partition invariant (B2) ──────────────

    #[test]
    fn row_char_offsets_are_non_decreasing_with_inline_inserts() {
        // Inserts at several offsets, including one at byte 0 (row-start) and
        // one past the last real char (trailing). `resolve_grapheme_col`'s
        // partition_point requires the whole row sorted by char_offset.
        let rope = Rope::from_str("abcdef");
        let inserts = vec![
            InlineInsert {
                byte_offset: 0,
                text: "Z".into(),
                scope: crate::types::ScopeId(0),
            },
            InlineInsert {
                byte_offset: 2,
                text: "XY".into(),
                scope: crate::types::ScopeId(0),
            },
            InlineInsert {
                byte_offset: 6,
                text: "W".into(),
                scope: crate::types::ScopeId(0),
            },
        ];
        let mut scratch = FormatScratch::new();
        format_buffer_line(
            &rope,
            0,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            None,
            &inserts,
            &mut scratch,
        );
        assert!(
            scratch
                .graphemes
                .windows(2)
                .all(|w| w[0].char_offset <= w[1].char_offset),
            "char_offset must be non-decreasing across the row: {:?}",
            scratch
                .graphemes
                .iter()
                .map(|g| g.char_offset)
                .collect::<Vec<_>>()
        );
    }

    // ── Inline-insert width clamp (B7) ──────────────────────────────────

    #[test]
    fn wide_inline_insert_width_clamps_to_255_not_wraparound() {
        // A 300-char ASCII insert must clamp its display width to 255, not
        // wrap around via `as u8` truncation (300 % 256 = 44 would be the
        // buggy value).
        let rope = Rope::from_str("x");
        let text = String::from_utf8(vec![b'a'; 300]).unwrap();
        let inserts = vec![InlineInsert {
            byte_offset: 0,
            text,
            scope: crate::types::ScopeId(0),
        }];
        let mut scratch = FormatScratch::new();
        format_buffer_line(
            &rope,
            0,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            None,
            &inserts,
            &mut scratch,
        );
        let insert_g = scratch
            .graphemes
            .iter()
            .find(|g| matches!(g.content, CellContent::Virtual { .. }))
            .expect("insert grapheme present");
        assert_eq!(insert_g.width, 255, "width clamps at 255, not 300 % 256");
    }

    #[test]
    fn no_window_caller_does_not_overflow_u16_on_huge_line() {
        // Belt-and-braces: callers that pass `h_window: None` (cursor/visual-move
        // lookups) get no clipping at all, so a pathologically long line must
        // still not panic via `u16` overflow in `current_col` — the accumulation
        // saturates instead. 70,000 ASCII chars comfortably exceeds `u16::MAX`
        // (65,535).
        let text: String = "a".repeat(70_000);
        let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(graphemes.len(), 70_000, "no window: every char is scanned");
        assert_eq!(
            graphemes.last().unwrap().col,
            u16::MAX,
            "current_col saturates at u16::MAX rather than wrapping"
        );
    }

    #[test]
    fn wrapping_modes_unaffected_by_h_window_none() {
        // Regression: passing None (the only value wrapping modes ever get)
        // must reproduce the existing wrap test's output exactly.
        let (rows, graphemes) =
            do_format_windowed("hello world", WrapMode::Soft { width: 7 }, None);
        let row0 = &graphemes[rows[0].graphemes.clone()];
        assert_eq!(row0.len(), 7, "soft wrap still splits mid-word at column 7");
    }

    // ── display_rows_for_line (G6) ───────────────────────────────────────

    struct FixedAnchorSource {
        anchor: crate::providers::VirtualLineAnchor,
        count: usize,
    }

    impl crate::providers::VirtualLineSource for FixedAnchorSource {
        fn virtual_lines(
            &self,
            visible_lines: std::ops::Range<usize>,
            _content_width: u16,
            out: &mut Vec<crate::providers::VirtualLine>,
        ) {
            let line = match self.anchor {
                crate::providers::VirtualLineAnchor::Before(n)
                | crate::providers::VirtualLineAnchor::After(n) => n,
            };
            if visible_lines.contains(&line) {
                for _ in 0..self.count {
                    out.push(crate::providers::VirtualLine {
                        anchor: self.anchor,
                        provider_id: 0,
                        text: String::new(),
                        segments: Vec::new(),
                    });
                }
            }
        }
    }

    #[test]
    fn display_rows_for_line_counts_before_and_after_virtual_rows() {
        // Line 5 of "a\nb\nc\nd\ne\nf\n" (0-based: line 5 is "f") has 2
        // Before rows and 1 After row from two separate providers, plus its
        // own single content row (no wrap).
        let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
        let mut providers = ProviderSet::new();
        providers.add_virtual_line_source(Box::new(FixedAnchorSource {
            anchor: crate::providers::VirtualLineAnchor::Before(5),
            count: 2,
        }));
        providers.add_virtual_line_source(Box::new(FixedAnchorSource {
            anchor: crate::providers::VirtualLineAnchor::After(5),
            count: 1,
        }));

        let mut scratch = FormatScratch::new();
        let breakdown = display_rows_for_line(
            &rope,
            5,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            &providers,
            80,
            &mut scratch,
        );

        assert_eq!(breakdown.before, 2);
        assert_eq!(breakdown.content, 1, "unwrapped single-char line is 1 row");
        assert_eq!(breakdown.after, 1);
        assert_eq!(breakdown.total(), 4);
    }

    #[test]
    fn display_rows_for_line_ignores_virtual_rows_anchored_to_other_lines() {
        // A provider anchored to line 2 must not leak into line 5's breakdown.
        let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
        let mut providers = ProviderSet::new();
        providers.add_virtual_line_source(Box::new(FixedAnchorSource {
            anchor: crate::providers::VirtualLineAnchor::Before(2),
            count: 3,
        }));

        let mut scratch = FormatScratch::new();
        let breakdown = display_rows_for_line(
            &rope,
            5,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            &providers,
            80,
            &mut scratch,
        );

        assert_eq!(breakdown.before, 0);
        assert_eq!(breakdown.after, 0);
    }

    #[test]
    fn display_rows_for_line_no_providers_is_content_only() {
        let rope = Rope::from_str("hello\n");
        let providers = ProviderSet::new();
        let mut scratch = FormatScratch::new();
        let breakdown = display_rows_for_line(
            &rope,
            0,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            &providers,
            80,
            &mut scratch,
        );
        assert_eq!(
            breakdown,
            RowsBreakdown {
                before: 0,
                content: 1,
                after: 0
            }
        );
    }
}
