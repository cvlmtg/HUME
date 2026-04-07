use std::borrow::Cow;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use engine::types::EditorMode;

use crate::core::grapheme::grapheme_col_in_line;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

/// Hardcoded left section for Command/Search modes.
const MINIBUF_LEFT: &[StatusElement] = &[StatusElement::MiniBuf];

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

/// Draw a section's spans left-to-right starting at `x`.
fn draw_section(screen_buf: &mut ScreenBuf, spans: &[(Cow<'static, str>, Style)], mut x: u16, y: u16) {
    for (text, style) in spans {
        screen_buf.set_string(x, y, text.as_ref(), *style);
        x += UnicodeWidthStr::width(text.as_ref()) as u16;
    }
}

// ── Engine integration ────────────────────────────────────────────────────────

/// Short-lived statusline provider that borrows `&Editor` directly.
///
/// Created each frame in `Editor::run()` and passed to `EngineView::render()`.
/// No snapshot, no Arc, no Mutex — the provider reads editor state on demand
/// during the render call.
pub(crate) struct HumeStatusline<'a> {
    pub(crate) editor: &'a Editor,
}

impl engine::providers::StatuslineProvider for HumeStatusline<'_> {
    fn render(
        &self,
        area: ratatui::layout::Rect,
        _theme: &engine::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    ) {
        let editor = self.editor;
        let colors = EditorColors::default();
        let y = area.y;

        if editor.minibuf.is_none() {
            if let Some(ref msg) = editor.status_msg {
                fill_row_colors(buf, &colors, area, y);
                buf.set_string(area.x + 1, y, msg, colors.statusline);
                return;
            }
        }

        render_statusline(buf, editor, &colors, area, y);
    }
}

fn fill_row_colors(buf: &mut ScreenBuf, colors: &EditorColors, area: Rect, y: u16) {
    buf.set_style(Rect::new(area.x, y, area.width, 1), colors.statusline);
}

fn render_statusline(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    colors: &EditorColors,
    area: Rect,
    y: u16,
) {
    let config = &editor.statusline_config;

    let (left_elems, center_elems, right_elems): (&[StatusElement], &[StatusElement], &[StatusElement]) =
        if editor.minibuf.is_some() {
            (MINIBUF_LEFT, &[], &config.right)
        } else {
            (&config.left, &config.center, &config.right)
        };

    fill_row_colors(screen_buf, colors, area, y);

    let left_spans  = pad_left(render_section(left_elems, editor, colors), colors);
    let center_spans = render_section(center_elems, editor, colors);
    let right_spans  = pad_right(render_section(right_elems, editor, colors), colors);

    let left_w   = section_width(&left_spans);
    let center_w = section_width(&center_spans);
    let right_w  = section_width(&right_spans);

    let left_x    = area.x;
    let left_end  = left_x + left_w;
    let right_x   = area.right().saturating_sub(right_w);
    let right_fits = right_x >= left_end;
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap      = right_fence.saturating_sub(left_end);
    let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
    let center_fits = !center_spans.is_empty()
        && center_w <= gap
        && center_x >= left_end
        && center_x + center_w <= right_fence;

    draw_section(screen_buf, &left_spans, left_x, y);
    if right_fits  { draw_section(screen_buf, &right_spans,  right_x,  y); }
    if center_fits { draw_section(screen_buf, &center_spans, center_x, y); }

    // Minibuf cursor is rendered by the terminal cursor (set_cursor_position +
    // set_color_for_mode in cursor.rs). No cell-level override needed.
}

fn render_element(seg: StatusElement, editor: &Editor, colors: &EditorColors) -> (Cow<'static, str>, Style) {
    match seg {
        StatusElement::Mode => {
            let (label, style) = match editor.mode {
                EditorMode::Normal  => ("NOR", colors.status_normal),
                EditorMode::Extend  => ("EXT", colors.status_extend),
                EditorMode::Insert  => ("INS", colors.status_insert),
                EditorMode::Search  => ("SRC", colors.status_search),
                EditorMode::Command => ("CMD", colors.status_command),
                EditorMode::Select  => ("SEL", colors.status_select),
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
            let head = editor.doc.sels().primary().head;
            let head_line = buf.char_to_line(head);
            let col_0 = grapheme_col_in_line(buf, head_line, head);
            (Cow::Owned(format!("{}:{}", head_line + 1, col_0 + 1)), colors.statusline)
        }
        StatusElement::KittyProtocol => {
            let label = if editor.kitty_enabled { "🐱" } else { "" };
            (Cow::Borrowed(label), colors.statusline)
        }
        StatusElement::Selections => (Cow::Borrowed(""), colors.statusline),
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

fn render_section(elements: &[StatusElement], editor: &Editor, colors: &EditorColors) -> Vec<(Cow<'static, str>, Style)> {
    let mut spans: Vec<(Cow<'static, str>, Style)> = Vec::with_capacity(elements.len() * 2);

    for &seg in elements {
        let (text, style) = render_element(seg, editor, colors);
        if text.is_empty() { continue; }

        if let Some((prev_text, _)) = spans.last() {
            let a_ends_space   = prev_text.ends_with(' ');
            let b_starts_space = text.starts_with(' ');

            if !a_ends_space && !b_starts_space {
                spans.push((Cow::Borrowed(" "), colors.statusline));
                spans.push((text, style));
            } else if a_ends_space && b_starts_space {
                let trimmed = match &text {
                    Cow::Borrowed(s) => Cow::Borrowed(s.strip_prefix(' ').unwrap_or(s)),
                    Cow::Owned(s) => Cow::Owned(s.strip_prefix(' ').unwrap_or(s.as_str()).to_string()),
                };
                spans.push((trimmed, style));
            } else {
                spans.push((text, style));
            }
        } else {
            spans.push((text, style));
        }
    }

    spans
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
