use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::buffer::Buffer;
use crate::core::grapheme::grapheme_count;
use crate::editor::{Editor, Mode};

/// Hardcoded left section for Command/Search modes.
const MINIBUF_LEFT: &[StatusElement] = &[StatusElement::Mode, StatusElement::Separator, StatusElement::MiniBuf];

/// Fill an entire statusline row with spaces in the base style.
///
/// Both bottom-row renderers do this as their first step to clear
/// whatever was drawn in the previous frame.
fn fill_row(screen_buf: &mut ScreenBuf, colors: &crate::ui::theme::EditorColors, area: Rect, y: u16) {
    let blank = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, colors.statusline);
}

// ── Configuration ─────────────────────────────────────────────────────────────

/// A named element that can appear in a statusline section.
///
/// Elements are the building blocks of the statusline. The mode indicator,
/// separators, and data fields are all first-class element variants —
/// there is no special chrome. You control the layout by choosing which
/// elements appear in each section and in what order.
///
/// The Steel scripting layer constructs [`StatusLineConfig`] values at
/// runtime; this enum is the wire format for those configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusElement {
    /// The mode indicator: `" NOR "`, `" INS "`, `" EXT "`, `" CMD "`, or `" SRC "`.
    ///
    /// Rendered with the per-mode color. The 5-char width includes leading and
    /// trailing spaces so it naturally fills a section without needing a separator.
    Mode,
    /// A thin vertical bar `│` in the base statusline style.
    ///
    /// Place this explicitly between elements that need a visual divider.
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
    /// Search match count: `"[3/42]"` when a search regex is active, empty otherwise.
    ///
    /// The current index is 1-based — the match whose range contains the primary
    /// cursor head. Shows `0` when the cursor is between matches (e.g. the live
    /// search has no hit yet).
    SearchMatches,
    /// The mini-buffer input field: prompt character followed by typed text.
    ///
    /// Rendered only when `editor.minibuf` is `Some`. Produces the prompt
    /// character followed by the input text. The block cursor within the
    /// input is applied as a post-render patch in [`render_statusline`].
    MiniBuf,
}

/// Describes the content layout of the statusline's three sections.
///
/// Each section is a sequence of [`StatusElement`]s rendered in order. Adjacent
/// elements within a section are joined with a boundary-aware spacing rule so
/// that spacing feels natural without any element needing to hard-code its
/// neighbours.
///
/// The default config reproduces the built-in statusline layout exactly, so
/// the editor looks identical with no configuration:
///
/// ```text
///  NOR │ notes.txt              42:7
/// ```
#[derive(Debug, Clone)]
pub(crate) struct StatusLineConfig {
    /// Elements rendered left-aligned at the start of the statusline row.
    pub left: Vec<StatusElement>,
    /// Elements centered between the left and right sections. Empty by default.
    pub center: Vec<StatusElement>,
    /// Elements rendered right-aligned at the end of the statusline row.
    pub right: Vec<StatusElement>,
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        Self {
            left: vec![StatusElement::Mode, StatusElement::Separator, StatusElement::FileName, StatusElement::DirtyIndicator],
            center: vec![],
            right: vec![StatusElement::KittyProtocol, StatusElement::SearchMatches, StatusElement::Position],
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Render the bottom row of the terminal: status message or statusline.
///
/// When a mini-buffer is active, the statusline uses a hardcoded layout with a
/// [`StatusElement::MiniBuf`] element instead of the user's normal config.
/// Transient status messages are shown only when the mini-buffer is inactive.
pub(crate) fn render_bottom_row(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    area: Rect,
    y: u16,
) {
    if let (None, Some(msg)) = (&editor.minibuf, &editor.status_msg) {
        render_status_message(screen_buf, &editor.colors, area, y, msg);
    } else {
        render_statusline(screen_buf, editor, area, y);
    }
}

// ── Renderers ─────────────────────────────────────────────────────────────────

/// Render a transient status message on the bottom row.
///
/// Uses the inverted statusline style so the message stands out. The message
/// is cleared on the next keypress.
fn render_status_message(
    screen_buf: &mut ScreenBuf,
    colors: &crate::ui::theme::EditorColors,
    area: Rect,
    y: u16,
    msg: &str,
) {
    fill_row(screen_buf, colors, area, y);
    screen_buf.set_string(area.x + 1, y, msg, colors.statusline); // +1: left margin
}

/// Render the one-row statusline at the bottom of the area.
///
/// Content is driven by [`StatusLineConfig`], which lives on `Editor`. Each
/// section (left, center, right) is a sequence of [`StatusElement`]s rendered
/// into styled spans and placed at the appropriate edge of the row.
///
/// Elements within a section are joined with a boundary-aware spacing rule:
/// - Neither boundary is a space → a single space is inserted between them.
/// - Exactly one boundary is a space → concatenated directly.
/// - Both boundaries are spaces → the new element's leading space is trimmed
///   so there is exactly one space between the two.
///
/// Narrow-terminal degradation:
/// - Center is dropped if it would overlap left or right.
/// - Right is dropped if it would overlap left.
fn render_statusline(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    area: Rect,
    y: u16,
) {
    let colors = &editor.colors;
    let config = &editor.statusline_config;

    // When a mini-buffer is active, swap to a hardcoded layout that shows the
    // mode pill + input field on the left, while preserving the user's right
    // section (which may include SearchMatches, Position, etc.).
    let (left_elems, center_elems, right_elems): (&[StatusElement], &[StatusElement], &[StatusElement]) =
        if editor.minibuf.is_some() {
            (MINIBUF_LEFT, &[], &config.right)
        } else {
            (&config.left, &config.center, &config.right)
        };

    // Fill the entire row with the base statusline style first.
    fill_row(screen_buf, colors, area, y);

    // Render each section into a sequence of styled spans.
    let left_spans = render_section(left_elems, editor);
    let center_spans = render_section(center_elems, editor);
    let right_spans = render_section(right_elems, editor);

    let left_w = section_width(&left_spans);
    let center_w = section_width(&center_spans);
    let right_w = section_width(&right_spans);

    // Left section: starts at the left edge.
    let left_x = area.x;
    let left_end = left_x + left_w;

    // Right section: 1 space of right margin.
    let right_x = area.right().saturating_sub(right_w + 1);
    let right_fits = right_x >= left_end;

    // Center section: centered in the gap between left and right.
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap = right_fence.saturating_sub(left_end);
    let center_x = left_end + gap / 2 - center_w / 2;
    let center_fits = !center_spans.is_empty()
        && center_w <= gap
        && center_x >= left_end
        && center_x + center_w <= right_fence;

    // Draw sections.
    draw_section(screen_buf, &left_spans, left_x, y);
    if right_fits {
        draw_section(screen_buf, &right_spans, right_x, y);
    }
    if center_fits {
        draw_section(screen_buf, &center_spans, center_x, y);
    }

    // MiniBuf cursor: style one cell to show a visible block against the
    // reversed statusline background. remove_modifier(REVERSED) clears the
    // reversed bit, leaving terminal-default colors — a visible block.
    // This workaround goes away once the theme uses explicit bg colors.
    if let Some(mb) = &editor.minibuf {
        let mb_offset = last_span_offset(&left_spans);
        let prompt_w = mb.prompt.width().unwrap_or(0) as u16;
        let input_before_cursor = UnicodeWidthStr::width(&mb.input[..mb.cursor]) as u16;
        let cursor_x = left_x + mb_offset + prompt_w + input_before_cursor;
        if cursor_x < area.right() {
            let ch = mb.input[mb.cursor..].graphemes(true).next().unwrap_or(" ");
            let cursor_style = Style::new().remove_modifier(Modifier::REVERSED);
            screen_buf.set_string(cursor_x, y, ch, cursor_style);
        }
    }
}

// ── Element rendering ─────────────────────────────────────────────────────────

/// Render a single element into its text and style.
///
/// Returns `(String::new(), _)` for elements that have nothing to show in the
/// current context (e.g. [`StatusElement::Selections`] when only one selection
/// is active). The caller skips zero-width spans.
fn render_element(seg: StatusElement, editor: &Editor) -> (String, Style) {
    let colors = &editor.colors;
    match seg {
        StatusElement::Mode => {
            // The mode indicator includes a leading and trailing space so its
            // neighbors don't need to add their own padding.
            let (label, style) = match (editor.mode, editor.extend) {
                (Mode::Normal, true)  => (" EXT ", colors.status_extend),
                (Mode::Normal, false) => (" NOR ", colors.status_normal),
                (Mode::Insert, _)     => (" INS ", colors.status_insert),
                (Mode::Search, _)     => (" SRC ", colors.status_search),
                (Mode::Command, _)    => (" CMD ", colors.status_command),
            };
            (label.to_string(), style)
        }
        StatusElement::Separator => ("│".to_string(), colors.statusline),
        StatusElement::FileName => {
            let name = editor.file_path.as_deref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("[scratch]");
            (name.to_string(), colors.statusline)
        }
        StatusElement::Position => {
            let buf = editor.doc.buf();
            let cursor_head = editor.doc.sels().primary().head;
            let cursor_line = buf.char_to_line(cursor_head);
            let col_0 = grapheme_col_in_line(buf, cursor_line, cursor_head);
            (format!("{}:{}", cursor_line + 1, col_0 + 1), colors.statusline)
        }
        StatusElement::KittyProtocol => {
            if editor.kitty_enabled {
                ("🐱".to_string(), colors.statusline)
            } else {
                (String::new(), colors.statusline)
            }
        }
        StatusElement::Selections => {
            let n = editor.doc.sels().len();
            // Only show the count when there is more than one selection —
            // the single-cursor case is the default and needs no annotation.
            if n > 1 {
                (format!("{n} sels"), colors.statusline)
            } else {
                (String::new(), colors.statusline)
            }
        }
        StatusElement::DirtyIndicator => {
            if editor.doc.is_dirty() {
                ("[+]".to_string(), colors.statusline)
            } else {
                (String::new(), colors.statusline)
            }
        }
        StatusElement::SearchMatches => {
            if let Some((current, total)) = editor.search.match_count() {
                let w = if editor.search.wrapped() { "W " } else { "" };
                (format!("{w}[{current}/{total}]"), colors.statusline)
            } else {
                (String::new(), colors.statusline)
            }
        }
        StatusElement::MiniBuf => {
            if let Some(mb) = &editor.minibuf {
                let mut text = String::with_capacity(1 + mb.input.len());
                text.push(mb.prompt);
                text.push_str(&mb.input);
                (text, colors.statusline)
            } else {
                (String::new(), colors.statusline)
            }
        }
    }
}

/// Render a section's elements into a sequence of styled spans ready for drawing.
///
/// Skips empty elements (e.g. [`StatusElement::Selections`] in single-cursor
/// mode) and applies the boundary-aware spacing rule between adjacent spans.
fn render_section(elements: &[StatusElement], editor: &Editor) -> Vec<(String, Style)> {
    // Each element produces at most 2 spans (the element + a possible gap span).
    let mut spans: Vec<(String, Style)> = Vec::with_capacity(elements.len() * 2);

    for &seg in elements {
        let (text, style) = render_element(seg, editor);
        if text.is_empty() {
            continue;
        }

        if let Some((prev_text, _)) = spans.last() {
            // Boundary-aware spacing between adjacent elements:
            let a_ends_space = prev_text.ends_with(' ');
            let b_starts_space = text.starts_with(' ');

            if !a_ends_space && !b_starts_space {
                // Neither boundary is a space — insert a gap span.
                spans.push((" ".to_string(), editor.colors.statusline));
                spans.push((text, style));
            } else if a_ends_space && b_starts_space {
                // Both boundaries are spaces — trim exactly one leading space
                // from the incoming element so there is exactly one space
                // between them, not two. We use strip_prefix (not
                // trim_start_matches) to remove at most one space so that
                // elements intentionally padded with multiple leading spaces
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

/// Total display-column width of a rendered section.
fn section_width(spans: &[(String, Style)]) -> u16 {
    spans.iter().map(|(t, _)| UnicodeWidthStr::width(t.as_str()) as u16).sum()
}

/// Display-column offset of the last span in a section.
///
/// Used to locate the MiniBuf span, which is always the last element in the
/// minibuf left section (`MINIBUF_LEFT`).
fn last_span_offset(spans: &[(String, Style)]) -> u16 {
    let total = section_width(spans);
    let last_w = spans.last().map(|(t, _)| UnicodeWidthStr::width(t.as_str()) as u16).unwrap_or(0);
    total - last_w
}

/// Draw a section's spans left-to-right starting at `x`.
fn draw_section(screen_buf: &mut ScreenBuf, spans: &[(String, Style)], mut x: u16, y: u16) {
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
