use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

// ── Engine integration ────────────────────────────────────────────────────────

/// A point-in-time snapshot of all statusline-visible editor state.
///
/// The editor writes a fresh snapshot once per frame (after event dispatch,
/// before `term.draw`). The engine's render path reads this snapshot through
/// the `Arc<Mutex<...>>` shared with `HumeStatusline`. Per-frame rebuild is
/// acceptable — it's a handful of clones of short strings.
pub(crate) struct StatuslineSnapshot {
    pub mode: EditorMode,
    /// `Arc` so snapshot clones are O(1) refcount bumps instead of `PathBuf` heap copies.
    pub file_path: Option<Arc<PathBuf>>,
    /// `(line_1based, col_1based)` of the primary selection head.
    pub head_pos: (usize, usize),
    pub kitty_enabled: bool,
    pub is_dirty: bool,
    /// `Some((current_1based, total))` when a search regex is active.
    pub match_count: Option<(usize, usize)>,
    pub search_wrapped: bool,
    /// Mini-buffer state when active.
    pub minibuf: Option<crate::editor::MiniBuffer>,
    /// Transient status message (shown instead of normal statusline content).
    pub status_msg: Option<String>,
    /// `Arc` so snapshot clones are O(1) refcount bumps instead of cloning 3 `Vec`s.
    pub config: Arc<StatusLineConfig>,
    pub colors: EditorColors,
}

impl StatuslineSnapshot {
    /// Blank snapshot for use before the first frame is rendered.
    ///
    /// `file_path` and `config` are the only caller-specific values; everything
    /// else gets a sensible zero/default so the statusline renders without panic.
    pub(crate) fn initial(
        file_path: Option<Arc<PathBuf>>,
        config: Arc<StatusLineConfig>,
    ) -> Self {
        Self {
            mode: EditorMode::Normal,
            file_path,
            head_pos: (1, 1),
            kitty_enabled: false,
            is_dirty: false,
            match_count: None,
            search_wrapped: false,
            minibuf: None,
            status_msg: None,
            config,
            colors: EditorColors::default(),
        }
    }

    /// Capture the current editor state into a snapshot.
    pub(crate) fn from_editor(editor: &Editor) -> Self {
        let buf = editor.doc.buf();
        let head = editor.doc.sels().primary().head;
        let head_line = buf.char_to_line(head);
        let col_0 = grapheme_col_in_line(buf, head_line, head);

        Self {
            mode: editor.mode,
            // Arc clone — O(1) refcount bump, no PathBuf heap copy.
            file_path: editor.file_path.clone(),
            head_pos: (head_line + 1, col_0 + 1),
            kitty_enabled: editor.kitty_enabled,
            is_dirty: editor.doc.is_dirty(),
            match_count: editor.search.match_count(),
            search_wrapped: editor.search.wrapped(),
            minibuf: editor.minibuf.clone(),
            status_msg: editor.status_msg.clone(),
            // Arc clone — O(1) refcount bump, no Vec heap copies.
            config: Arc::clone(&editor.statusline_config),
            colors: EditorColors::default(),
        }
    }
}

/// Engine-compatible statusline provider.
///
/// Holds a shared `Arc<Mutex<StatuslineSnapshot>>` updated by the editor
/// each frame. Implements `StatuslineProvider` so it can be registered on
/// `EditorView::statusline`.
pub(crate) struct HumeStatusline {
    pub(crate) data: Arc<Mutex<StatuslineSnapshot>>,
}

impl engine::providers::StatuslineProvider for HumeStatusline {
    fn render(
        &self,
        area: ratatui::layout::Rect,
        _theme: &engine::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    ) {
        let snap = self.data.lock().unwrap();
        let y = area.y;

        if snap.minibuf.is_none() {
            if let Some(ref msg) = snap.status_msg {
                fill_row_colors(buf, &snap.colors, area, y);
                buf.set_string(area.x + 1, y, msg, snap.colors.statusline);
                return;
            }
        }

        render_statusline_from_snapshot(buf, &snap, area, y);
    }
}

fn fill_row_colors(buf: &mut ScreenBuf, colors: &EditorColors, area: Rect, y: u16) {
    buf.set_style(Rect::new(area.x, y, area.width, 1), colors.statusline);
}

fn render_statusline_from_snapshot(
    screen_buf: &mut ScreenBuf,
    snap: &StatuslineSnapshot,
    area: Rect,
    y: u16,
) {
    let colors = &snap.colors;
    let config = &snap.config;

    let (left_elems, center_elems, right_elems): (&[StatusElement], &[StatusElement], &[StatusElement]) =
        if snap.minibuf.is_some() {
            (MINIBUF_LEFT, &[], &config.right)
        } else {
            (&config.left, &config.center, &config.right)
        };

    fill_row_colors(screen_buf, colors, area, y);

    let left_spans = pad_left(render_section_snap(left_elems, snap), colors);
    let center_spans = render_section_snap(center_elems, snap);
    let right_spans = pad_right(render_section_snap(right_elems, snap), colors);

    let left_w = section_width(&left_spans);
    let center_w = section_width(&center_spans);
    let right_w = section_width(&right_spans);

    let left_x = area.x;
    let left_end = left_x + left_w;
    let right_x = area.right().saturating_sub(right_w);
    let right_fits = right_x >= left_end;
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap = right_fence.saturating_sub(left_end);
    let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
    let center_fits = !center_spans.is_empty()
        && center_w <= gap
        && center_x >= left_end
        && center_x + center_w <= right_fence;

    draw_section(screen_buf, &left_spans, left_x, y);
    if right_fits { draw_section(screen_buf, &right_spans, right_x, y); }
    if center_fits { draw_section(screen_buf, &center_spans, center_x, y); }

    if let Some(mb) = &snap.minibuf {
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

fn render_element_snap(seg: StatusElement, snap: &StatuslineSnapshot) -> (Cow<'static, str>, Style) {
    let colors = &snap.colors;
    match seg {
        StatusElement::Mode => {
            let (label, style) = match snap.mode {
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
            let name = snap.file_path.as_deref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("[scratch]");
            (Cow::Owned(name.to_string()), colors.statusline)
        }
        StatusElement::Position => {
            let (line, col) = snap.head_pos;
            (Cow::Owned(format!("{line}:{col}")), colors.statusline)
        }
        StatusElement::KittyProtocol => {
            let label = if snap.kitty_enabled { "🐱" } else { "" };
            (Cow::Borrowed(label), colors.statusline)
        }
        StatusElement::Selections => (Cow::Borrowed(""), colors.statusline),
        StatusElement::DirtyIndicator => {
            let label = if snap.is_dirty { "[+]" } else { "" };
            (Cow::Borrowed(label), colors.statusline)
        }
        StatusElement::SearchMatches => {
            if let Some((current, total)) = snap.match_count {
                if total == 0 {
                    (Cow::Borrowed(""), colors.statusline)
                } else {
                    let w = if snap.search_wrapped { "W " } else { "" };
                    (Cow::Owned(format!("{w}[{current}/{total}]")), colors.statusline)
                }
            } else {
                (Cow::Borrowed(""), colors.statusline)
            }
        }
        StatusElement::MiniBuf => {
            if let Some(mb) = &snap.minibuf {
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

fn render_section_snap(elements: &[StatusElement], snap: &StatuslineSnapshot) -> Vec<(Cow<'static, str>, Style)> {
    let mut spans: Vec<(Cow<'static, str>, Style)> = Vec::with_capacity(elements.len() * 2);

    for &seg in elements {
        let (text, style) = render_element_snap(seg, snap);
        if text.is_empty() { continue; }

        if let Some((prev_text, _)) = spans.last() {
            let a_ends_space = prev_text.ends_with(' ');
            let b_starts_space = text.starts_with(' ');

            if !a_ends_space && !b_starts_space {
                spans.push((Cow::Borrowed(" "), snap.colors.statusline));
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
