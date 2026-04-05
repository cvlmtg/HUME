use std::borrow::Cow;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::grapheme::grapheme_col_in_line;
use crate::editor::{Editor, Mode};

/// Hardcoded left section for Command/Search modes.
const MINIBUF_LEFT: &[StatusElement] = &[StatusElement::MiniBuf];

/// Apply the base statusline style across an entire row.
///
/// Both bottom-row renderers do this as their first step so that the
/// background color is uniform before individual spans are drawn on top.
/// Uses `set_style` on the row rect instead of allocating a blank string.
fn fill_row(screen_buf: &mut ScreenBuf, colors: &crate::ui::theme::EditorColors, area: Rect, y: u16) {
    screen_buf.set_style(Rect::new(area.x, y, area.width, 1), colors.statusline);
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
    /// The mode indicator: `"NOR"`, `"INS"`, `"EXT"`, `"CMD"`, `"SRC"`, or `"SEL"`.
    ///
    /// Rendered with the per-mode color. Contains no padding — the renderer's
    /// edge padding and inter-element spacing handle surrounding whitespace.
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
/// 42:7 notes.txt              │ NOR
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
            left: vec![
                StatusElement::Position,
                StatusElement::FileName,
                StatusElement::DirtyIndicator,
            ],
            center: vec![],
            right: vec![
                StatusElement::SearchMatches,
                StatusElement::KittyProtocol,
                StatusElement::Separator,
                StatusElement::Mode,
            ],
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

    // Render each section into a sequence of styled spans, then pad the
    // outer edges: one space before the first left element and one space
    // after the last right element. This keeps individual elements free
    // of edge-awareness — they only worry about inter-element spacing.
    let left_spans = pad_left(render_section(left_elems, editor), colors);
    let center_spans = render_section(center_elems, editor);
    let right_spans = pad_right(render_section(right_elems, editor), colors);

    let left_w = section_width(&left_spans);
    let center_w = section_width(&center_spans);
    let right_w = section_width(&right_spans);

    // Left section: starts at the left edge.
    let left_x = area.x;
    let left_end = left_x + left_w;

    // Right section: padding is already included in the spans.
    let right_x = area.right().saturating_sub(right_w);
    let right_fits = right_x >= left_end;

    // Center section: centered in the gap between left and right.
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap = right_fence.saturating_sub(left_end);
    let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
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
fn render_element(seg: StatusElement, editor: &Editor) -> (Cow<'static, str>, Style) {
    let colors = &editor.colors;
    match seg {
        StatusElement::Mode => {
            let (label, style) = match editor.mode {
                Mode::Normal   => ("NOR", colors.status_normal),
                Mode::Extend   => ("EXT", colors.status_extend),
                Mode::Insert   => ("INS", colors.status_insert),
                Mode::Search   => ("SRC", colors.status_search),
                Mode::Command  => ("CMD", colors.status_command),
                Mode::Select   => ("SEL", colors.status_select),
            };
            (Cow::Borrowed(label), style)
        }
        StatusElement::Separator => (Cow::Borrowed("│"), colors.statusline),
        StatusElement::FileName => {
            let name = editor.file_path.as_deref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("[scratch]");
            (Cow::Owned(name.to_string()), colors.statusline)
        }
        StatusElement::Position => {
            let buf = editor.doc.buf();
            let cursor_head = editor.doc.sels().primary().head;
            let cursor_line = buf.char_to_line(cursor_head);
            let col_0 = grapheme_col_in_line(buf, cursor_line, cursor_head);
            (Cow::Owned(format!("{}:{}", cursor_line + 1, col_0 + 1)), colors.statusline)
        }
        StatusElement::KittyProtocol => {
            let label = if editor.kitty_enabled { "🐱" } else { "" };
            (Cow::Borrowed(label), colors.statusline)
        }
        StatusElement::Selections => {
            let n = editor.doc.sels().len();
            // Only show the count when there is more than one selection —
            // the single-cursor case is the default and needs no annotation.
            if n > 1 {
                (Cow::Owned(format!("{n} sels")), colors.statusline)
            } else {
                (Cow::Borrowed(""), colors.statusline)
            }
        }
        StatusElement::DirtyIndicator => {
            let label = if editor.doc.is_dirty() { "[+]" } else { "" };
            (Cow::Borrowed(label), colors.statusline)
        }
        StatusElement::SearchMatches => {
            if let Some((current, total)) = editor.search.match_count() {
                if total == 0 {
                    (Cow::Borrowed(""), colors.statusline)
                } else {
                    let w = if editor.search.wrapped() { "W " } else { "" };
                    (Cow::Owned(format!("{w}[{current}/{total}]")), colors.statusline)
                }
            } else {
                (Cow::Borrowed(""), colors.statusline)
            }
        }
        StatusElement::MiniBuf => {
            if let Some(mb) = &editor.minibuf {
                let mut text = String::with_capacity(2 + mb.input.len());
                text.push(mb.prompt);
                text.push_str(&mb.input);
                (Cow::Owned(text), colors.statusline)
            } else {
                (Cow::Borrowed(""), colors.statusline)
            }
        }
    }
}

/// Render a section's elements into a sequence of styled spans ready for drawing.
///
/// Skips empty elements (e.g. [`StatusElement::Selections`] in single-cursor
/// mode) and applies the boundary-aware spacing rule between adjacent spans.
fn render_section(elements: &[StatusElement], editor: &Editor) -> Vec<(Cow<'static, str>, Style)> {
    // Each element produces at most 2 spans (the element + a possible gap span).
    let mut spans: Vec<(Cow<'static, str>, Style)> = Vec::with_capacity(elements.len() * 2);

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
                spans.push((Cow::Borrowed(" "), editor.colors.statusline));
                spans.push((text, style));
            } else if a_ends_space && b_starts_space {
                // Both boundaries are spaces — trim exactly one leading space
                // from the incoming element so there is exactly one space
                // between them, not two. We use strip_prefix (not
                // trim_start_matches) to remove at most one space so that
                // elements intentionally padded with multiple leading spaces
                // keep their extra indent.
                let trimmed = match &text {
                    Cow::Borrowed(s) => Cow::Borrowed(s.strip_prefix(' ').unwrap_or(s)),
                    Cow::Owned(s) => Cow::Owned(s.strip_prefix(' ').unwrap_or(s.as_str()).to_string()),
                };
                spans.push((trimmed, style));
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

// ── Edge padding ─────────────────────────────────────────────────────────────

/// Prepend a 1-space padding span to the left section.
///
/// This gives every left section a consistent left margin without requiring
/// individual elements to be edge-aware.
fn pad_left(
    mut spans: Vec<(Cow<'static, str>, Style)>,
    colors: &crate::ui::theme::EditorColors,
) -> Vec<(Cow<'static, str>, Style)> {
    if !spans.is_empty() {
        spans.insert(0, (Cow::Borrowed(" "), colors.statusline));
    }
    spans
}

/// Append a 1-space padding span to the right section.
///
/// Mirrors [`pad_left`] for the trailing edge, replacing the old hardcoded
/// `+1` right-margin offset in the placement arithmetic.
fn pad_right(
    mut spans: Vec<(Cow<'static, str>, Style)>,
    colors: &crate::ui::theme::EditorColors,
) -> Vec<(Cow<'static, str>, Style)> {
    if !spans.is_empty() {
        spans.push((Cow::Borrowed(" "), colors.statusline));
    }
    spans
}

// ── Drawing helpers ───────────────────────────────────────────────────────────

/// Total display-column width of a rendered section.
fn section_width(spans: &[(Cow<'static, str>, Style)]) -> u16 {
    spans.iter().map(|(t, _)| UnicodeWidthStr::width(t.as_ref()) as u16).sum()
}

/// Display-column offset of the last span in a section.
///
/// Used to locate the MiniBuf span, which is always the last element in the
/// minibuf left section (`MINIBUF_LEFT`).
fn last_span_offset(spans: &[(Cow<'static, str>, Style)]) -> u16 {
    let total = section_width(spans);
    let last_w = spans.last().map(|(t, _)| UnicodeWidthStr::width(t.as_ref()) as u16).unwrap_or(0);
    total - last_w
}

/// Draw a section's spans left-to-right starting at `x`.
fn draw_section(screen_buf: &mut ScreenBuf, spans: &[(Cow<'static, str>, Style)], mut x: u16, y: u16) {
    for (text, style) in spans {
        screen_buf.set_string(x, y, text.as_ref(), *style);
        x += UnicodeWidthStr::width(text.as_ref()) as u16;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn s(text: &'static str) -> (Cow<'static, str>, Style) {
        (Cow::Borrowed(text), Style::default())
    }

    // ── section_width ─────────────────────────────────────────────────────────

    #[test]
    fn section_width_empty() {
        assert_eq!(section_width(&[]), 0);
    }

    #[test]
    fn section_width_ascii() {
        let spans = vec![s("NOR"), s(" "), s("│")];
        // "NOR"=3, " "=1, "│"=1 (U+2502 is width 1)
        assert_eq!(section_width(&spans), 5);
    }

    #[test]
    fn section_width_cjk() {
        // CJK character is display-width 2.
        let spans = vec![s("A"), (Cow::Borrowed("中"), Style::default())];
        assert_eq!(section_width(&spans), 3);
    }

    // ── last_span_offset ─────────────────────────────────────────────────────

    #[test]
    fn last_span_offset_single_span() {
        // Only one span: offset is 0 (nothing before it).
        let spans = vec![s("abc")];
        assert_eq!(last_span_offset(&spans), 0);
    }

    #[test]
    fn last_span_offset_multiple_spans() {
        // " " (1) + ":" (1) + "cmd" (3) → last span starts at offset 2.
        let spans = vec![s(" "), s(":"), s("cmd")];
        assert_eq!(last_span_offset(&spans), 2);
    }

    #[test]
    fn last_span_offset_empty() {
        assert_eq!(last_span_offset(&[]), 0);
    }

    // ── pad_left / pad_right ──────────────────────────────────────────────────

    #[test]
    fn pad_left_prepends_space() {
        let colors = crate::ui::theme::EditorColors::default();
        let spans = pad_left(vec![s("NOR")], &colors);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0.as_ref(), " ");
        assert_eq!(spans[1].0.as_ref(), "NOR");
    }

    #[test]
    fn pad_left_empty_is_noop() {
        let colors = crate::ui::theme::EditorColors::default();
        let spans = pad_left(vec![], &colors);
        assert!(spans.is_empty());
    }

    #[test]
    fn pad_right_appends_space() {
        let colors = crate::ui::theme::EditorColors::default();
        let spans = pad_right(vec![s("NOR")], &colors);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0.as_ref(), "NOR");
        assert_eq!(spans[1].0.as_ref(), " ");
    }

    #[test]
    fn pad_right_empty_is_noop() {
        let colors = crate::ui::theme::EditorColors::default();
        let spans = pad_right(vec![], &colors);
        assert!(spans.is_empty());
    }

    // ── center_x arithmetic ───────────────────────────────────────────────────

    #[test]
    fn center_x_saturates_on_overflow() {
        // When center_w > gap, saturating_sub prevents u16 wrapping.
        // gap/2=1, center_w/2=5 → without saturating_sub this would wrap.
        let left_end: u16 = 5;
        let gap: u16 = 2;
        let center_w: u16 = 10;
        let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
        // Should not panic and should produce a value ≤ left_end (saturated to 0 at best).
        assert!(center_x <= left_end);
    }
}
