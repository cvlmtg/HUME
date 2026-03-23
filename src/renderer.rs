use std::path::Path;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::buffer::Buffer;
use crate::display_line::DisplayLine;
use crate::document::Document;
use crate::editor::Mode;
use crate::selection::SelectionSet;
use crate::view::{LineNumberStyle, ViewState};

// ── Public entry point ────────────────────────────────────────────────────────

/// Render the current editor state into a ratatui screen buffer.
///
/// `area` is the full terminal area (including the status bar row).
/// The renderer splits it into:
///   - rows `0 .. view.height` — document content + gutter
///   - row `view.height`       — status bar (1 row, always the last)
///
/// This function is pure: it only writes to `screen_buf` and reads from its
/// arguments. All terminal I/O is handled by the caller (the editor event
/// loop).
pub(crate) fn render(
    doc: &Document,
    view: &ViewState,
    mode: Mode,
    file_path: Option<&Path>,
    area: Rect,
    screen_buf: &mut ScreenBuf,
) {
    let buf = doc.buf();
    let sels = doc.sels();
    let primary_head = sels.primary().head;
    let cursor_line = buf.char_to_line(primary_head);

    let display_lines = view.display_lines(buf);

    // ── Content rows ──────────────────────────────────────────────────────────

    for row in 0..view.height {
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }

        if let Some(dl) = display_lines.get(row) {
            render_gutter(screen_buf, view, dl, cursor_line, area.x, y);
            render_content(
                screen_buf,
                dl,
                area.x + view.gutter_width as u16,
                y,
                area.width.saturating_sub(view.gutter_width as u16),
                sels,
                buf,
            );
        } else {
            // Past end of buffer — draw `~` in the gutter column.
            screen_buf.set_string(area.x, y, "~", Style::new().fg(Color::DarkGray));
        }
    }

    // ── Status bar ────────────────────────────────────────────────────────────

    let status_y = area.y + view.height as u16;
    if status_y < area.bottom() {
        render_status_bar(screen_buf, mode, file_path, cursor_line, primary_head, buf, area, status_y);
    }
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
    view: &ViewState,
    dl: &DisplayLine<'_>,
    cursor_line: usize,
    x: u16,
    y: u16,
) {
    // Virtual lines have no line number — nothing to render in the gutter.
    let Some(line_number) = dl.line_number else { return };
    let line_idx = line_number.saturating_sub(1); // 0-based

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

    let is_cursor_line = line_idx == cursor_line;
    let style = if is_cursor_line {
        Style::new() // default — slightly brighter than dimmed neighbours
    } else {
        Style::new().fg(Color::DarkGray)
    };

    screen_buf.set_string(x, y, &gutter_str, style);
}

// ── Content ───────────────────────────────────────────────────────────────────

/// Render the text content of one display line into the screen buffer.
///
/// Iterates grapheme clusters (via `unicode-segmentation`) so that multi-byte
/// characters and combining sequences are treated as single units. Display
/// widths come from `unicode-width` so CJK double-width characters consume
/// exactly 2 columns.
///
/// Every character that falls within any selection range `[start, end]` is
/// rendered as `Modifier::REVERSED`, covering the full selected region. If a
/// cursor (head) sits past the last grapheme (end-of-line / empty line), a
/// reversed space is drawn there.
fn render_content(
    screen_buf: &mut ScreenBuf,
    dl: &DisplayLine<'_>,
    x: u16,
    y: u16,
    width: u16,
    sels: &SelectionSet,
    _buf: &Buffer,
) {
    let char_offset = dl.char_offset.unwrap_or(0);

    // line_end_incl = position of the stripped '\n' (one past last content char).
    let content_chars = dl.content.len_chars();
    let line_end_incl = char_offset + content_chars;

    // Collect selections whose range overlaps this line: [char_offset, line_end_incl].
    // A selection [s.start(), s.end()] overlaps if s.end() >= char_offset && s.start() <= line_end_incl.
    use crate::selection::Selection;
    let sels_on_line: Vec<Selection> = sels
        .iter_sorted()
        .copied()
        .filter(|s| s.end() >= char_offset && s.start() <= line_end_incl)
        .collect();

    if sels_on_line.is_empty() && dl.char_offset.is_none() {
        return; // virtual line with no selection overlap — nothing to render
    }

    let content_str = dl.content.to_string();
    let mut col: u16 = 0;
    let mut char_pos = char_offset;

    for grapheme in content_str.graphemes(true) {
        let gw = UnicodeWidthStr::width(grapheme) as u16;
        // Combining marks have display width 0 — advance at least 1 col so
        // they don't stack on the gutter edge.
        let advance = gw.max(1);

        if col + advance > width {
            break; // clip at right edge
        }

        // Highlight if this char falls within any selection's inclusive range.
        let selected = sels_on_line.iter().any(|s| char_pos >= s.start() && char_pos <= s.end());
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };

        screen_buf.set_string(x + col, y, grapheme, style);
        col += advance;
        char_pos += grapheme.chars().count();
    }

    // After the loop, char_pos == line_end_incl (the '\n' position).
    // If any selection reaches this position (cursor on the newline / empty line),
    // draw a reversed space so the cursor is visible.
    let selected_at_eol = sels_on_line.iter().any(|s| char_pos >= s.start() && char_pos <= s.end());
    if selected_at_eol && col < width {
        screen_buf.set_string(x + col, y, " ", Style::new().add_modifier(Modifier::REVERSED));
    }
}

// ── Status bar ────────────────────────────────────────────────────────────────

/// Render the one-row status bar at the bottom of the area.
///
/// Layout (all with inverted style):
/// - Left  : one space + mode label (`NOR`/`INS`) + one space + filename
/// - Right : `line:col` (both 1-based) + one space
///
/// `INS` is rendered in cyan to make the mode transition visually obvious.
fn render_status_bar(
    screen_buf: &mut ScreenBuf,
    mode: Mode,
    file_path: Option<&Path>,
    cursor_line: usize,
    cursor_head: usize,
    buf: &Buffer,
    area: Rect,
    y: u16,
) {
    let style = Style::new().add_modifier(Modifier::REVERSED);

    // Fill the entire row with inverted spaces first.
    let blank: String = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, style);

    // Mode label: "NOR" (default) or "INS" (cyan) — 3 chars, at column 1.
    let (mode_label, mode_style) = match mode {
        Mode::Normal => ("NOR", style),
        Mode::Insert => ("INS", style.fg(Color::Cyan)),
    };
    screen_buf.set_string(area.x + 1, y, mode_label, mode_style);

    // Filename at column 5 (space + 3-char label + space).
    let filename = file_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("[scratch]");
    screen_buf.set_string(area.x + 5, y, filename, style);

    // Right: "line:col" (1-based column = grapheme count from line start + 1).
    let col_0 = grapheme_col_in_line(buf, cursor_line, cursor_head);
    let pos_str = format!("{}:{}", cursor_line + 1, col_0 + 1);
    // Place with 1 space of right margin.
    let pos_x = area.right().saturating_sub(pos_str.len() as u16 + 1);
    screen_buf.set_string(pos_x, y, &pos_str, style);
}

/// Count grapheme clusters from the start of `line_idx` to `char_pos`.
///
/// Returns the 0-based grapheme offset of the cursor within its line — the
/// same unit used by left/right cursor movement. This is intentionally a
/// logical position (grapheme index), not a display column: if the line
/// contains wide characters, the visual column may differ, but the reported
/// number matches how many times the user pressed → to get there.
fn grapheme_col_in_line(buf: &Buffer, line_idx: usize, char_pos: usize) -> usize {
    let line_start = buf.line_to_char(line_idx);
    // char_pos should be >= line_start, but saturating_sub guards against
    // any edge cases in empty buffers.
    let slice = buf.slice(line_start..char_pos.max(line_start));
    slice.to_string().graphemes(true).count()
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
pub(crate) fn render_to_string(
    doc: &Document,
    view: &ViewState,
    mode: Mode,
    file_path: Option<&Path>,
    width: u16,
    height: u16,
) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut screen_buf = ScreenBuf::empty(area);
    render(doc, view, mode, file_path, area, &mut screen_buf);

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
    use crate::buffer::Buffer;
    use crate::document::Document;
    use crate::editor::Mode;
    use crate::selection::{Selection, SelectionSet};
    use crate::view::{compute_gutter_width, LineNumberStyle, ViewState};

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

    // ── Snapshot tests ────────────────────────────────────────────────────────

    #[test]
    fn render_simple_file() {
        let doc = doc_at("hello\nworld\n", 0);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        // height = 3 content rows + 1 status = 4 total
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 4);
        insta::assert_snapshot!(out, @r"
          1 hello
          2 world
        ~
         NOR [scratch]  1:1");
    }

    #[test]
    fn render_empty_buffer() {
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 4);
        // Empty buffer has one visible line (the structural \n) with no content.
        insta::assert_snapshot!(out, @r"
          1
        ~
        ~
         NOR [scratch]  1:1");
    }

    #[test]
    fn render_cursor_on_second_line() {
        // Cursor on 'w' at the start of "world\n" — char offset 6.
        let doc = doc_at("hello\nworld\n", 6);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 4);
        insta::assert_snapshot!(out, @r"
          1 hello
          2 world
        ~
         NOR [scratch]  2:1");
    }

    #[test]
    fn render_status_bar_with_file_path() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let path = std::path::Path::new("/home/user/notes.txt");
        let out = render_to_string(&doc, &v, Mode::Normal, Some(path), 20, 3);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
         NOR notes.txt  1:1");
    }

    #[test]
    fn render_line_numbers_absolute() {
        let doc = doc_at("a\nb\nc\n", 0);
        let v = view(&doc, 20, 4, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          2 b
          3 c
        ~
         NOR [scratch]  1:1");
    }

    #[test]
    fn render_line_numbers_relative() {
        // Cursor on line 1 (0-based). Line 0 is 1 away, line 2 is 1 away.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Relative);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          0 b
          1 c
        ~
         NOR [scratch]  2:1");
    }

    #[test]
    fn render_line_numbers_hybrid() {
        // Cursor on line 1 (0-based). Cursor line shows absolute; others relative.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Hybrid);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 5);
        insta::assert_snapshot!(out, @r"
          1 a
          2 b
          1 c
        ~
         NOR [scratch]  2:1");
    }

    #[test]
    fn render_tilde_rows_for_short_file() {
        // 1-line file with a 5-row viewport: 1 content row + 4 tildes.
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 5, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 6);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
        ~
        ~
        ~
         NOR [scratch]  1:1");
    }

    #[test]
    fn render_col_advances_past_multibyte() {
        // Status bar col should count grapheme clusters, not bytes.
        // "café" is 4 graphemes but 5 bytes (é = U+00E9 = 2 bytes in UTF-8).
        // Cursor at end of "café" = char offset 4.
        let doc = doc_at("café\n", 4);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Normal, None, 20, 3);
        // Position should show 1:5 (4 graphemes before cursor, so col 5).
        insta::assert_snapshot!(out, @r"
          1 café
        ~
         NOR [scratch]  1:5");
    }

    #[test]
    fn render_multi_cursor() {
        use ratatui::layout::Rect;
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
        let area = Rect::new(0, 0, 15, 5);
        let mut screen = ScreenBuf::empty(area);
        render(&doc, &v, Mode::Normal, None, area, &mut screen);

        // Both cursor cells must have the REVERSED modifier.
        // 'a' is at column gw (after the gutter), row 0.
        // 'b' is at column gw, row 1.
        let cursor_a = screen[(gw as u16, 0)].modifier;
        let cursor_b = screen[(gw as u16, 1)].modifier;
        assert!(cursor_a.contains(Modifier::REVERSED), "'a' cell should be REVERSED");
        assert!(cursor_b.contains(Modifier::REVERSED), "'b' cell should be REVERSED");

        // Non-cursor 'c' at row 2 must NOT be reversed.
        let non_cursor = screen[(gw as u16, 2)].modifier;
        assert!(!non_cursor.contains(Modifier::REVERSED), "'c' cell should not be REVERSED");
    }

    #[test]
    fn render_selection_range_highlighted() {
        use ratatui::layout::Rect;
        // "hello\n": selection anchor=1 ('e'), head=3 ('l') → chars 1,2,3 highlighted.
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
        let area = Rect::new(0, 0, 20, 3);
        let mut screen = ScreenBuf::empty(area);
        render(&doc, &v, Mode::Normal, None, area, &mut screen);

        // 'h' at col gw+0 — outside selection, not reversed.
        let h_cell = screen[(gw as u16, 0)].modifier;
        assert!(!h_cell.contains(Modifier::REVERSED), "'h' should not be highlighted");

        // 'e','l','l' at cols gw+1, gw+2, gw+3 — inside [1,3], all reversed.
        for (label, col) in [("'e'", gw + 1), ("'l'", gw + 2), ("'l'", gw + 3)] {
            let cell = screen[(col as u16, 0)].modifier;
            assert!(cell.contains(Modifier::REVERSED), "{label} should be highlighted");
        }

        // 'o' at col gw+4 — outside selection, not reversed.
        let o_cell = screen[(gw as u16 + 4, 0)].modifier;
        assert!(!o_cell.contains(Modifier::REVERSED), "'o' should not be highlighted");
    }

    #[test]
    fn render_insert_mode_label() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let out = render_to_string(&doc, &v, Mode::Insert, None, 20, 3);
        insta::assert_snapshot!(out, @r"
          1 hi
        ~
         INS [scratch]  1:1");
    }
}
