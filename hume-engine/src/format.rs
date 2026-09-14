use std::ops::Range;

use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::offset::ExclusiveRange;
use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::InlineInsert;
use crate::types::{CellContent, DisplayLine, DisplayLineKind, Grapheme};

mod cells;
mod scratch;
mod virtual_cells;

pub use scratch::{FormatBound, LineFormat, VirtualLineScratch};
pub(crate) use virtual_cells::{VirtualRun, push_arena_text, push_virtual_cells};

use cells::grapheme_display;

// ---------------------------------------------------------------------------
// Buffer line formatting
// ---------------------------------------------------------------------------

/// Format one buffer line, appending zero or more `DisplayLine`s.
///
/// `h_window` clips emitted graphemes to a horizontal column range — used only
/// by the fused render pipeline in `WrapMode::None`, where a single line can be
/// arbitrarily long (a 1MB minified-JS line is a real case). Once the scan
/// passes `h_window.end`, formatting stops early (bounding CPU cost to the
/// visible prefix instead of the whole line); graphemes left of `h_window.start`
/// are scanned (needed for tab-stop column arithmetic) but not pushed, since the
/// compose stage would discard them anyway. Pass `None` for wrapping modes
/// (already bounded by `wrap_width`) and for editor-side callers, which bound
/// themselves by target position through `bound` instead — the window is a
/// *viewport* clip, and their targets are routinely outside it (secondary
/// selection heads are never tracked horizontally; the primary's own target
/// is off-window until `ensure_cursor_visible_horizontal` scrolls to it
/// afterwards). Reusing `h_window` for these queries was tried and reverted:
/// a clipped-out target silently resolves to the wrong column instead of
/// erroring.
///
/// `bound` stops the scan once the requesting query's answer is determined —
/// see [`FormatBound`]. Pass [`FormatBound::Full`] whenever the display line
/// count or the line's tail matters.
#[allow(clippy::too_many_arguments)]
pub fn format_buffer_line(
    rope: &Rope,
    line_idx: hume_rope::line::RopeyLine,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    h_window: Option<Range<DisplayLineCol>>,
    bound: FormatBound,
    inline_inserts: &[InlineInsert],
    out: &mut LineFormat,
) {
    // The caller (`display_lines::DisplayLineMap::ensure_formatted`) resets `out` right before
    // this call, so `text_start` is always 0 — kept as a variable (not
    // assumed) so `line_str` below stays correct if that contract ever
    // changes. Rope chunks are valid UTF-8.
    let text_start = out.line_texts.len();
    let line_slice = rope.line(line_idx.index());
    // The one buffer whose final size is known before writing it. Reserving
    // turns the chunk loop into a single allocation instead of a doubling
    // chain, which matters because `LineFormat::new` deliberately hands over
    // an empty buffer.
    out.line_texts.reserve(line_slice.len_bytes());
    for chunk in line_slice.chunks() {
        out.line_texts.push_str(chunk);
    }
    // Strip the trailing `\n` ropey includes for every non-final line — the
    // EOL sentinel below is emitted only for a line that actually had one.
    let had_newline = hume_rope::lines::truncate_line_break(&mut out.line_texts);

    let line_str = &out.line_texts[text_start..];

    // Byte offset where trailing whitespace begins. A ws grapheme is
    // "trailing" iff its byte offset is at/after this point — this excludes
    // leading and interior whitespace in one check. On an all-whitespace line
    // `trim_end()` yields `""` (offset 0), so every ws char counts as trailing.
    let trailing_ws_start = line_str.trim_end().len();
    let indent_depth = hume_rope::width::indent_depth(line_str, tab_width);

    // `WrapMode { width }` stays terminal-bounded (`u16`) — widened here since
    // it's compared against `current_display_col`, which now tracks a document column
    // that can exceed a `u16`. `None` means no wrap.
    let wrap_width: Option<u32> = wrap_mode.wrap_width().map(u32::from);
    // For indent-wrap, continuation display lines start at this column.
    let indent_display_cols: DisplayLineCol = if matches!(wrap_mode, WrapMode::Indent { .. }) {
        DisplayLineCol::new(hume_rope::width::indent_stop(
            indent_depth as u32,
            tab_width,
        ))
    } else {
        DisplayLineCol::new(0)
    };
    // Word/Indent backtrack to the last whitespace on overflow; Soft splits at
    // the exact wrap column.
    let word_break = matches!(wrap_mode, WrapMode::Word { .. } | WrapMode::Indent { .. });

    // ── Display line / column state ─────────────────────────────────────
    // Aliases into the output buffers so the rest of the function can use
    // the original `lines_out` / `graphemes_out` names without further changes.
    let lines_out = &mut out.display_lines;
    let graphemes_out = &mut out.graphemes;
    let virtual_texts_out = &mut out.virtual_texts;

    let mut insert_idx = 0usize;
    let mut wrap = WrapState {
        current_display_col: DisplayLineCol::new(0),
        wrap_index: 0,
        line_g_start: graphemes_out.len(),
        // Word-wrap state: remember the last whitespace position in the current display line.
        last_ws_g_idx: graphemes_out.len(), // grapheme index of last ws boundary
        word_break,
    };

    // Push the first display line.
    lines_out.push(DisplayLine {
        kind: DisplayLineKind::LineStart { line_idx },
        graphemes: wrap.line_g_start..0, // closed later
    });

    let mut in_leading_ws = true;

    // Running absolute char position within the buffer. Populated per grapheme
    // so the style stage can resolve selection positions without rope lookups.
    let mut char_pos = hume_rope::lines::line_start_char(rope, line_idx).index();

    // Set when the scan stopped early — either `h_window` reached its right
    // edge, or `bound` was satisfied. Everything past that point — the EOL
    // sentinel, trailing inserts, the newline indicator — sits at or beyond
    // the true end of line, so it is skipped rather than emitted at a column
    // the truncated scan never reached.
    let mut clipped = false;

    'lines: for (byte_offset, grapheme_str) in line_str.grapheme_indices(true) {
        // ── Inject inline inserts before this byte offset ─────────────────
        while insert_idx < inline_inserts.len()
            && inline_inserts[insert_idx].byte_offset.index() <= byte_offset
        {
            if h_window
                .as_ref()
                .is_some_and(|w| wrap.current_display_col >= w.end)
            {
                clipped = true;
                break 'lines;
            }
            let ins = &inline_inserts[insert_idx];
            if wrap_width.is_none() && h_window.is_none() {
                // No wrapping and no horizontal window: `maybe_wrap` below
                // would be a no-op and the visibility check would
                // short-circuit on `h_window`'s own `None` — the only thing
                // the insert's width would answer in that case is "is it
                // empty", cheaper to ask directly than to walk every
                // grapheme cluster to sum a width nothing downstream reads.
                if !ins.text.is_empty() {
                    push_virtual_cells(
                        virtual_texts_out,
                        graphemes_out,
                        &VirtualRun {
                            text: &ins.text,
                            byte_offset,
                            char_offset: char_pos,
                            indent_depth,
                        },
                        tab_width,
                        &mut wrap.current_display_col,
                        |_| Some(ins.scope),
                    );
                }
            } else {
                // `wrap_width`/`h_window` are mutually exclusive
                // (`DisplayLineMap::with_h_window`'s own debug_assert: h_window is a
                // `WrapMode::None`-only clip), so exactly one of the two
                // branches below ever does anything to `ins_width`:
                // `maybe_wrap` moves `current_display_col` only when
                // wrapping, and the `visible` check only reads `ins_width`
                // when `h_window` is `Some` — which is only when not
                // wrapping, i.e. before `maybe_wrap` had any chance to move
                // anything. Either way `ins_width` is measured at the exact
                // column it's later used against; nothing here can go stale.
                let ins_width = hume_rope::width::str_width(
                    &ins.text,
                    wrap.current_display_col.get() as usize,
                    tab_width,
                )
                .min(255) as u8;
                if ins_width > 0 {
                    wrap.maybe_wrap(
                        ins_width,
                        wrap_width,
                        indent_display_cols,
                        line_idx,
                        indent_depth,
                        lines_out,
                        graphemes_out,
                    );
                    let visible = h_window.as_ref().is_none_or(|w| {
                        wrap.current_display_col
                            .advance_saturating(ins_width as u32)
                            > w.start
                    });
                    if visible {
                        push_virtual_cells(
                            virtual_texts_out,
                            graphemes_out,
                            &VirtualRun {
                                text: &ins.text,
                                byte_offset,
                                char_offset: char_pos,
                                indent_depth,
                            },
                            tab_width,
                            &mut wrap.current_display_col,
                            |_| Some(ins.scope),
                        );
                    } else {
                        wrap.current_display_col = wrap
                            .current_display_col
                            .advance_saturating(ins_width as u32);
                    }
                }
            }
            insert_idx += 1;
        }

        if h_window
            .as_ref()
            .is_some_and(|w| wrap.current_display_col >= w.end)
        {
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
            wrap.current_display_col,
            tab_width,
            whitespace,
            is_trailing,
            virtual_texts_out,
        );

        // ── Wrap if necessary ─────────────────────────────────────────────
        wrap.maybe_wrap(
            width,
            wrap_width,
            indent_display_cols,
            line_idx,
            indent_depth,
            lines_out,
            graphemes_out,
        );

        // A tab deferred whole to a continuation display line expands from its new
        // (post-wrap) column, not the one `grapheme_display` computed it at —
        // tab width is column-dependent, unlike every other grapheme's.
        let width = if grapheme_str == "\t" {
            hume_rope::width::grapheme_width(
                "\t",
                wrap.current_display_col.get() as usize,
                tab_width,
            ) as u8
        } else {
            width
        };

        // ── Emit grapheme ─────────────────────────────────────────────────
        let char_count = grapheme_str.chars().count();
        // Read after `maybe_wrap`, which rewrites `current_display_col` when it moves
        // this grapheme to a continuation display line. Shared by the pushed cell and
        // the `bound` check below so the two cannot disagree.
        let start_display_col = wrap.current_display_col;
        let byte_start = ByteCol::new(byte_offset);
        let byte_range = ExclusiveRange::new(
            byte_start,
            byte_start.advance_saturating(grapheme_str.len()),
        );
        let visible = h_window
            .as_ref()
            .is_none_or(|w| start_display_col.advance_saturating(width as u32) > w.start);
        if visible {
            graphemes_out.push(Grapheme {
                byte_range,
                char_offset: char_pos,
                display_col: start_display_col,
                width,
                content,
                indent_depth,
                scope: None,
            });
        }
        char_pos += char_count;
        wrap.current_display_col = wrap.current_display_col.advance_saturating(width as u32);

        // For CJK (width == 2): emit a WidthContinuation placeholder so the
        // render stage knows not to write anything to the second cell.
        if width == 2 && visible {
            // Both cells of a double-wide char always stay on the same display line.
            // Backing up the primary to avoid overflow is not yet implemented.
            graphemes_out.push(Grapheme {
                byte_range,
                // Same char as the primary cell — this is not a distinct buffer position.
                char_offset: char_pos - char_count,
                display_col: wrap.current_display_col,
                width: 0, // zero — does not consume columns
                content: CellContent::WidthContinuation,
                indent_depth,
                scope: None,
            });
        }

        // ── Track word-break position ─────────────────────────────────────
        // Recorded after the emit above (the grapheme and, for a two-column
        // cluster, its `WidthContinuation`) so `graphemes_out.len()` already
        // points one past every cell this whitespace grapheme occupies. A
        // tab landing on a 2-column stop pushes both its own cell and a
        // continuation cell; recording the boundary before either was pushed
        // (as a bare `+ 1`) assumed one cell per whitespace grapheme and left
        // a split at this boundary stranding the continuation as the next
        // display line's first cell while the tab itself stayed on the
        // previous display line.
        if is_ws && !in_leading_ws {
            wrap.last_ws_g_idx = graphemes_out.len();
        }

        // Checked here, at the very end of the iteration, so the grapheme that
        // satisfies the bound is emitted whole — with any inline inserts that
        // precede it and its own width-continuation cell. Stopping earlier
        // (inside the insert-injection loop) could leave a run of `Virtual`
        // cells as the last thing on the display line, and `NearestContent` excludes
        // those, so the real grapheme they decorate would go missing.
        if bound.reached(byte_range, start_display_col) {
            clipped = true;
            break 'lines;
        }
    }

    // ── End-of-line sentinel, trailing inserts, newline indicator ──────────
    // Skipped entirely when the h_window scan stopped early (`clipped`): all
    // three sit at or past the true end of line, which is off-screen by
    // definition once the window's right edge has been passed.
    if !clipped {
        // Both the EOL sentinel and the newline indicator below sit at the
        // line's own end byte — an empty span, since neither is real line
        // content.
        let eol_bytes =
            ExclusiveRange::new(ByteCol::new(line_str.len()), ByteCol::new(line_str.len()));

        // Emit an Empty grapheme at the char offset of the trailing `\n` whenever
        // the line has a trailing newline. This gives the cursor/selection-head a
        // cell to land on when positioned on the newline character (e.g. after `x`
        // selects the whole line). Without this, `char_offset_to_display_col` in
        // the style stage finds no grapheme at the `\n` position and leaves the cursor
        // invisible in block-cursor modes.
        //
        // For truly empty lines (just "\n") this is the only grapheme (display_col 0).
        // For non-empty lines it sits one column past the last visible character.
        if had_newline {
            // A display line that fits exactly `wrap_width` columns of real
            // content has no column left for the sentinel itself — wrap it
            // onto a fresh continuation display line (its own `maybe_wrap`
            // call, same as any other cell) rather than letting it land one
            // column past the pane's own right edge, where the cursor it
            // stands in for would render invisible or bleed into the divider
            // seam.
            wrap.maybe_wrap(
                1,
                wrap_width,
                indent_display_cols,
                line_idx,
                indent_depth,
                lines_out,
                graphemes_out,
            );
            graphemes_out.push(Grapheme {
                byte_range: eol_bytes,
                char_offset: char_pos, // char offset of the `\n`
                display_col: wrap.current_display_col,
                width: 1,
                content: CellContent::Empty,
                indent_depth: 0,
                scope: None,
            });
        }

        // ── Emit any trailing inline inserts ────────────────────────────────
        for ins in &inline_inserts[insert_idx..] {
            push_virtual_cells(
                virtual_texts_out,
                graphemes_out,
                &VirtualRun {
                    text: &ins.text,
                    byte_offset: line_str.len(),
                    char_offset: char_pos,
                    indent_depth,
                },
                tab_width,
                &mut wrap.current_display_col,
                |_| Some(ins.scope),
            );
        }

        // ── Newline indicator ───────────────────────────────────────────────
        // Emitted at the end of the line (after all content and trailing inserts)
        // on the last wrap display line. A newline is inherently always at end-of-line,
        // so there's no "trailing vs interior" distinction here — just on/off.
        if had_newline && whitespace.newline {
            let (start, len) = push_arena_text(virtual_texts_out, whitespace.newline_char);
            graphemes_out.push(Grapheme {
                byte_range: eol_bytes,
                // Same offset as the EOL sentinel (the `\n` position). Style-stage
                // lookups resolve to the *first* grapheme at a given offset, which
                // is the EOL sentinel pushed earlier in this function — the
                // indicator itself is never the cursor-cell match.
                char_offset: char_pos,
                display_col: wrap.current_display_col,
                width: 1,
                content: CellContent::Whitespace { start, len },
                indent_depth,
                scope: None,
            });
        }
    }

    // Close the last display line.
    close_display_line_at(lines_out, wrap.line_g_start, graphemes_out.len());
}

// ---------------------------------------------------------------------------
// Wrap state
// ---------------------------------------------------------------------------

/// Mutable state for the word-wrap / soft-wrap pass inside `format_buffer_line`.
///
/// Grouping these four fields avoids threading them as separate `&mut`
/// parameters through `maybe_wrap`.
struct WrapState {
    current_display_col: DisplayLineCol,
    wrap_index: u16,
    /// Index into `graphemes_out` where the current display line began.
    line_g_start: usize,
    /// Grapheme index of the last seen whitespace boundary in the current
    /// display line (for word-wrap backtracking) — `== line_g_start` means
    /// none has been seen yet, since a split resets both to the same value
    /// in the same `maybe_wrap` call.
    last_ws_g_idx: usize,
    /// Whether to backtrack to the last whitespace boundary on overflow.
    /// True for `Word`/`Indent`; false for `Soft`, which always splits at the
    /// exact wrap column even mid-word.
    word_break: bool,
}

impl WrapState {
    /// If adding `width` columns to `current_display_col` would overflow `wrap_width`,
    /// close the current display line and start a new one. Implements
    /// word-wrap backtracking: when `word_break` is set and a whitespace
    /// boundary has been seen in the current display line, the display line
    /// splits there; otherwise it splits at the current grapheme (soft
    /// break, may split a word).
    #[allow(clippy::too_many_arguments)]
    fn maybe_wrap(
        &mut self,
        width: u8,
        wrap_width: Option<u32>,
        indent_display_cols: DisplayLineCol,
        line_idx: hume_rope::line::RopeyLine,
        indent_depth: u8,
        lines_out: &mut Vec<DisplayLine>,
        graphemes_out: &mut [Grapheme],
    ) {
        let Some(wrap_width) = wrap_width else {
            return;
        };
        if self
            .current_display_col
            .advance_saturating(width as u32)
            .get()
            <= wrap_width
        {
            return;
        }
        if self.current_display_col == DisplayLineCol::new(0) {
            // Single grapheme wider than the viewport — emit it anyway to avoid
            // an infinite loop. (This can happen with very wide tab stops.)
            return;
        }

        // Determine split point: backtrack to last whitespace only when word
        // breaking is enabled (Word/Indent); Soft always splits at the current
        // grapheme, mid-word if necessary.
        let split_at = if self.word_break && self.last_ws_g_idx > self.line_g_start {
            self.last_ws_g_idx
        } else {
            graphemes_out.len() // soft break: split here
        };

        // Close current display line at split_at.
        close_display_line_at(lines_out, self.line_g_start, split_at);

        // Start new display line.
        self.wrap_index += 1;
        self.line_g_start = split_at;

        // Recalculate `current_display_col` for graphemes in [split_at..] on the new display line.
        let mut new_display_col = indent_display_cols;
        for g in &mut graphemes_out[split_at..] {
            g.display_col = new_display_col;
            g.indent_depth = indent_depth;
            new_display_col = new_display_col.advance_saturating(g.width as u32);
        }
        self.current_display_col = new_display_col;
        self.last_ws_g_idx = split_at;

        lines_out.push(DisplayLine {
            kind: DisplayLineKind::Wrap {
                line_idx,
                wrap_index: self.wrap_index,
            },
            graphemes: self.line_g_start..0, // closed later
        });
    }
}

// ---------------------------------------------------------------------------
// Display line closing helpers
// ---------------------------------------------------------------------------

/// Close the last display line in `lines_out`, spanning `[line_g_start, split_at)`.
/// `split_at` is either a mid-line wrap boundary or, for the final display
/// line on a line, `graphemes_out.len()` (every grapheme emitted for the line so far).
fn close_display_line_at(lines_out: &mut [DisplayLine], line_g_start: usize, split_at: usize) {
    if let Some(dline) = lines_out.last_mut() {
        dline.graphemes = line_g_start..split_at;
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
