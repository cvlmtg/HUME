use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::core::buffer::Buffer;
use crate::core::grapheme::grapheme_count;
use crate::editor::{Editor, Mode};

/// Fill an entire status-bar row with spaces in the base style.
///
/// All three bottom-row renderers do this as their first step to clear
/// whatever was drawn in the previous frame.
fn fill_row(screen_buf: &mut ScreenBuf, colors: &crate::ui::theme::EditorColors, area: Rect, y: u16) {
    let blank = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, colors.status_bar);
}

// ── Configuration ─────────────────────────────────────────────────────────────

/// A named element that can appear in a statusline slot.
///
/// Segments are the building blocks of the status bar. The mode pill,
/// separators, and data fields are all first-class segment variants —
/// there is no special chrome. You control the layout by choosing which
/// segments appear in each slot and in what order.
///
/// The Steel scripting layer constructs [`StatusLineConfig`] values at
/// runtime; this enum is the wire format for those configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusSegment {
    /// The mode pill: `" NOR "`, `" INS "`, or `" EXT "`.
    ///
    /// Rendered with the per-mode color (`status_normal`, `status_insert`,
    /// `status_extend`). The 5-char width includes leading and trailing spaces
    /// so it naturally fills a slot without needing a separator.
    ModePill,
    /// A thin vertical bar `│` in the base status bar style.
    ///
    /// Place this explicitly between segments that need a visual divider.
    Separator,
    /// The file's basename, or `"[scratch]"` for unnamed buffers.
    FileName,
    /// Cursor position as `"line:col"` (both 1-based, col = grapheme index).
    Position,
    /// Selection count as `"N sels"`, or the empty string when only one
    /// selection is active (so it occupies no space in single-cursor mode).
    #[allow(dead_code)]
    Selections,
    /// Kitty keyboard protocol indicator: `"🐱"` when active, empty otherwise.
    ///
    /// Useful for diagnosing whether the protocol was successfully negotiated.
    KittyProtocol,
    /// Dirty indicator: `"[+]"` when the buffer has unsaved changes, empty otherwise.
    DirtyIndicator,
}

/// Describes the content layout of the status bar's three horizontal slots.
///
/// Each slot is a sequence of [`StatusSegment`]s rendered in order. Adjacent
/// segments within a slot are joined with a boundary-aware spacing rule so
/// that spacing feels natural without any segment needing to hard-code its
/// neighbours.
///
/// The default config reproduces the built-in status bar layout exactly, so
/// the editor looks identical with no configuration:
///
/// ```text
///  NOR │ notes.txt              42:7
/// ```
#[derive(Debug, Clone)]
pub(crate) struct StatusLineConfig {
    /// Segments rendered left-aligned at the start of the status bar row.
    pub left: Vec<StatusSegment>,
    /// Segments centered between the left and right slot ends. Empty by default.
    pub center: Vec<StatusSegment>,
    /// Segments rendered right-aligned at the end of the status bar row.
    pub right: Vec<StatusSegment>,
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        Self {
            left: vec![StatusSegment::ModePill, StatusSegment::Separator, StatusSegment::FileName, StatusSegment::DirtyIndicator],
            center: vec![],
            right: vec![StatusSegment::KittyProtocol, StatusSegment::Position],
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Render the bottom row of the terminal: command line, status message, or
/// status bar — whichever has the highest priority.
///
/// Priority: command mini-buffer > transient status message > normal status bar.
pub(crate) fn render_bottom_row(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    area: Rect,
    y: u16,
) {
    if let Some(mb) = &editor.minibuf {
        render_command_line(screen_buf, &editor.colors, area, y, mb.prompt, &mb.input);
    } else if let Some(msg) = editor.status_msg.as_deref() {
        render_status_message(screen_buf, &editor.colors, area, y, msg);
    } else {
        render_status_bar(screen_buf, editor, area, y);
    }
}

// ── Renderers ─────────────────────────────────────────────────────────────────

/// Render the command-line mini-buffer on the bottom row.
///
/// Fully replaces the status bar — no mode pill, no segments. The prompt
/// character (e.g. `:`) makes the mode self-evident. The terminal cursor
/// is positioned after the input by the caller.
fn render_command_line(
    screen_buf: &mut ScreenBuf,
    colors: &crate::ui::theme::EditorColors,
    area: Rect,
    y: u16,
    prompt: char,
    input: &str,
) {

    // The command line fully replaces the status bar row — no segment layout,
    // no mode pill. The prompt character makes the mode self-evident.
    fill_row(screen_buf, colors, area, y);

    // +1: 1-column left margin, matching the leading space of the mode pill
    // in the normal status bar so the text is visually aligned.
    let cmd_str = format!("{prompt}{input}");
    screen_buf.set_string(area.x + 1, y, &cmd_str, colors.status_bar);
}

/// Render a transient status message on the bottom row.
///
/// Uses the inverted status bar style so the message stands out. The message
/// is cleared on the next keypress.
fn render_status_message(
    screen_buf: &mut ScreenBuf,
    colors: &crate::ui::theme::EditorColors,
    area: Rect,
    y: u16,
    msg: &str,
) {
    fill_row(screen_buf, colors, area, y);
    screen_buf.set_string(area.x + 1, y, msg, colors.status_bar); // +1: left margin, see render_command_line
}

/// Render the one-row status bar at the bottom of the area.
///
/// Content is driven by [`StatusLineConfig`], which lives on `Editor`. Each slot
/// (left, center, right) is a sequence of [`StatusSegment`]s that are rendered
/// into styled spans and placed at the appropriate edge of the row.
///
/// Segments within a slot are joined with a boundary-aware spacing rule:
/// - Neither boundary is a space → a single space is inserted between them.
/// - Exactly one boundary is a space → concatenated directly.
/// - Both boundaries are spaces → the new segment's leading space is trimmed
///   so there is exactly one space between the two.
///
/// Narrow-terminal degradation:
/// - Center is dropped if it would overlap left or right.
/// - Right is dropped if it would overlap left.
fn render_status_bar(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    area: Rect,
    y: u16,
) {
    let colors = &editor.colors;
    let config = &editor.statusline_config;

    // Fill the entire row with the base status bar style first.
    fill_row(screen_buf, colors, area, y);

    // Render each slot into a sequence of styled spans.
    let left_spans = render_slot(&config.left, editor);
    let center_spans = render_slot(&config.center, editor);
    let right_spans = render_slot(&config.right, editor);

    let left_w = slot_width(&left_spans);
    let center_w = slot_width(&center_spans);
    let right_w = slot_width(&right_spans);

    // Left slot: starts at the left edge.
    let left_x = area.x;
    let left_end = left_x + left_w;

    // Right slot: 1 space of right margin.
    let right_x = area.right().saturating_sub(right_w + 1);
    let right_fits = right_x >= left_end;

    // Center slot: centered in the gap between left and right.
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap = right_fence.saturating_sub(left_end);
    let center_x = left_end + gap / 2 - center_w / 2;
    let center_fits = !center_spans.is_empty()
        && center_w <= gap
        && center_x >= left_end
        && center_x + center_w <= right_fence;

    // Draw slots.
    draw_slot(screen_buf, &left_spans, left_x, y);
    if right_fits {
        draw_slot(screen_buf, &right_spans, right_x, y);
    }
    if center_fits {
        draw_slot(screen_buf, &center_spans, center_x, y);
    }
}

// ── Segment rendering ─────────────────────────────────────────────────────────

/// Render a single segment into its text and style.
///
/// Returns `(String::new(), _)` for segments that have nothing to show in the
/// current context (e.g. [`StatusSegment::Selections`] when only one selection
/// is active). The caller skips zero-width spans.
fn render_segment(seg: StatusSegment, editor: &Editor) -> (String, Style) {
    let colors = &editor.colors;
    match seg {
        StatusSegment::ModePill => {
            // The mode pill includes a leading and trailing space so its
            // neighbors don't need to add their own padding.
            let (label, style) = match (editor.mode, editor.extend) {
                (Mode::Normal, true)  => (" EXT ", colors.status_extend),
                (Mode::Normal, false) => (" NOR ", colors.status_normal),
                (Mode::Insert, _)     => (" INS ", colors.status_insert),
                // Command mode is handled by render_command_line before this is reached.
                (Mode::Command, _)    => unreachable!("render_command_line handles Command mode"),
            };
            (label.to_string(), style)
        }
        StatusSegment::Separator => ("│".to_string(), colors.status_bar),
        StatusSegment::FileName => {
            let name = editor.file_path.as_deref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("[scratch]");
            (name.to_string(), colors.status_bar)
        }
        StatusSegment::Position => {
            let buf = editor.doc.buf();
            let cursor_head = editor.doc.sels().primary().head;
            let cursor_line = buf.char_to_line(cursor_head);
            let col_0 = grapheme_col_in_line(buf, cursor_line, cursor_head);
            (format!("{}:{}", cursor_line + 1, col_0 + 1), colors.status_bar)
        }
        StatusSegment::KittyProtocol => {
            if editor.kitty_enabled {
                ("🐱".to_string(), colors.status_bar)
            } else {
                (String::new(), colors.status_bar)
            }
        }
        StatusSegment::Selections => {
            let n = editor.doc.sels().len();
            // Only show the count when there is more than one selection —
            // the single-cursor case is the default and needs no annotation.
            if n > 1 {
                (format!("{n} sels"), colors.status_bar)
            } else {
                (String::new(), colors.status_bar)
            }
        }
        StatusSegment::DirtyIndicator => {
            if editor.doc.is_dirty() {
                ("[+]".to_string(), colors.status_bar)
            } else {
                (String::new(), colors.status_bar)
            }
        }
    }
}

/// Render a slot's segments into a sequence of styled spans ready for drawing.
///
/// Skips empty segments (e.g. [`StatusSegment::Selections`] in single-cursor
/// mode) and applies the boundary-aware spacing rule between adjacent spans.
fn render_slot(segments: &[StatusSegment], editor: &Editor) -> Vec<(String, Style)> {
    // Each segment produces at most 2 spans (the segment + a possible gap span).
    let mut spans: Vec<(String, Style)> = Vec::with_capacity(segments.len() * 2);

    for &seg in segments {
        let (text, style) = render_segment(seg, editor);
        if text.is_empty() {
            continue;
        }

        if let Some((prev_text, _)) = spans.last() {
            // Boundary-aware spacing between adjacent segments:
            let a_ends_space = prev_text.ends_with(' ');
            let b_starts_space = text.starts_with(' ');

            if !a_ends_space && !b_starts_space {
                // Neither boundary is a space — insert a gap span.
                spans.push((" ".to_string(), editor.colors.status_bar));
                spans.push((text, style));
            } else if a_ends_space && b_starts_space {
                // Both boundaries are spaces — trim exactly one leading space
                // from the incoming segment so there is exactly one space
                // between them, not two. We use strip_prefix (not
                // trim_start_matches) to remove at most one space so that
                // segments intentionally padded with multiple leading spaces
                // keep their extra indent.
                let trimmed = text.strip_prefix(' ').unwrap_or(&text);
                spans.push((trimmed.to_string(), style));
            } else {
                // Exactly one boundary is a space — concatenate directly.
                spans.push((text, style));
            }
        } else {
            spans.push((text, style));
        }
    }

    spans
}

// ── Drawing helpers ───────────────────────────────────────────────────────────

/// Total display-column width of a rendered slot.
fn slot_width(spans: &[(String, Style)]) -> u16 {
    spans.iter().map(|(t, _)| UnicodeWidthStr::width(t.as_str()) as u16).sum()
}

/// Draw a slot's spans left-to-right starting at `x`.
fn draw_slot(screen_buf: &mut ScreenBuf, spans: &[(String, Style)], mut x: u16, y: u16) {
    for (text, style) in spans {
        screen_buf.set_string(x, y, text, *style);
        x += UnicodeWidthStr::width(text.as_str()) as u16;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 0-based grapheme column of `char_pos` within line `line_idx`.
///
/// This is a logical position (grapheme index), not a display column: wide
/// characters count as one, not two. The value matches how many times the
/// user pressed → to reach the cursor from the start of the line.
pub(crate) fn grapheme_col_in_line(buf: &Buffer, line_idx: usize, char_pos: usize) -> usize {
    grapheme_count(buf, buf.line_to_char(line_idx), char_pos)
}
