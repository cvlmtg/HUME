use std::borrow::Cow;

use crossterm::cursor::SetCursorStyle;
use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;

use crate::core::grapheme::grapheme_advance;
use crate::core::selection::Selection;
use crate::editor::{Editor, Mode};
use crate::ops::text_object::find_bracket_pair;
use crate::ui::formatter::{DocumentFormatter, VisualRow, cursor_visual_pos};
use crate::ui::highlight::HighlightSet;
use crate::ui::theme::EditorColors;
use crate::ui::whitespace::WhitespaceShow;
use crate::ui::statusline::render_bottom_row;

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
        Mode::Insert | Mode::Command | Mode::Search | Mode::Select => SetCursorStyle::SteadyBar,
    }
}

/// Render the current editor state into a ratatui screen buffer.
///
/// `area` is the full terminal area (including the statusline row).
/// The renderer splits it via [`layout`] into gutter, content, and statusline.
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
    let buf = editor.doc.buf();
    let total_lines = buf.len_lines().saturating_sub(1);
    let lay = layout(area, editor.view.gutter_width() as u16);
    let highlights = compute_highlights(editor);
    let cursor_line = buf.char_to_line(editor.doc.sels().primary().head);

    // ── Content rows via DocumentFormatter ───────────────────────────────────
    //
    // The formatter is the single source of truth for row boundaries: it tells
    // us exactly which chars appear on which visual row, accounting for soft-
    // wrap, tabs, and (future) virtual lines. The renderer consumes these row
    // descriptors and draws gutter + content for each.

    let mut last_rendered_row: Option<usize> = None;
    // Both scratch buffers are created once here and reused across all rows,
    // eliminating per-row heap allocations.
    let mut sels_scratch: Vec<Selection> = Vec::new();
    let mut gutter_scratch = String::new();

    for vrow in DocumentFormatter::new(buf, &editor.view) {
        let y = area.y + vrow.row as u16;
        if y >= lay.content.y + lay.content.height {
            break;
        }

        // Gutter: delegate to composable column providers.
        editor.view.gutter.render_row(
            screen_buf,
            &vrow,
            editor.view.line_number_style,
            cursor_line,
            &editor.colors,
            lay.gutter.x,
            y,
            total_lines,
            &mut gutter_scratch,
        );

        // Content: grapheme walk with per-character style resolution.
        // For indent-wrapped continuation rows, `visual_indent` columns of
        // padding are prepended — fill them with the cursor-line bg (or default
        // bg) so they don't appear as unstyled terminal cells.
        let indent = vrow.visual_indent as u16;
        let content_x = lay.content.x + indent;
        let content_w = lay.content.width.saturating_sub(indent);
        if indent > 0 {
            let indent_style = if buf.char_to_line(vrow.char_start) == cursor_line {
                editor.colors.cursor_line
            } else {
                editor.colors.default
            };
            screen_buf.set_style(Rect::new(lay.content.x, y, indent, 1), indent_style);
        }
        render_row_content(screen_buf, editor, &highlights, &vrow, cursor_line, content_x, y, content_w, &mut sels_scratch);

        last_rendered_row = Some(vrow.row);
    }

    // ── Tilde rows past end of buffer ─────────────────────────────────────────
    let first_empty = last_rendered_row.map_or(0, |r| r + 1);
    for row in first_empty..editor.view.height {
        let y = area.y + row as u16;
        if y >= lay.content.y + lay.content.height {
            break;
        }
        screen_buf.set_string(area.x, y, "~", editor.colors.tilde);
    }

    // ── Bottom row (statusline / command line / status message) ───────────────

    if lay.statusline.y < area.bottom() {
        render_bottom_row(screen_buf, editor, area, lay.statusline.y);
    }

    CursorState { pos: compute_cursor_pos(editor) }
}

// ── Layout ────────────────────────────────────────────────────────────────────

struct Layout {
    gutter: Rect,
    content: Rect,
    statusline: Rect,
}

/// Divide the terminal area into gutter, content, and statusline regions.
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
    let statusline = Rect::new(area.x, area.y + content_height, area.width, 1);
    Layout { gutter, content, statusline }
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
    for &(start, end_incl) in editor.search.matches() {
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
/// In Normal mode returns `None` — the visual `cursor_head` cell acts as the
/// cursor and the real terminal cursor is hidden. In Insert mode, delegates to
/// [`cursor_visual_pos`] which scans the [`DocumentFormatter`] output.
///
/// Both rendering and cursor positioning consume the same formatter row
/// boundaries, so they can never disagree about where a character is on screen.
fn compute_cursor_pos(editor: &Editor) -> Option<(u16, u16)> {
    match editor.mode {
        Mode::Insert => {
            let buf = editor.doc.buf();
            let head = editor.doc.sels().primary().head;
            cursor_visual_pos(buf, &editor.view, head).map(|(col, row)| {
                ((editor.view.gutter_width() + col) as u16, row as u16)
            })
        }
        // Normal uses a visual block cell; Command/Search use a MiniBuf cursor.
        Mode::Normal | Mode::Command | Mode::Search | Mode::Select => None,
    }
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

/// Compose the whitespace indicator style with the resolved base style.
///
/// Uses the whitespace fg colour but preserves the base style's bg. This means
/// whitespace indicators inside a selection show the selection bg but use the
/// dim whitespace fg — the indicator is visible but the selection extent remains
/// clear.
///
/// Exception: when the base style is `cursor_head`, no patching is done — the
/// cursor cell must remain fully visible and unambiguous.
fn compose_whitespace_style(base: Style, colors: &EditorColors) -> Style {
    if base == colors.cursor_head {
        return base;
    }
    // Patch fg from the whitespace style; keep everything else from the base.
    Style {
        fg: colors.whitespace.fg,
        ..base
    }
}

/// Whether a whitespace grapheme at `char_pos` should be shown as an indicator,
/// given the per-type [`WhitespaceShow`] setting and the trailing boundary.
fn should_show_ws(show: WhitespaceShow, char_pos: usize, trailing_start: usize) -> bool {
    match show {
        WhitespaceShow::None => false,
        WhitespaceShow::All => true,
        WhitespaceShow::Trailing => char_pos >= trailing_start,
    }
}

/// Render the text content of one visual row into the screen buffer.
///
/// Iterates grapheme clusters (via `unicode-segmentation`) so that multi-byte
/// characters and combining sequences are treated as single units. Display
/// widths come from `unicode-width` so CJK double-width characters consume
/// exactly 2 columns. Style resolution is delegated to [`resolve_style`].
///
/// `abs_col` (absolute display column within the buffer line) is taken directly
/// from `vrow.col_offset_in_line` — the formatter already computed it when
/// determining wrap break points, so we get it for free.
fn render_row_content(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    highlights: &HighlightSet,
    vrow: &VisualRow,
    cursor_line: usize,
    x: u16,
    y: u16,
    width: u16,
    sels_scratch: &mut Vec<Selection>,
) {
    let buf = editor.doc.buf();
    let mode = editor.mode;
    let colors = &editor.colors;
    let sels = editor.doc.sels();
    let char_offset = vrow.char_start;

    // vrow.char_end is the exclusive end of content (= position of '\n' for the
    // last segment; = first char of next segment for intermediate wrap rows).
    let line_end_excl = vrow.char_end;

    // Collect selections that overlap this display row into the caller-provided
    // buffer. Reusing the allocation avoids a heap alloc per row (40-60×/frame).
    // Selection is Copy (two usizes), count per row is typically 1.
    sels_scratch.clear();
    sels_scratch.extend(
        sels.iter_sorted()
            .filter(|s| s.end() >= char_offset && s.start() <= line_end_excl)
            .copied(),
    );
    let sels_on_line = &*sels_scratch;

    // Whether this row is the primary cursor's row. Used for cursor-line bg tint.
    // Continuation rows share the buffer line of the first segment.
    let is_cursor_line_row = buf.char_to_line(char_offset) == cursor_line;

    // Pre-fill the content area with cursor-line bg so empty space at the end
    // of the row also gets the tint. Individual cells are overwritten below.
    if is_cursor_line_row {
        screen_buf.set_style(Rect::new(x, y, width, 1), colors.cursor_line);
    }

    // Borrow the row content as &str — zero-copy for contiguous rope slices
    // (the common case), falling back to an owned String for multi-chunk lines.
    let content_cow: Cow<str> = buf.slice(char_offset..vrow.char_end).into();
    let mut char_pos = char_offset;

    // Show selections in Normal and Search mode; suppress in Insert mode
    // (the bar cursor handles visual feedback there).
    let show_sels = mode != Mode::Insert;

    let col_offset = editor.view.col_offset;
    let tab_width = editor.view.tab_width.max(1);
    let ws_cfg = &editor.view.whitespace;
    let mut display_col: usize = 0;

    // `abs_col` = absolute display column from the buffer line's first char.
    // The formatter pre-computed this as `vrow.col_offset_in_line` — we get it
    // for free instead of calling `display_col_in_line()` again.
    let mut abs_col: usize = vrow.col_offset_in_line;

    // Pre-scan for trailing whitespace boundary: only needed when at least one
    // whitespace type uses WhitespaceShow::Trailing. Skipping it when unused
    // avoids a full grapheme walk before the main rendering loop.
    let trailing_start: usize =
        if ws_cfg.render.space == WhitespaceShow::Trailing
            || ws_cfg.render.tab == WhitespaceShow::Trailing
            || ws_cfg.render.newline == WhitespaceShow::Trailing
        {
            let mut last_non_ws = char_offset;
            let mut pos = char_offset;
            for g in content_cow.graphemes(true) {
                let g_end = pos + g.chars().count();
                if !g.chars().all(|c| c == ' ' || c == '\t') {
                    last_non_ws = g_end;
                }
                pos = g_end;
            }
            last_non_ws
        } else {
            usize::MAX // sentinel: char_pos >= usize::MAX is never true
        };

    for grapheme in content_cow.graphemes(true) {
        let is_tab = grapheme == "\t";
        let advance = grapheme_advance(grapheme, abs_col, tab_width);

        if display_col + advance <= col_offset {
            display_col += advance;
            abs_col += advance;
            char_pos += grapheme.chars().count();
            continue;
        }

        // A wide character (e.g. CJK) or tab might straddle the left edge:
        // its first column is before col_offset but later columns are visible.
        // Render placeholder spaces so the partial char appears as a gap.
        if display_col < col_offset {
            let style = resolve_style(char_pos, colors, highlights, sels_on_line, is_cursor_line_row, show_sels);
            let visible = (display_col + advance).saturating_sub(col_offset);
            for i in 0..visible {
                if i as u16 >= width { break; }
                screen_buf.set_string(x + i as u16, y, " ", style);
            }
            display_col += advance;
            abs_col += advance;
            char_pos += grapheme.chars().count();
            continue;
        }

        let screen_col = (display_col - col_offset) as u16;

        if screen_col + advance as u16 > width {
            break; // clip at right edge
        }

        let style = resolve_style(char_pos, colors, highlights, sels_on_line, is_cursor_line_row, show_sels);

        if is_tab {
            // Tab: expand to `advance` columns. First cell gets the indicator
            // character if enabled; remaining cells are spaces.
            let show_tab = should_show_ws(ws_cfg.render.tab, char_pos, trailing_start);
            let (first_ch, tab_style) = if show_tab {
                (ws_cfg.chars.tab, compose_whitespace_style(style, colors))
            } else {
                (' ', style)
            };
            let mut utf8_buf = [0u8; 4];
            let first_str = first_ch.encode_utf8(&mut utf8_buf);
            screen_buf.set_stringn(x + screen_col, y, first_str, 1, tab_style);
            for i in 1..advance {
                if screen_col + i as u16 >= width { break; }
                screen_buf.set_string(x + screen_col + i as u16, y, " ", tab_style);
            }
        } else if grapheme == " " && should_show_ws(ws_cfg.render.space, char_pos, trailing_start) {
            let ws_style = compose_whitespace_style(style, colors);
            let mut utf8_buf = [0u8; 4];
            let s = ws_cfg.chars.space.encode_utf8(&mut utf8_buf);
            screen_buf.set_stringn(x + screen_col, y, s, 1, ws_style);
        } else {
            screen_buf.set_string(x + screen_col, y, grapheme, style);
        }

        display_col += advance;
        abs_col += advance;
        char_pos += grapheme.chars().count();
    }

    // After the loop, char_pos points to either the '\n' (for the last segment
    // of a buffer line) or the first char of the next segment (for intermediate
    // wrap rows). EOL rendering only applies to the last segment of each line.
    let at_newline = vrow.is_last_segment
        && (buf.char_at(char_pos) == Some('\n') || char_pos == buf.len_chars());
    let eol_screen_col = display_col.saturating_sub(col_offset) as u16;

    if at_newline {
        if ws_cfg.render.newline != WhitespaceShow::None && eol_screen_col < width {
            let base_style = if is_cursor_line_row { colors.cursor_line } else { colors.default };
            let ws_style = compose_whitespace_style(base_style, colors);
            let mut utf8_buf = [0u8; 4];
            let s = ws_cfg.chars.newline.encode_utf8(&mut utf8_buf);
            screen_buf.set_stringn(x + eol_screen_col, y, s, 1, ws_style);
        }

        // If any selection's head or range reaches the newline position, draw a
        // space with the selection/cursor style so the cursor is visible past
        // the last glyph.
        let eol_is_head     = show_sels && sels_on_line.iter().any(|s| char_pos == s.head);
        let eol_is_selected = !eol_is_head && show_sels && sels_on_line.iter().any(|s| {
            char_pos >= s.start() && char_pos <= s.end()
        });

        if (eol_is_head || eol_is_selected) && eol_screen_col < width {
            let style = if eol_is_head { colors.cursor_head } else { colors.selection };
            screen_buf.set_string(x + eol_screen_col, y, " ", style);
        }
    }
}

// ── Test helper ───────────────────────────────────────────────────────────────

/// Render to a plain string for snapshot testing.
///
/// Creates a temporary ratatui buffer of `width × height`, calls [`render`],
/// and serialises it row by row. Each row is right-trimmed so snapshots stay
/// compact. The statusline row is included as the last line.
///
/// `height` must be `view.height + 1` (content rows + statusline).
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
    use crate::ui::statusline::{StatusLineConfig, StatusElement};
    use crate::ui::gutter::GutterConfig;
    use crate::ui::view::{LineNumberStyle, ViewState};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a Document with the cursor at a specific char offset.
    fn doc_at(content: &str, cursor: usize) -> Document {
        let buf = Buffer::from(content);
        let sels = SelectionSet::single(Selection::cursor(cursor));
        Document::new(buf, sels)
    }

    /// Build a ViewState for snapshot tests.
    fn view(doc: &Document, width: usize, height: usize, style: LineNumberStyle) -> ViewState {
        let cached_total_lines = doc.buf().len_lines().saturating_sub(1);
        ViewState {
            scroll_offset: 0,
            height,
            width,
            gutter: GutterConfig::default(),
            cached_total_lines,
            line_number_style: style,
            col_offset: 0,
            tab_width: 4,
            whitespace: crate::ui::whitespace::WhitespaceConfig::default(),
            soft_wrap: false,
            word_wrap: false,
            indent_wrap: false,
            scroll_sub_offset: 0,
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
        insta::assert_snapshot!(out, @"
          1 hello
          2 world
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_empty_buffer() {
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        // Empty buffer has one visible line (the structural \n) with no content.
        insta::assert_snapshot!(out, @"
          1
        ~
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_cursor_on_second_line() {
        // Cursor on 'w' at the start of "world\n" — char offset 6.
        let doc = doc_at("hello\nworld\n", 6);
        let v = view(&doc, 20, 3, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        insta::assert_snapshot!(out, @"
          1 hello
          2 world
        ~
         2:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_statusline_with_file_path() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_file_path(std::path::PathBuf::from("/home/user/notes.txt"));
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 hi
        ~
         1:1 notes.txt│ NOR
        ");
    }

    #[test]
    fn render_line_numbers_absolute() {
        let doc = doc_at("a\nb\nc\n", 0);
        let v = view(&doc, 20, 4, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @"
          1 a
          2 b
          3 c
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_line_numbers_relative() {
        // Cursor on line 1 (0-based). Line 0 is 1 away, line 2 is 1 away.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Relative);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @"
          1 a
          0 b
          1 c
        ~
         2:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_line_numbers_hybrid() {
        // Cursor on line 1 (0-based). Cursor line shows absolute; others relative.
        let doc = doc_at("a\nb\nc\n", 2); // char 2 = start of "b\n"
        let v = view(&doc, 20, 4, LineNumberStyle::Hybrid);
        let out = render_to_string(&editor_for(doc, v), 20, 5);
        insta::assert_snapshot!(out, @"
          1 a
          2 b
          1 c
        ~
         2:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_tilde_rows_for_short_file() {
        // 1-line file with a 5-row viewport: 1 content row + 4 tildes.
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 5, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 6);
        insta::assert_snapshot!(out, @"
          1 hi
        ~
        ~
        ~
        ~
         1:1 [scratch]│ NOR
        ");
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
        insta::assert_snapshot!(out, @"
          1 café
        ~
         1:5 [scratch]│ NOR
        ");
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
        let v = view(&doc, 15, 4, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 15, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 15, 3, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 15, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 15, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 15, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        let v = view(&doc, 10, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let gw = editor.view.gutter_width();
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
        insta::assert_snapshot!(out, @"
          1 hi
        ~
         1:1 [scratch]│ INS
        ");
    }

    #[test]
    fn render_extend_mode_label() {
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_extend(true);
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 hi
        ~
         1:1 [scratch]│ EXT
        ");
    }

    #[test]
    fn command_mode_cursor_position() {
        // Command mode uses a visual block cursor on the statusline cell.
        // The terminal cursor is hidden (None); the cell at the cursor position
        // must have REVERSED cleared so it appears as normal video (dark bg).
        use ratatui::layout::Rect;
        use ratatui::style::Modifier;
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        // " :set" with cursor at end → cursor cell at col 5, row 2.
        // Layout: " " pad (1) + ":" (1) + "set" (3) = 5
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
        // With empty input the cursor cell is at col 2 (after prompt).
        // Layout: " " pad (1) + ":" (1) = 2
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
        let v = view(&doc, 15, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_mode(Mode::Insert);
        let gw = editor.view.gutter_width();
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
            left: vec![StatusElement::Separator, StatusElement::FileName],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // The gap span produces exactly one space between │ and [scratch].
        insta::assert_snapshot!(out, @"
         1
        │ [scratch]
        ");
    }

    #[test]
    fn statusline_join_one_space_boundary_joins_directly() {
        // pad_left inserts a " " span, then Mode ("NOR") starts with a
        // non-space. The pad ends with ' ', Mode starts with 'N' → rule (b):
        // exactly one boundary has a space, so they join directly.
        // Then Mode ends with 'R', FileName starts with '[' → rule (a):
        // a gap span is inserted.
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 1, LineNumberStyle::Absolute);
        let config = StatusLineConfig {
            left: vec![StatusElement::Mode, StatusElement::FileName],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // " " pad + "NOR" + " " gap + "[scratch]"
        insta::assert_snapshot!(out, @r"
          1
         NOR [scratch]");
    }

    #[test]
    fn statusline_join_identical_elements_inserts_gap() {
        // Two Modes: "NOR" + "NOR". Neither has spaces, so rule (a) inserts
        // a gap between them. pad_left adds the leading space.
        let doc = doc_at("\n", 0);
        let v = view(&doc, 20, 1, LineNumberStyle::Absolute);
        let config = StatusLineConfig {
            left: vec![StatusElement::Mode, StatusElement::Mode],
            center: vec![],
            right: vec![],
        };
        let editor = editor_for(doc, v).with_statusline_config(config);
        let out = render_to_string(&editor, 20, 2);
        // " " pad + "NOR" + " " gap + "NOR"
        insta::assert_snapshot!(out, @r"
          1
         NOR NOR");
    }

    // ── Search match count ────────────────────────────────────────────────────

    #[test]
    fn search_match_count_in_statusline() {
        // "hello world hello\n" — cursor on the first 'h' (char 0).
        // Searching "hello" yields 2 matches; cursor is on match 1.
        let doc = doc_at("hello world hello\n", 0);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @"
          1 hello world hello
        ~
         1:1 [scratch]    [1/2] │ NOR
        ");
    }

    #[test]
    fn search_match_count_zero_when_cursor_between_matches() {
        // Cursor on ' ' (char 5), which is between the two "hello" matches.
        // current index should be 0.
        let doc = doc_at("hello world hello\n", 5);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v).with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @"
          1 hello world hello
        ~
         1:6 [scratch]    [0/2] │ NOR
        ");
    }

    #[test]
    fn search_match_count_in_search_mode_minibuf() {
        // In Search mode the statusline shows the MiniBuf element on the left
        // and the user's right section (including SearchMatches) on the right.
        let doc = doc_at("hello world hello\n", 0);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_mode(Mode::Search)
            .with_minibuf('/', "hello")
            .with_search_regex("hello");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @"
          1 hello world hello
        ~
         /hello           [1/2] │ SRC
        ");
    }

    #[test]
    fn search_match_count_absent_without_search_regex() {
        // No active search — count must not appear.
        let doc = doc_at("hello\n", 0);
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let out = render_to_string(&editor, 25, 3);
        insta::assert_snapshot!(out, @"
          1 hello
        ~
         1:1 [scratch]     │ NOR
        ");
    }

    #[test]
    fn search_match_count_shown_in_command_mode() {
        // After confirming a search, entering Command mode shows the match
        // count in the right section — it's part of the user's statusline
        // config and applies uniformly across all modes.
        let doc = doc_at("hello world hello\n", 0);
        let v = view(&doc, 30, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_search_regex("hello")
            .with_mode(Mode::Command)
            .with_minibuf(':', "w");
        let out = render_to_string(&editor, 30, 3);
        insta::assert_snapshot!(out, @"
          1 hello world hello
        ~
         :w               [1/2] │ CMD
        ");
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
        insta::assert_snapshot!(out, @"
          1 xhello
        ~
         1:2 notes.txt [+] │ NOR
        ");
    }

    #[test]
    fn render_dirty_indicator_absent_when_clean() {
        let doc = doc_at("hello\n", 0);
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v)
            .with_file_path(std::path::PathBuf::from("/tmp/notes.txt"));
        let out = render_to_string(&editor, 25, 3);
        insta::assert_snapshot!(out, @"
          1 hello
        ~
         1:1 notes.txt     │ NOR
        ");
    }

    // ── Whitespace rendering ─────────────────────────────────────────────────

    #[test]
    fn render_whitespace_none_is_default() {
        // Default config (all None) — spaces render as spaces, no indicators.
        let doc = doc_at("hello world\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let out = render_to_string(&editor_for(doc, v), 20, 3);
        insta::assert_snapshot!(out, @"
          1 hello world
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_all_spaces() {
        // WhitespaceShow::All on spaces — middle dot replaces every space.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("hi lo\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { space: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 hi·lo
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_trailing_only() {
        // Trailing mode — mid-content space stays normal, trailing spaces get indicators.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("hi lo   \n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { space: WhitespaceShow::Trailing, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 hi lo···
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_tab_expansion() {
        // Tab expands to 4 columns (default tab_width), no indicator.
        let doc = doc_at("\thi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let editor = editor_for(doc, v);
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1     hi
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_tab_indicator() {
        // Tab with indicator — arrow + fill spaces.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("\thi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { tab: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 →   hi
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_tab_mid_line() {
        // Tab after content aligns to next tab stop.
        // "ab" is 2 cols, tab expands to 2 more cols (next stop at col 4).
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("ab\tcd\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { tab: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 ab→ cd
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_newline_indicator() {
        // Newline indicator draws ⏎ at end of line.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("hi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { newline: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 hi⏎
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_trailing_all_spaces_line() {
        // A line that is entirely spaces — all are trailing in Trailing mode.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("   \n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { space: WhitespaceShow::Trailing, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 ···
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_tab_with_horizontal_scroll() {
        // Tab at col 0 expands to 4 columns. With col_offset=2, the first
        // two columns are scrolled off — only 2 placeholder spaces remain visible.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("\thi\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.col_offset = 2;
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { tab: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 20, 3);
        // The tab's first 2 cols (including the →) are scrolled off; 2 fill spaces remain.
        insta::assert_snapshot!(out, @"
          1   hi
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_custom_chars() {
        // Custom indicator characters instead of defaults.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("a b\n", 0);
        let v = view(&doc, 20, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender { space: WhitespaceShow::All, ..Default::default() },
            chars: WhitespaceChars { space: '•', ..Default::default() },
        };
        let out = render_to_string(&editor, 20, 3);
        insta::assert_snapshot!(out, @"
          1 a•b
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_whitespace_combined_indicators() {
        // All indicator types enabled simultaneously on a line with mixed whitespace.
        use crate::ui::whitespace::{WhitespaceConfig, WhitespaceRender, WhitespaceChars, WhitespaceShow};
        let doc = doc_at("a \tb \n", 0);
        let v = view(&doc, 25, 2, LineNumberStyle::Absolute);
        let mut editor = editor_for(doc, v);
        editor.view.whitespace = WhitespaceConfig {
            render: WhitespaceRender {
                space: WhitespaceShow::All,
                tab: WhitespaceShow::All,
                newline: WhitespaceShow::All,
            },
            chars: WhitespaceChars::default(),
        };
        let out = render_to_string(&editor, 25, 3);
        // "a" (1) + "·" (1) + "→ " (tab: 2 cols to next stop at 4) + "b" (1) + "·" (1) + "⏎"
        insta::assert_snapshot!(out, @"
          1 a·→ b·⏎
        ~
         1:1 [scratch]     │ NOR
        ");
    }

    // ── Soft-wrap snapshot tests ─────────────────────────────────────────────

    /// Build a ViewState with soft wrap enabled for snapshot tests.
    fn wrap_view(doc: &Document, width: usize, height: usize) -> ViewState {
        let buf = doc.buf();
        ViewState {
            scroll_offset: 0,
            height,
            width,
            gutter: GutterConfig::default(), cached_total_lines: buf.len_lines().saturating_sub(1),
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: crate::ui::whitespace::WhitespaceConfig::default(),
            soft_wrap: true,
            word_wrap: false,
            indent_wrap: false,
            scroll_sub_offset: 0,
        }
    }

    #[test]
    fn render_soft_wrap_no_wrap_needed() {
        // "abcdefgh" fits in content_width 16 (width 20, gutter 4) — no wrapping.
        let doc = doc_at("abcdefgh\n", 0);
        let v = wrap_view(&doc, 20, 3);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        insta::assert_snapshot!(out, @r"
          1 abcdefgh
        ~
        ~
         1:1 [scratch]│ NOR
        ");
    }

    #[test]
    fn render_soft_wrap_basic() {
        // gutter_width = 4, total width 8 → content_width 4.
        // "abcdefgh" wraps into "abcd" + "efgh".
        let doc = doc_at("abcdefgh\n", 0);
        let v = wrap_view(&doc, 8, 4);
        assert_eq!(v.content_width(), 4);
        let out = render_to_string(&editor_for(doc, v), 8, 5);
        insta::assert_snapshot!(out, @r"
          1 abcd
            efgh
        ~
        ~
         1:1 [sc
        ");
    }

    #[test]
    fn render_soft_wrap_mixed_lines() {
        // "hi" (fits) + "abcdefgh" (wraps to 2 rows at content_width 4).
        let doc = doc_at("hi\nabcdefgh\n", 0);
        let v = wrap_view(&doc, 8, 4);
        let out = render_to_string(&editor_for(doc, v), 8, 5);
        insta::assert_snapshot!(out, @r"
          1 hi
          2 abcd
            efgh
        ~
         1:1 [sc
        ");
    }

    #[test]
    fn render_soft_wrap_clips_to_height() {
        // Line wraps to 3 rows but viewport is only 2.
        let doc = doc_at("abcdefghijkl\n", 0);
        let v = wrap_view(&doc, 8, 2);
        let out = render_to_string(&editor_for(doc, v), 8, 3);
        insta::assert_snapshot!(out, @r"
          1 abcd
            efgh
         1:1 [sc
        ");
    }

    #[test]
    fn render_soft_wrap_cursor_on_continuation() {
        // Cursor at char 6 ('g') → second wrapped row.
        let doc = doc_at("abcdefgh\n", 6);
        let v = wrap_view(&doc, 8, 3);
        let out = render_to_string(&editor_for(doc, v), 8, 4);
        insta::assert_snapshot!(out, @r"
          1 abcd
            efgh
        ~
         1:7 [sc
        ");
    }

    #[test]
    fn render_soft_wrap_empty_buffer() {
        let doc = doc_at("\n", 0);
        let v = wrap_view(&doc, 20, 3);
        let out = render_to_string(&editor_for(doc, v), 20, 4);
        insta::assert_snapshot!(out, @r"
          1
        ~
        ~
         1:1 [scratch]│ NOR
        ");
    }
}
