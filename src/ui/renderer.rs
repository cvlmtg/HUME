use std::borrow::Cow;

use crossterm::cursor::SetCursorStyle;
use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::selection::Selection;
use crate::editor::{Editor, Mode};
use crate::ops::text_object::find_bracket_pair;
use crate::ui::display_line::DisplayLine;
use crate::ui::highlight::HighlightSet;
use crate::ui::statusline::{grapheme_col_in_line, render_bottom_row};
use crate::ui::theme::EditorColors;
use crate::ui::view::LineNumberStyle;

// ── Public API ────────────────────────────────────────────────────────────────

/// The terminal cursor position to apply inside `frame.draw()`.
///
/// `None` in Normal mode — the visual `cursor_head` cell acts as the cursor,
/// so the real terminal cursor is hidden. `Some` in Insert and Command modes
/// where the bar cursor needs to track the editing position.
pub(crate) struct CursorState {
    pub pos: Option<(u16, u16)>,
}

/// The cursor shape to emit **after** `term.draw()`.
///
/// Must be the last escape sequence before blocking — ratatui's `ShowCursor`
/// flush can otherwise reset the shape on some terminals.
pub(crate) fn cursor_style(mode: Mode) -> SetCursorStyle {
    match mode {
        Mode::Normal => SetCursorStyle::SteadyBlock,
        Mode::Insert | Mode::Command | Mode::Search => SetCursorStyle::SteadyBar,
    }
}

/// Render the current editor state into a ratatui screen buffer.
///
/// `area` is the full terminal area (including the status bar row).
/// The renderer splits it via [`layout`] into gutter, content, and status bar.
///
/// Highlights (bracket match, future search hits) are computed internally from
/// `editor` state — ephemeral per-frame values, not part of the public API.
///
/// Returns [`CursorState`] for the caller to apply inside the draw closure.
/// The cursor shape must be applied after the draw call via [`cursor_style`].
///
/// This function is pure: it only writes to `screen_buf` and reads from its
/// arguments. All terminal I/O is handled by the caller.
pub(crate) fn render(editor: &Editor, area: Rect, screen_buf: &mut ScreenBuf) -> CursorState {
    let lay = layout(area, editor.view.gutter_width as u16);
    let highlights = compute_highlights(editor);

    let buf = editor.doc.buf();
    let display_lines = editor.view.display_lines(buf);

    // ── Content rows ──────────────────────────────────────────────────────────

    for row in 0..editor.view.height {
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }

        if let Some(dl) = display_lines.get(row) {
            render_gutter(screen_buf, editor, dl, lay.gutter.x, y);
            render_content(screen_buf, editor, &highlights, dl, lay.content.x, y, lay.content.width);
        } else {
            // Past end of buffer — draw `~` in the gutter column.
            screen_buf.set_string(area.x, y, "~", editor.colors.tilde);
        }
    }

    // ── Bottom row (status bar / command line / status message) ───────────────

    if lay.status_bar.y < area.bottom() {
        render_bottom_row(screen_buf, editor, area, lay.status_bar.y);
    }

    CursorState { pos: compute_cursor_pos(editor) }
}

// ── Layout ────────────────────────────────────────────────────────────────────

struct Layout {
    gutter: Rect,
    content: Rect,
    status_bar: Rect,
}

/// Divide the terminal area into gutter, content, and status bar regions.
///
/// Spatial relationships are explicit here rather than scattered across
/// individual render functions. When splits/panes arrive, each pane gets its
/// own `Layout` computed from its allocated sub-area.
fn layout(area: Rect, gutter_width: u16) -> Layout {
    let content_height = area.height.saturating_sub(1);
    let gutter = Rect::new(area.x, area.y, gutter_width, content_height);
    let content = Rect::new(
        area.x + gutter_width,
        area.y,
        area.width.saturating_sub(gutter_width),
        content_height,
    );
    let status_bar = Rect::new(area.x, area.y + content_height, area.width, 1);
    Layout { gutter, content, status_bar }
}

// ── Highlights ────────────────────────────────────────────────────────────────

/// Compute per-frame highlights from editor state.
///
/// Produces bracket-match highlights when the primary cursor sits on a bracket
/// in Normal or Search mode. Insert mode suppresses bracket matching — the bar
/// cursor doesn't "sit on" a character the same way.
///
/// When a search regex is cached (during Search mode or after confirming a
/// search for use with `n`/`N`), all match ranges are highlighted with
/// `editor.colors.search_match`. The primary match is already shown as a
/// selection, so overlapping search highlights will be visually overridden by
/// the selection/cursor colors in [`resolve_style`].
fn compute_highlights(editor: &Editor) -> HighlightSet {
    if editor.mode == Mode::Insert {
        // Zero allocation: building an empty set is trivial.
        return HighlightSet::new().build();
    }

    let head = editor.doc.sels().primary().head;
    let mut hl = HighlightSet::new();

    // ── Search match highlights ───────────────────────────────────────────────
    for &(start, end_incl) in &editor.search_matches {
        hl.push(start, end_incl, editor.colors.search_match);
    }

    // ── Bracket match highlight ───────────────────────────────────────────────
    if let Some(ch) = editor.doc.buf().char_at(head) {
        let pair = match ch {
            '(' | ')' => Some(('(', ')')),
            '[' | ']' => Some(('[', ']')),
            '{' | '}' => Some(('{', '}')),
            '<' | '>' => Some(('<', '>')),
            _ => None,
        };
        if let Some((open, close)) = pair
            && let Some((op, cp)) = find_bracket_pair(editor.doc.buf(), head, open, close)
        {
            // Highlight the OTHER bracket — the cursor already marks the one it's on.
            let match_pos = if head == op { cp } else { op };
            hl.push(match_pos, match_pos, editor.colors.bracket_match);
        }
    }

    hl.build()
}

// ── Cursor ────────────────────────────────────────────────────────────────────

/// Compute the terminal cursor position for the current editor state.
///
/// Returns `None` in Normal mode — the visual `cursor_head` cell acts as the
/// cursor; the real terminal cursor should be hidden.
fn compute_cursor_pos(editor: &Editor) -> Option<(u16, u16)> {
    match editor.mode {
        Mode::Normal => None,
        Mode::Insert => cursor_screen_pos(editor),
        // Command/Search use a visual block cursor rendered directly onto the
        // status-bar cell (see render_command_line). No terminal cursor needed.
        Mode::Command | Mode::Search => None,
    }
}

/// Map the primary cursor to screen (col, row), or `None` if scrolled out.
fn cursor_screen_pos(editor: &Editor) -> Option<(u16, u16)> {
    let buf = editor.doc.buf();
    let view = &editor.view;
    let head = editor.doc.sels().primary().head;
    let cursor_line = buf.char_to_line(head);
    let screen_row = cursor_line.checked_sub(view.scroll_offset)?;
    if screen_row >= view.height {
        return None;
    }
    let col = grapheme_col_in_line(buf, cursor_line, head);
    Some(((view.gutter_width + col) as u16, screen_row as u16))
}

// ── Gutter ────────────────────────────────────────────────────────────────────

/// Render the line-number gutter cell for one display row.
///
/// The label (absolute or relative number) is right-aligned in
/// `gutter_width - 1` columns, followed by one space separator.
/// Non-cursor lines are dimmed; the cursor line keeps the default style
/// so it stands out.
fn render_gutter(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    dl: &DisplayLine<'_>,
    x: u16,
    y: u16,
) {
    // Virtual lines have no line number — nothing to render in the gutter.
    let Some(line_number) = dl.line_number else { return };
    let line_idx = line_number.saturating_sub(1); // 0-based

    let cursor_line = editor.doc.buf().char_to_line(editor.doc.sels().primary().head);
    let view = &editor.view;
    let label = match view.line_number_style {
        LineNumberStyle::Absolute => format!("{line_number}"),
        LineNumberStyle::Relative => format!("{}", line_idx.abs_diff(cursor_line)),
        LineNumberStyle::Hybrid => {
            if line_idx == cursor_line {
                format!("{line_number}")
            } else {
                format!("{}", line_idx.abs_diff(cursor_line))
            }
        }
    };

    // Right-align the label in `gutter_width - 1` columns, then one space.
    let gutter_str = format!("{:>width$} ", label, width = view.gutter_width.saturating_sub(1));

    let colors = &editor.colors;
    let style = if line_idx == cursor_line { colors.gutter_cursor_line } else { colors.gutter };
    screen_buf.set_string(x, y, &gutter_str, style);
}

// ── Content ───────────────────────────────────────────────────────────────────

/// Resolve the style for a single character at `char_pos`.
///
/// Priority (highest first):
/// 1. `cursor_head`  — the selection's head position (the actual cursor cell)
/// 2. `selection`    — other chars within any selection's inclusive range
/// 3. `highlights`   — bracket match, search hits, diagnostics
/// 4. (future slot)  — syntax highlighting would go here
/// 5. `cursor_line`  — subtle bg tint on the primary cursor's line
/// 6. `default`      — no decoration
///
/// In Insert mode `show_sels` is false — all selection/cursor highlights are
/// suppressed and the real terminal bar cursor is used instead.
fn resolve_style(
    char_pos: usize,
    colors: &EditorColors,
    highlights: &HighlightSet,
    sels_on_line: &[Selection],
    is_cursor_line: bool,
    show_sels: bool,
) -> Style {
    if show_sels && sels_on_line.iter().any(|s| char_pos == s.head) {
        return colors.cursor_head;
    }
    if show_sels && sels_on_line.iter().any(|s| char_pos >= s.start() && char_pos <= s.end()) {
        return colors.selection;
    }
    if let Some(hl) = highlights.style_at(char_pos) {
        return hl;
    }
    // Future: syntax highlighting goes here (between highlights and cursor_line).
    if is_cursor_line {
        return colors.cursor_line;
    }
    colors.default
}

/// Render the text content of one display line into the screen buffer.
///
/// Iterates grapheme clusters (via `unicode-segmentation`) so that multi-byte
/// characters and combining sequences are treated as single units. Display
/// widths come from `unicode-width` so CJK double-width characters consume
/// exactly 2 columns. Style resolution is delegated to [`resolve_style`].
fn render_content(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    highlights: &HighlightSet,
    dl: &DisplayLine<'_>,
    x: u16,
    y: u16,
    width: u16,
) {
    let mode = editor.mode;
    let colors = &editor.colors;
    let sels = editor.doc.sels();
    let cursor_line = editor.doc.buf().char_to_line(sels.primary().head);
    let char_offset = dl.char_offset.unwrap_or(0);

    // line_end_excl = position of the stripped '\n' (one past the last content char).
    let content_chars = dl.content.len_chars();
    let line_end_excl = char_offset + content_chars;

    // Collect selections that overlap this display line once so the per-grapheme
    // style checks can iterate a tiny local slice rather than re-filtering the
    // full set each time. Selection is Copy (two usizes), and the count per line
    // is typically 1, so this Vec rarely exceeds a single inline allocation.
    let sels_on_line: Vec<Selection> = sels
        .iter_sorted()
        .filter(|s| s.end() >= char_offset && s.start() <= line_end_excl)
        .copied()
        .collect();

    if sels_on_line.is_empty() && dl.char_offset.is_none() {
        return; // virtual line with no selection overlap — nothing to render
    }

    // Whether this display line is the primary cursor's line. Used for the
    // cursor-line background tint (lowest priority in the style chain).
    let is_cursor_line_row = dl.line_number.is_some_and(|ln| ln.saturating_sub(1) == cursor_line);

    // Pre-fill the content area with the cursor-line bg so empty space at the
    // end of the line also gets the tint. Individual cells are overwritten below
    // with higher-priority styles (selection, cursor head).
    if is_cursor_line_row {
        // set_style paints the style onto existing cells without allocating.
        screen_buf.set_style(Rect::new(x, y, width, 1), colors.cursor_line);
    }

    // Borrow the line content as &str when the rope slice is contiguous (the
    // common case for typical line lengths), falling back to an owned String
    // only when the line spans multiple rope chunks.
    let content_cow: Cow<str> = dl.content.into();
    let mut col: u16 = 0;
    let mut char_pos = char_offset;

    // Show selections in Normal and Search mode; suppress in Insert (bar cursor does the job).
    let show_sels = mode != Mode::Insert;

    for grapheme in content_cow.graphemes(true) {
        let gw = UnicodeWidthStr::width(grapheme) as u16;
        // Combining marks have display width 0 — advance at least 1 col so
        // they don't stack on the gutter edge.
        let advance = gw.max(1);

        if col + advance > width {
            break; // clip at right edge
        }

        let style = resolve_style(char_pos, colors, highlights, &sels_on_line, is_cursor_line_row, show_sels);
        screen_buf.set_string(x + col, y, grapheme, style);
        col += advance;
        char_pos += grapheme.chars().count();
    }

    // After the loop, char_pos == line_end_excl (the '\n' position).
    // If any selection's head or range reaches this position (cursor on the
    // newline / empty line), draw a space with the appropriate style so the
    // cursor is visible past the last glyph.
    let eol_is_head     = show_sels && sels_on_line.iter().any(|s| char_pos == s.head);
    let eol_is_selected = !eol_is_head && show_sels && sels_on_line.iter().any(|s| {
        char_pos >= s.start() && char_pos <= s.end()
    });

    if (eol_is_head || eol_is_selected) && col < width {
        let style = if eol_is_head { colors.cursor_head } else { colors.selection };
        screen_buf.set_string(x + col, y, " ", style);
    }
}

// ── Test helper ───────────────────────────────────────────────────────────────

/// Render to a plain string for snapshot testing.
///
/// Creates a temporary ratatui buffer of `width × height`, calls [`render`],
/// and serialises it row by row. Each row is right-trimmed so snapshots stay
/// compact. The status bar row is included as the last line.
///
/// `height` must be `view.height + 1` (content rows + status bar).
#[cfg(test)]
pub(crate) fn render_to_string(editor: &Editor, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut screen_buf = ScreenBuf::empty(area);
    render(editor, area, &mut screen_buf);

    (0..height)
        .map(|y| {
            let row: String = (0..width)
                .map(|x| screen_buf[(x, y)].symbol().to_string())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::Buffer;
    use crate::core::document::Document;
    use crate::editor::{Editor, Mode};
    use crate::core::selection::{Selection, SelectionSet};
    use crate::ui::statusline::{StatusLineConfig, StatusSegment};
    use crate::ui::view::{compute_gutter_width, LineNumberStyle, ViewState};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a Document with the cursor at a specific char offset.
    fn doc_at(content: &str, cursor: usize) -> Document {
        let buf = Buffer::from(content);
        let sels = SelectionSet::single(Selection::cursor(cursor));
        Document::new(buf, sels)
    }

    /// Build a ViewState for snapshot tests.
    fn view(doc: &Document, width: usize, height: usize, style: LineNumberStyle) -> ViewState {
        let buf = doc.buf();
        ViewState {
            scroll_offset: 0,
            height,
            width,
            gutter_width: compute_gutter_width(buf.len_lines()),
            line_number_style: style,
        }
    }

    /// Build a default Editor for rendering tests.
    fn editor_for(doc: Document, view: ViewState) -> Editor {
        Editor::for_testing(doc, view)
    }

    // ── Snapshot tests ────────────────────────────────────────────────────────

    #[test]
    fn render_simple_file() {
        let doc = doc_at("hello\nworld\n", 0);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        insta::assert_snapshot!(out, @r"
          1 hello
          2 world
        ~
         NOR │ [scratch]1:1");
    }

    #[test]
    fn render_empty_buffer() {
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        // Empty buffer has one visible line (the structural \n) with no content.
        insta::assert_snapshot!(out, @r"
          1
        ~
        ~
         NOR │ [scratch]1:1");
    }

    #[test]
    fn render_cursor_on_second_line() {
        // Cursor on 'w' at the start of "world\n" — char offset 6.
        let doc = doc_at("hello\nworld\n", 6);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        insta::assert_snapshot!(out, @r"
          1 hello
          2 world
        ~
         NOR │ [scratch]2:1");
    }

    #[test]
    fn render_status_bar_with_file_path() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_file_path(std::path::PathBuf::from("/home/user/notes.txt"));
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
         NOR │ notes.txt1:1");
    }

    #[test]
    fn render_line_numbers_absolute() {
        let doc = doc_at("a\nb\nc\n", 0);
        let v = view(&doc, 20, 4, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          2 b
          3 c
        ~
         NOR │ [scratch]1:1");
    }

    #[test]
    fn render_line_numbers_relative() {
        // Cursor on line 1 (0-based). Line 0 is 1 away, line 2 is 1 away.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Relative);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          0 b
          1 c
        ~
         NOR │ [scratch]2:1");
    }

    #[test]
    fn render_line_numbers_hybrid() {
        // Cursor on line 1 (0-based). Cursor line shows absolute; others relative.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Hybrid);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          2 b
          1 c
        ~
         NOR │ [scratch]2:1");
    }

    #[test]
    fn render_tilde_rows_for_short_file() {
        // 1-line file with a 5-row viewport: 1 content row + 4 tildes.
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 5, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 6);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
        ~
        ~
        ~
         NOR │ [scratch]1:1");
    }

    #[test]
    fn render_col_advances_past_multibyte() {
        // Status bar col should count grapheme clusters, not bytes.
        // "café" is 4 graphemes but 5 bytes (é = U+00E9 = 2 bytes in UTF-8).
        // Cursor at end of "café" = char offset 4.
        let doc = doc_at("café\n", 4);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 3);
        // Position should show 1:5 (4 graphemes before cursor, so col 5).
        insta::assert_snapshot!(out, @r"
          1 café
        ~
         NOR │ [scratch]1:5");
    }

    #[test]
    fn render_multi_cursor() {
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        // Two cursors: one on 'a' (char 0), one on 'b' (char 2).
        let buf = Buffer::from("a\nb\nc\n");
        let sels = SelectionSet::from_vec(
            vec![
                Selection::cursor(0), // line 0, 'a'
                Selection::cursor(2), // line 1, 'b'
            ],
            0, // primary = first
        );
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState {
            scroll_offset: 0,
            height: 4,
            width: 15,
            gutter_width: gw,
            line_number_style: LineNumberStyle::Absolute,
        };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 5);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        // Both cursor cells must have the cursor_head background (white).
        // 'a' is at column gw (after the gutter), row 0.
        // 'b' is at column gw, row 1.
        let cursor_head_bg = Color::Rgb(255, 255, 255);
        assert_eq!(screen[(gw as u16, 0)].bg, cursor_head_bg, "'a' cell should have cursor_head bg");
        assert_eq!(screen[(gw as u16, 1)].bg, cursor_head_bg, "'b' cell should have cursor_head bg");

        // Non-cursor 'c' at row 2 must NOT have the cursor_head background.
        assert_ne!(screen[(gw as u16, 2)].bg, cursor_head_bg, "'c' cell should not have cursor_head bg");
    }

    #[test]
    fn render_selection_range_highlighted() {
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        // "hello\n": selection anchor=1 ('e'), head=3 (second 'l').
        // Range [1,3]: 'e' (1), first 'l' (2) → selection body; second 'l' (3) → cursor head.
        let buf = Buffer::from("hello\n");
        let sels = SelectionSet::single(Selection::new(1, 3));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState {
            scroll_offset: 0,
            height: 2,
            width: 20,
            gutter_width: gw,
            line_number_style: LineNumberStyle::Absolute,
        };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 20, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let cursor_head_bg = Color::Rgb(255, 255, 255);
        let selection_bg   = Color::Rgb(68, 68, 120);

        // 'h' (0) — outside selection, no selection background.
        assert_ne!(screen[(gw as u16, 0)].bg, selection_bg,   "'h' should not have selection bg");
        assert_ne!(screen[(gw as u16, 0)].bg, cursor_head_bg, "'h' should not have cursor_head bg");

        // 'e' (1) and first 'l' (2) — selection body.
        assert_eq!(screen[(gw as u16 + 1, 0)].bg, selection_bg, "'e' should have selection bg");
        assert_eq!(screen[(gw as u16 + 2, 0)].bg, selection_bg, "first 'l' should have selection bg");

        // second 'l' (3) — cursor head.
        assert_eq!(screen[(gw as u16 + 3, 0)].bg, cursor_head_bg, "second 'l' (head) should have cursor_head bg");

        // 'o' (4) — outside selection.
        assert_ne!(screen[(gw as u16 + 4, 0)].bg, selection_bg,   "'o' should not have selection bg");
        assert_ne!(screen[(gw as u16 + 4, 0)].bg, cursor_head_bg, "'o' should not have cursor_head bg");
    }

    #[test]
    fn render_cursor_head_overrides_selection() {
        // Within a selection, the head cell gets cursor_head style, not selection style.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("abc\n");
        // anchor=0 ('a'), head=2 ('c'). Chars 0,1 are selection body; char 2 is cursor head.
        let sels = SelectionSet::single(Selection::new(0, 2));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let cursor_head_bg = Color::Rgb(255, 255, 255);
        let selection_bg   = Color::Rgb(68, 68, 120);
        assert_eq!(screen[(gw as u16,     0)].bg, selection_bg,   "'a' should be selection");
        assert_eq!(screen[(gw as u16 + 1, 0)].bg, selection_bg,   "'b' should be selection");
        assert_eq!(screen[(gw as u16 + 2, 0)].bg, cursor_head_bg, "'c' (head) should be cursor_head");
    }

    #[test]
    fn render_cursor_line_bg_on_unselected_cells() {
        // Cells on the cursor line that are outside any selection get cursor_line bg.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("abc\ndef\n");
        // Cursor on 'a' (char 0). Line 0 is the cursor line.
        let sels = SelectionSet::single(Selection::cursor(0));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 3, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 4);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let cursor_line_bg = Color::Rgb(35, 35, 45);
        let cursor_head_bg = Color::Rgb(255, 255, 255);

        // 'a' (head) at row 0, col gw — cursor_head, not cursor_line.
        assert_eq!(screen[(gw as u16, 0)].bg, cursor_head_bg, "'a' (head) should be cursor_head");
        // 'b' at row 0, col gw+1 — on cursor line but outside selection → cursor_line bg.
        assert_eq!(screen[(gw as u16 + 1, 0)].bg, cursor_line_bg, "'b' should have cursor_line bg");
        // 'd' at row 1, col gw — not on cursor line → no cursor_line bg.
        assert_ne!(screen[(gw as u16, 1)].bg, cursor_line_bg, "'d' should NOT have cursor_line bg");
    }

    #[test]
    fn render_selection_overrides_cursor_line() {
        // On the cursor line, selected non-head cells get selection bg, not cursor_line bg.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("abcd\n");
        // Selection from anchor=0 ('a') to head=2 ('c'). Cursor line = line 0.
        let sels = SelectionSet::single(Selection::new(0, 2));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let cursor_line_bg = Color::Rgb(35, 35, 45);
        let selection_bg   = Color::Rgb(68, 68, 120);
        let cursor_head_bg = Color::Rgb(255, 255, 255);

        // 'a' and 'b' are selection body on the cursor line → selection wins.
        assert_eq!(screen[(gw as u16, 0)].bg, selection_bg, "'a' selection overrides cursor_line");
        assert_eq!(screen[(gw as u16 + 1, 0)].bg, selection_bg, "'b' selection overrides cursor_line");
        // 'c' is head → cursor_head wins over both.
        assert_eq!(screen[(gw as u16 + 2, 0)].bg, cursor_head_bg, "'c' cursor_head wins");
        // 'd' is on cursor line but outside selection → cursor_line bg.
        assert_eq!(screen[(gw as u16 + 3, 0)].bg, cursor_line_bg, "'d' gets cursor_line bg");
    }

    // ── Bracket match highlight tests ─────────────────────────────────────────

    #[test]
    fn bracket_match_highlights_partner() {
        // Cursor on '(' at pos 0. render() computes the match automatically —
        // the ')' at pos 6 should receive bracket_match bg; '(' gets cursor_head.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("(hello)\n");
        let sels = SelectionSet::single(Selection::cursor(0));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let bracket_bg     = Color::Rgb(60, 55, 20);
        let cursor_head_bg = Color::Rgb(255, 255, 255);

        assert_eq!(screen[(gw as u16, 0)].bg, cursor_head_bg,     "'(' is cursor_head");
        assert_eq!(screen[(gw as u16 + 6, 0)].bg, bracket_bg,     "')' gets bracket_match bg");
    }

    #[test]
    fn bracket_match_does_not_override_selection() {
        // Cursor on '(' at pos 0 triggers bracket match for ')' at pos 6.
        // ')' is also within the selection — selection style must win over bracket_match.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("(hello)\n");
        // Backward selection: anchor=6 (')'), head=0 ('('). Covers '(hello)' [0..6].
        // Cursor (head) is on '(' → bracket match fires → ')' at 6 gets bracket_match.
        // But ')' is also in the selection body (not the head), so selection wins.
        let sels = SelectionSet::single(Selection::new(6, 0));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 15, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let selection_bg = Color::Rgb(68, 68, 120);
        let bracket_bg   = Color::Rgb(60, 55, 20);

        // ')' at char 6 is selected, so selection wins over bracket_match.
        assert_eq!(screen[(gw as u16 + 6, 0)].bg, selection_bg, "selection beats bracket_match");
        assert_ne!(screen[(gw as u16 + 6, 0)].bg, bracket_bg,   "bracket_match must not show");
    }

    #[test]
    fn bracket_match_overrides_cursor_line() {
        // Cursor on '(' at pos 0; ')' at pos 1 gets bracket_match on the cursor line.
        // bracket_match must beat cursor_line.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("()\n");
        let sels = SelectionSet::single(Selection::cursor(0));
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 10, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v);
        let area = Rect::new(0, 0, 10, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let bracket_bg     = Color::Rgb(60, 55, 20);
        let cursor_line_bg = Color::Rgb(35, 35, 45);

        assert_eq!(screen[(gw as u16 + 1, 0)].bg, bracket_bg,     "bracket_match beats cursor_line");
        assert_ne!(screen[(gw as u16 + 1, 0)].bg, cursor_line_bg, "cursor_line must not win");
    }

    #[test]
    fn render_insert_mode_label() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_mode(Mode::Insert);
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
         INS │ [scratch]1:1");
    }

    #[test]
    fn render_extend_mode_label() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_extend(true);
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
         EXT │ [scratch]1:1");
    }

    #[test]
    fn command_mode_cursor_position() {
        // Command mode uses a visual block cursor on the status-bar cell.
        // The terminal cursor is hidden (None); the cell at the cursor position
        // must have REVERSED cleared so it appears as normal video (dark bg).
        use ratatui::layout::Rect;
        use ratatui::style::Modifier;
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        // ":set" with cursor at end → cursor cell at col 5, row 2.
        let editor = editor_for(doc, v)
            .with_mode(Mode::Command)
            .with_minibuf(':', "set");
        let area = Rect::new(0, 0, 20, 3);
        let mut screen = ScreenBuf::empty(area);
        let cursor = render(&editor, area, &mut screen);
        assert_eq!(cursor.pos, None, "command mode uses visual cursor; terminal cursor is hidden");
        let cell = screen.cell((5, 2)).unwrap();
        assert!(!cell.style().add_modifier.contains(Modifier::REVERSED), "cursor cell must not be REVERSED");
    }

    #[test]
    fn command_mode_cursor_empty_input() {
        // With empty input the cursor cell is at col 2 (right after the prompt).
        use ratatui::layout::Rect;
        use ratatui::style::Modifier;
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_mode(Mode::Command)
            .with_minibuf(':', "");
        let area = Rect::new(0, 0, 20, 3);
        let mut screen = ScreenBuf::empty(area);
        let cursor = render(&editor, area, &mut screen);
        assert_eq!(cursor.pos, None, "command mode uses visual cursor; terminal cursor is hidden");
        let cell = screen.cell((2, 2)).unwrap();
        assert!(!cell.style().add_modifier.contains(Modifier::REVERSED), "cursor cell must not be REVERSED");
    }

    #[test]
    fn insert_mode_no_bracket_highlight() {
        // In Insert mode bracket matching is suppressed — the partner bracket
        // must NOT receive the bracket_match background.
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        let buf = Buffer::from("(hello)\n");
        let sels = SelectionSet::single(Selection::cursor(0)); // cursor on '('
        let doc = Document::new(buf, sels);
        let gw = compute_gutter_width(doc.buf().len_lines());
        let v = ViewState { scroll_offset: 0, height: 2, width: 15, gutter_width: gw, line_number_style: LineNumberStyle::Absolute };
        let editor = editor_for(doc, v).with_mode(Mode::Insert);
        let area = Rect::new(0, 0, 15, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&editor, area, &mut screen);

        let bracket_bg = Color::Rgb(60, 55, 20);
        // ')' at col gw+6 must NOT have bracket_match bg in Insert mode.
        assert_ne!(screen[(gw as u16 + 6, 0)].bg, bracket_bg, "bracket match must be suppressed in Insert mode");
    }

    // ── Smart-join tests ──────────────────────────────────────────────────────
    //
    // Three boundary cases for the segment spacing rule:
    //   (a) neither boundary is a space → a gap span is inserted
    //   (b) exactly one boundary is a space → segments join directly
    //   (c) both boundaries are spaces → leading space of the incoming
    //       segment is trimmed so there is exactly one space between them

    #[test]
    fn statusline_join_neither_space_inserts_gap() {
        // Separator ends with '│' (non-space), FileName starts with '[' (non-space).
        // Rule (a): a space must be inserted between them.
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 1, LineNumberStyle::Absolute);
        let config = StatusLineConfig {
            left: vec![StatusSegment::Separator, StatusSegment::FileName],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // The gap span produces exactly one space between │ and [scratch].
        insta::assert_snapshot!(out, @r"
          1
        │ [scratch]");
    }

    #[test]
    fn statusline_join_one_space_boundary_joins_directly() {
        // ModePill ends with ' ' (space), FileName starts with '[' (non-space).
        // Rule (b): exactly one boundary has a space, so segments join directly —
        // no extra space is inserted and no space is trimmed.
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 1, LineNumberStyle::Absolute);
        let config = StatusLineConfig {
            left: vec![StatusSegment::ModePill, StatusSegment::FileName],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // ModePill's trailing space serves as the single separator — no double space.
        insta::assert_snapshot!(out, @r"
          1
         NOR [scratch]");
    }

    #[test]
    fn statusline_join_both_space_trims_duplicate() {
        // Two ModePills: first ends ' ', second starts ' '.
        // Rule (c): leading space of the second pill is trimmed so there is
        // exactly one space between them, not two.
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 1, LineNumberStyle::Absolute);
        let config = StatusLineConfig {
            left: vec![StatusSegment::ModePill, StatusSegment::ModePill],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // " NOR " + trim(" NOR ") = " NOR NOR " — one space between, not two.
        insta::assert_snapshot!(out, @r"
          1
         NOR NOR");
    }

    // ── Search match count ────────────────────────────────────────────────────

    #[test]
    fn search_match_count_in_status_bar() {
        // "hello world hello\n" — cursor on the first 'h' (char 0).
        // Searching "hello" yields 2 matches; cursor is on match 1.
        let doc = doc_at("hello world hello\n", 0);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @r"
          1 hello world hello
        ~
         NOR │ [scratch]    [1/2] 1:1");
    }

    #[test]
    fn search_match_count_zero_when_cursor_between_matches() {
        // Cursor on ' ' (char 5), which is between the two "hello" matches.
        // current index should be 0.
        let doc = doc_at("hello world hello\n", 5);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @r"
          1 hello world hello
        ~
         NOR │ [scratch]    [0/2] 1:6");
    }

    #[test]
    fn search_match_count_in_search_mode_minibuf() {
        // In Search mode the mini-buffer replaces the status bar.
        // The match count should appear right-aligned on the command line row.
        let doc = doc_at("hello world hello\n", 0);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_mode(Mode::Search)
            .with_minibuf('/', "hello")
            .with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @r"
          1 hello world hello
        ~
         /hello                 [1/2]");
    }

    #[test]
    fn search_match_count_absent_without_search_regex() {
        // No active search — count must not appear.
        let doc = doc_at("hello\n", 0);
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let out = render_to_string(&editor, 25, 3);
        insta::assert_snapshot!(out, @r"
          1 hello
        ~
         NOR │ [scratch]     1:1");
    }

    // ── Dirty indicator ───────────────────────────────────────────────────────

    #[test]
    fn render_dirty_indicator_shown_when_dirty() {
        let mut doc = doc_at("hello\n", 0);
        // Apply an edit so the document is dirty.
        doc.apply_edit(|b, s| crate::ops::edit::insert_char(b, s, 'x'));
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_file_path(std::path::PathBuf::from("/tmp/notes.txt"));
        let out = render_to_string(&editor, 25, 3);
        insta::assert_snapshot!(out, @r"
          1 xhello
        ~
         NOR │ notes.txt [+] 1:2");
    }

    #[test]
    fn render_dirty_indicator_absent_when_clean() {
        let doc = doc_at("hello\n", 0);
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_file_path(std::path::PathBuf::from("/tmp/notes.txt"));
        let out = render_to_string(&editor, 25, 3);
        insta::assert_snapshot!(out, @r"
          1 hello
        ~
         NOR │ notes.txt     1:1");
    }
}
