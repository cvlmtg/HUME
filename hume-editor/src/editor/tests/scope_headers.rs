// Scope header rows: the first lines of the scopes a pane's top rows are
// inside, painted over those rows from the grammar's `context.scm`.
//
// Requires the `rust`, `markdown` and `markdown.inline` grammar fixtures
// plus their `context.scm`: run scripts/fetch-test-grammars.sh.

use super::*;
use hume_core::{OptionalQuery, QueryFiles};
use hume_engine::pipeline::Direction;
use hume_engine::scope_headers::PaneRow;
use hume_engine::types::CellContent;
use hume_grid::{Grid, Position, Rect};
use hume_rope::line::ContentLine;
use test_fixtures::{grammar_query_path, require_grammars, require_helix_query};

/// The frame most tests here draw.
const VIEW: Rect = Rect::new(0, 0, 80, 25);

/// The frame the header snapshots draw.
const SNAP_VIEW: Rect = Rect::new(0, 0, 30, 16);

/// An editor on `source` with the `rust` grammar and its `context.scm`, the
/// initial parse drained. `source` is plain text; the cursor starts on its
/// first cluster. Indent guides are off, so a row reads as its line's text.
fn rust_context_editor(source: &str) -> Editor {
    let mut ed = rust_context_editor_undrained(source);
    ed.reparse_stale_buffers();
    ed
}

/// [`rust_context_editor`] with the initial parse still queued.
fn rust_context_editor_undrained(source: &str) -> Editor {
    let mut ed = super::structural::grammar_editor_undrained(
        "rust",
        "tree_sitter_rust",
        &[OptionalQuery::Context],
        &cursor_on_first_cluster(source),
    );
    ed.state.settings.indent_guides = false;
    ed
}

/// `source` marked up with a cursor on its first cluster, an ASCII char.
fn cursor_on_first_cluster(source: &str) -> String {
    format!("-[{}]>{}", &source[..1], &source[1..])
}

/// An editor on `source` as a markdown buffer with markdown's injections and
/// `context.scm` and rust's `context.scm`, the initial parse drained.
fn markdown_context_editor(source: &str) -> Editor {
    require_grammars(&["markdown", "markdown.inline", "rust"]);
    require_helix_query("markdown", OptionalQuery::Context.file_name());
    require_helix_query("rust", OptionalQuery::Context.file_name());
    super::injections_editor::markdown_editor_with_queries(
        source,
        &Dirs::none(),
        &[],
        &[OptionalQuery::Injections, OptionalQuery::Context],
        &[OptionalQuery::Context],
    )
    .0
}

/// Open `source` as a second buffer in a new focused split and set its
/// language to markdown, as `:set buffer language=markdown` does.
fn split_markdown_buffer(ed: &mut Editor, source: &str) -> PaneId {
    let bid = ed.open_scratch(BufferText::from(source));
    let pid = focused_split(ed, bid);
    run_set(ed, "buffer language=markdown").expect("the setting applies");
    pid
}

/// An `impl` holding two functions with 30-statement bodies:
/// line 0 `impl S {`, line 1 `fn a`, its body 2..=31, its `}` 32, a blank
/// line 33, `fn b` 34, its body 35..=64, its `}` 65, the `impl`'s `}` 66.
fn two_fn_impl() -> String {
    let body = |name: &str| -> String {
        (0..30)
            .map(|i| format!("        let {name}{i} = {i};\n"))
            .collect()
    };
    format!(
        "impl S {{\n    fn a() {{\n{}    }}\n\n    fn b() {{\n{}    }}\n}}\n",
        body("x"),
        body("y")
    )
}

/// `depth` nested Markdown headings, `#` to `depth` `#`s, each on its own
/// line, then 60 paragraph lines inside the innermost section.
fn nested_markdown(depth: usize) -> String {
    let headings: String = (1..=depth)
        .map(|level| {
            format!(
                "{} {}\n",
                "#".repeat(level),
                char::from(b'a' + level as u8 - 1)
            )
        })
        .collect();
    let body: String = (0..60).map(|i| format!("text {i}\n")).collect();
    headings + &body
}

/// The pane `pid`'s rect on the last drawn frame and its gutter width.
pub(super) fn drawn_pane(ed: &Editor, buf: &Grid, pid: PaneId) -> (Rect, u16) {
    let (width, height) = buf.size();
    let (origin, size) = (0..height)
        .flat_map(|y| (0..width).map(move |x| Position::new(x, y)))
        .find_map(|pos| {
            ed.view
                .pane_hit(pos)
                .filter(|hit| hit.pane() == pid && hit.x() == 0 && hit.y() == 0)
                .map(|hit| (pos, hit.size()))
        })
        .expect("the pane was drawn");
    let gutter = ed.state.pane_geometry(&ed.view, pid, size).gutter_width;
    (
        Rect::new(origin.x, origin.y, size.width(), size.height()),
        gutter,
    )
}

/// The text of pane `pid`'s first `n` rows past its gutter, trailing blanks
/// dropped.
pub(super) fn pane_rows(ed: &Editor, buf: &Grid, pid: PaneId, n: u16) -> Vec<String> {
    let (rect, gutter) = drawn_pane(ed, buf, pid);
    (0..n)
        .map(|row| {
            (rect.x + gutter..rect.right())
                .map(|x| cell(buf, x, rect.y + row))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// One row a pane draws, read from the layout the frame draws by.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DrawnRow {
    /// The buffer line the row shows.
    pub line: usize,
    /// The buffer chars the row draws as themselves, as runs of consecutive
    /// chars. Text a conceal draws in their place is not among them.
    pub shown: Vec<std::ops::Range<usize>>,
}

/// A [`DrawnRow`] showing the one run `shown` of `line`'s chars.
pub(super) fn drawn(line: usize, shown: std::ops::Range<usize>) -> DrawnRow {
    DrawnRow {
        line,
        shown: vec![shown],
    }
}

/// `row` as a [`DrawnRow`]. The display line is read whole, whatever the
/// pane's width or horizontal scroll.
fn read_row(line: ContentLine, row: hume_engine::display_lines::RenderDisplayLine<'_>) -> DrawnRow {
    let mut shown: Vec<std::ops::Range<usize>> = Vec::new();
    for cell in &row.graphemes[row.display_line.graphemes.clone()] {
        let (CellContent::Grapheme, Some(pos)) = (cell.content, cell.pos) else {
            continue;
        };
        let start = pos.offset().index();
        let len = row.line_text[cell.byte_range.as_byte_range()]
            .chars()
            .count();
        match shown.last_mut() {
            Some(run) if run.end == start => run.end += len,
            _ => shown.push(start..start + len),
        }
    }
    DrawnRow {
        line: line.index(),
        shown,
    }
}

/// Rows `0..n` of pane `pid` as the last frame drew them.
pub(super) fn drawn_rows(ed: &mut Editor, pid: PaneId, n: u16) -> Vec<DrawnRow> {
    let size = ed.pane_size(pid);
    let layout_key = ed.state.layout_key(&ed.view, pid, size);
    let bid = ed.view.panes[pid].buffer_id();
    let Editor { state, view, .. } = ed;
    let rope = state.buffers.get(bid).text().rope();
    let mut rows: Vec<Result<DrawnRow, ContentLine>> = Vec::new();
    {
        let (mut dlm, viewport) = crate::editor::frame::settled_pane_view(
            &state.buffers,
            &mut state.panes.entries,
            &mut view.panes,
            pid,
            &layout_key,
        );
        let geo = layout_key
            .geometry(state.buffers.get(bid).syntax_source())
            .expect("the pane has rows");
        let pane_rows = geo.rows(&mut dlm, viewport);
        for y in 0..n {
            rows.push(match pane_rows.row_at(&mut dlm, y) {
                PaneRow::Body(pos) => Ok(read_row(pos.line, dlm.render_display_line(pos))),
                PaneRow::Header(line) => Err(line),
            });
        }
    }
    let pane = &mut view.panes[pid];
    let mut header_dlm =
        pane.header_layout
            .display_map(rope, &pane.providers, &layout_key.inputs());
    rows.into_iter()
        .map(|row| {
            row.unwrap_or_else(|line| {
                let pos = header_dlm.first_content_display_line(line);
                read_row(line, header_dlm.render_display_line(pos))
            })
        })
        .collect()
}

/// The buffer line each of rows `0..n` of pane `pid` shows.
pub(super) fn drawn_lines(ed: &mut Editor, pid: PaneId, n: u16) -> Vec<usize> {
    drawn_rows(ed, pid, n).iter().map(|row| row.line).collect()
}

/// Rows `0..n` of the focused pane.
fn top_rows(ed: &Editor, buf: &Grid, n: u16) -> Vec<String> {
    pane_rows(ed, buf, ed.view.focused_pane(), n)
}

/// Line `n` of `source`.
fn line_of(source: &str, n: usize) -> String {
    source.lines().nth(n).expect("the line exists").to_owned()
}

/// The buffer line at the top of pane `pid`.
fn top_line(ed: &Editor, pid: PaneId) -> usize {
    ed.pane_viewport(pid).top().line.index()
}

/// Turn the wheel `ticks` notches down over pane `pid` of a [`VIEW`] frame,
/// one line a notch, and return the frame drawn after.
fn wheel_down(ed: &mut Editor, pid: PaneId, ticks: usize) -> Grid {
    let buf = ed.render_to_buf(VIEW);
    let (rect, _) = drawn_pane(ed, &buf, pid);
    ed.state.settings.mouse_scroll_lines = 1;
    for _ in 0..ticks {
        ed.handle_input(mouse_wheel_at(rect.x, rect.y, true));
    }
    ed.render_to_buf(VIEW)
}

/// A pane on `bid` built as `:split` builds one, gutter included, focused.
fn focused_split(ed: &mut Editor, bid: BufferId) -> PaneId {
    let pid = ed.split_onto(bid, Direction::Vertical);
    ed.state.focus.set_for_test(&mut ed.view, pid);
    pid
}

#[test]
fn push_up_drops_the_fn_header_when_its_closing_brace_is_under_it() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();

    wheel_down(&mut ed, pid, 10);
    assert_eq!(top_line(&ed, pid), 10);
    assert_eq!(drawn_lines(&mut ed, pid, 3), [0, 1, 12]);

    // The first fn's `}` (line 32) is the line at row 1.
    wheel_down(&mut ed, pid, 21);
    assert_eq!(top_line(&ed, pid), 31);
    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 32]);

    // The blank line after it is at row 1: still only the `impl` header.
    wheel_down(&mut ed, pid, 1);
    assert_eq!(top_line(&ed, pid), 32);
    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 33]);
}

#[test]
fn short_pane_keeps_two_headers_by_the_outer_level_setting() {
    let mut ed = markdown_context_editor(&nested_markdown(3));
    seek_to_line(&mut ed, 25);
    let rect = Rect::new(0, 0, 20, 6);

    ed.render_to_buf(rect);
    let pid = ed.view.focused_pane();
    assert_eq!(drawn_lines(&mut ed, pid, 2), [1, 2]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    ed.render_to_buf(rect);
    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 2]);
}

#[test]
fn tall_pane_keeps_three_headers_by_the_outer_level_setting() {
    let mut ed = markdown_context_editor(&nested_markdown(4));
    seek_to_line(&mut ed, 40);

    ed.render_to_buf(VIEW);
    let pid = ed.view.focused_pane();
    assert_eq!(drawn_lines(&mut ed, pid, 3), [1, 2, 3]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    ed.render_to_buf(VIEW);
    assert_eq!(drawn_lines(&mut ed, pid, 3), [0, 2, 3]);
}

/// Two panes on two buffers of the same nested markdown, each cursor on
/// line 40, with `setting` set on the second buffer only: the editor, the
/// two panes and the source.
fn two_markdown_panes(setting: &str) -> (Editor, PaneId, PaneId, String) {
    let source = nested_markdown(4);
    let mut ed = markdown_context_editor(&source);
    let first = ed.view.focused_pane();
    seek_to_line(&mut ed, 40);
    let second = split_markdown_buffer(&mut ed, &source);
    seek_to_line(&mut ed, 40);
    run_set(&mut ed, setting).expect("the setting applies");
    (ed, first, second, source)
}

#[test]
fn outer_level_is_per_buffer() {
    let (mut ed, first, second, _) = two_markdown_panes("buffer context-outer-level=true");

    ed.render_to_buf(VIEW);
    assert_eq!(drawn_lines(&mut ed, first, 3), [1, 2, 3]);
    assert_eq!(drawn_lines(&mut ed, second, 3), [0, 2, 3]);
}

/// An `impl` line longer than a 30-column pane, over one function with a
/// 20-statement body; with `long_line`, the body's tenth statement runs to
/// column 200.
fn wide_impl(long_line: bool) -> String {
    let body: String = (0..20)
        .map(|i| {
            if long_line && i == 10 {
                format!("        let wide = \"{}\";\n", "w".repeat(180))
            } else {
                format!("        let v{i} = {i};\n")
            }
        })
        .collect();
    format!("impl Iterator for AVeryLongTypeName {{\n    fn next() {{\n{body}    }}\n}}\n")
}

/// The theme, buffer and focused split pane the header snapshots render:
/// gruvbox, `wide_impl`, the cursor on line `cursor_line` of the split pane.
fn snapshot_editor(long_line: bool, cursor_line: usize) -> Editor {
    let mut ed = rust_context_editor(&wide_impl(long_line));
    ed.view.theme = crate::testing::build_snapshot_theme();
    let bid = ed.focused_buffer_id();
    focused_split(&mut ed, bid);
    seek_to_line(&mut ed, cursor_line);
    ed
}

#[test]
fn header_rows_snapshot_wrap_off() {
    let mut ed = snapshot_editor(false, 15);
    pin_no_wrap(&mut ed);
    run_set(&mut ed, "buffer gutter-line-numbers=absolute").expect("the setting applies");
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, SNAP_VIEW);
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_snapshot_wrap_on() {
    let mut ed = snapshot_editor(false, 15);
    run_set(&mut ed, "buffer gutter-line-numbers=absolute").expect("the setting applies");
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, SNAP_VIEW);
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_ignore_horizontal_scroll() {
    let mut ed = snapshot_editor(true, 12);
    pin_no_wrap(&mut ed);
    seek_to_column(&mut ed, 12, 150);
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, SNAP_VIEW);
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_show_no_selection_cursor_or_cursorline() {
    let mut ed = snapshot_editor(false, 15);
    let impl_line = line_of(&wide_impl(false), 0);
    let deep = char_at_column(&ed, 15, 0).index();
    select(
        &mut ed,
        &[(0, impl_line.chars().count() - 1), (deep, deep)],
        1,
    );
    let covered: Vec<String> = ed
        .current_view()
        .iter()
        .map(|sel| sel.slice().to_string())
        .collect();
    assert_eq!(
        covered,
        [impl_line, " ".to_owned()],
        "premise: a secondary selection covers the impl line"
    );
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, SNAP_VIEW);
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_use_popup_background_when_ui_context_is_unset() {
    let mut ed = snapshot_editor(false, 15);
    pin_no_wrap(&mut ed);
    let pid = ed.view.focused_pane();
    let buf = ed.render_to_buf(SNAP_VIEW);
    let popup_bg = ed
        .view
        .theme
        .resolve_by_name(hume_engine::types::Scope(
            hume_engine::theme::ui_scopes::POPUP,
        ))
        .bg;
    assert!(
        popup_bg.is_some(),
        "premise: gruvbox's ui.popup has a background"
    );
    assert!(
        !ed.view
            .theme
            .has_entry(hume_engine::theme::ui_scopes::CONTEXT),
        "premise: gruvbox leaves ui.context unset"
    );

    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    assert!(gutter > 0, "premise: the split pane draws a gutter");
    insta::assert_snapshot!(pane_rows(&ed, &buf, pid, 2).join("\n"), @"
    impl Iterator for AVery
        fn next() {
    ");
    for row in 0..2 {
        for x in rect.x..rect.right() {
            assert_eq!(
                buf[(x, rect.y + row)].style().bg,
                popup_bg,
                "header row {row}, column {x}"
            );
        }
    }
}

/// Line 0 heads row 0 from above the top. Line 1 heads row 1 while it is the
/// top line itself, inside the range the pane's decorations are synced for.
/// Line 5 is a body row and shows each decoration. A header row draws the
/// highlight on its line and none of the hint, EOL text or sign.
#[test]
fn header_rows_draw_highlights_but_no_layout_decorations() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    pin_no_wrap(&mut ed);
    wheel_down(&mut ed, pid, 1);

    let marked = ed.view.registry.intern("ui.selection");
    let marked_bg = ed
        .view
        .theme
        .resolve_by_name(hume_engine::types::Scope("ui.selection"))
        .bg;
    let header_bg = ed.view.theme.ui().context.bg;
    assert!(
        marked_bg.is_some() && marked_bg != header_bg,
        "premise: the highlight's background differs from the header rows'"
    );
    let decorated = [0, 1, 5].map(hume_rope::line::ContentLine::new);
    let columns_0_and_4 =
        decorated.map(|line| [0, 4].map(|col| char_at_column(&ed, line.index(), col)));
    let decorations = &mut ed.state.config.decorations;
    decorations.set_inlay_hints(
        "test".to_owned(),
        bid,
        columns_0_and_4
            .iter()
            .map(|&[_, col_4]| hume_decorations::InlayHintEntry {
                pos: col_4,
                text: "H:".into(),
                side: hume_core::Side::Before,
            })
            .collect(),
    );
    decorations.set_eol_text(
        "test".to_owned(),
        bid,
        decorated
            .iter()
            .map(|&line| hume_decorations::EolTextEntry {
                line,
                text: " EOL".into(),
                scope: marked,
                hide_on_insert_line: false,
            })
            .collect(),
    );
    decorations.register_sign_source("test".to_owned(), bid, 0);
    decorations.set_signs(
        "test".to_owned(),
        bid,
        decorated
            .iter()
            .map(|&line| hume_decorations::SignEntry {
                line,
                text: "!".into(),
                scope: marked,
            })
            .collect(),
    );
    decorations.set_extra_highlights(
        "test".to_owned(),
        bid,
        columns_0_and_4
            .iter()
            .map(|&[col_0, col_4]| hume_decorations::ExtraHighlightEntry {
                start: col_0,
                end: col_4,
                scope: marked,
            })
            .collect(),
    );
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(top_line(&ed, pid), 1, "premise: `fn a()` is the top line");

    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    let gutter_text = |row: u16| -> String {
        (rect.x..rect.x + gutter)
            .map(|x| cell(&buf, x, rect.y + row))
            .collect()
    };
    insta::assert_snapshot!(
        pane_rows(&ed, &buf, pid, 5).remove(4),
        @"    H:    let x3 = 3; EOL"
    );
    assert!(
        gutter_text(4).contains('!'),
        "premise: a body row draws its line's sign"
    );
    insta::assert_snapshot!(pane_rows(&ed, &buf, pid, 2).join("\n"), @"
    impl S {
        fn a() {
    ");
    for row in 0..2 {
        assert!(!gutter_text(row).contains('!'), "header row {row}'s gutter");
        for x in rect.x + gutter..rect.right() {
            let highlighted = x < rect.x + gutter + 4;
            assert_eq!(
                buf[(x, rect.y + row)].style().bg,
                if highlighted { marked_bg } else { header_bg },
                "header row {row}, column {x}"
            );
        }
    }
}

#[test]
fn two_panes_on_one_buffer_show_their_own_headers() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let first = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let second = ed.split_onto(bid, Direction::Horizontal);

    wheel_down(&mut ed, first, 33);
    wheel_down(&mut ed, second, 10);
    assert_eq!(top_line(&ed, first), 33);
    assert_eq!(top_line(&ed, second), 10);

    assert_eq!(drawn_lines(&mut ed, first, 2), [0, 34]);
    assert_eq!(drawn_lines(&mut ed, second, 3), [0, 1, 12]);
}

#[test]
fn context_enabled_off_on_one_buffer_only() {
    let (mut ed, first, second, _) = two_markdown_panes("buffer context-enabled=false");

    ed.render_to_buf(VIEW);
    assert_eq!(drawn_lines(&mut ed, first, 1), [1]);
    let second_top = top_line(&ed, second);
    assert_eq!(drawn_lines(&mut ed, second, 1), [second_top]);
}

#[test]
fn defaults_show_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    wheel_down(&mut ed, pid, 10);

    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 1]);
    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(if (get-buffer-option bid "context-outer-level") "true" "false")"#
        ),
        "false"
    );
}

/// Header rows over lines 0 and 1 above a top line of 10: the body's range
/// comes back under `'ranges`, the two header lines under `'headers` as one
/// range.
#[test]
fn buffer_viewport_ranges_name_the_header_lines_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    wheel_down(&mut ed, pid, 10);
    assert_eq!(
        drawn_lines(&mut ed, pid, 2),
        [0, 1],
        "premise: two header rows"
    );
    assert_eq!(top_line(&ed, pid), 10, "premise: the view scrolled");

    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(let ([shown (buffer-viewport-ranges bid)])
                 (if (and (equal? (hash-ref shown 'ranges) (list (viewport-range bid)))
                          (equal? (hash-ref shown 'headers)
                                  (list (hash 'start-line 0 'end-line 2))))
                     "headers apart"
                     (to-string shown)))"#
        ),
        "headers apart"
    );
}

/// A header line that another pane's body shows is in `'ranges` already,
/// so `'headers` leaves it out.
#[test]
fn buffer_viewport_ranges_leave_out_header_lines_a_range_covers() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let top_pane = ed.view.focused_pane();
    let scrolled = focused_split(&mut ed, bid);
    wheel_down(&mut ed, scrolled, 10);
    assert_eq!(
        drawn_lines(&mut ed, scrolled, 2),
        [0, 1],
        "premise: two header rows"
    );
    assert_eq!(
        top_line(&ed, top_pane),
        0,
        "premise: the other pane shows line 0"
    );

    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(to-string (hash-ref (buffer-viewport-ranges bid) 'headers))"#
        ),
        "()"
    );
}

/// Two panes whose header rows draw lines 0 and 34, and 0 and 1: `'headers`
/// holds each line once, in order, lines 0 and 1 joined into one range.
#[test]
fn buffer_viewport_ranges_join_header_lines_across_panes() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let first = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let second = ed.split_onto(bid, Direction::Horizontal);
    wheel_down(&mut ed, first, 36);
    wheel_down(&mut ed, second, 10);
    assert_eq!(
        drawn_lines(&mut ed, first, 2),
        [0, 34],
        "premise: the first pane's header rows"
    );
    assert_eq!(
        drawn_lines(&mut ed, second, 2),
        [0, 1],
        "premise: the second pane's header rows"
    );
    for pid in [first, second] {
        let body = crate::editor::geometry::shown_lines(&mut ed.state, &mut ed.view, pid).visible;
        assert!(
            [0, 1, 34]
                .into_iter()
                .all(|line| !body.contains(ContentLine::new(line))),
            "premise: no body shows a header line: {body:?}"
        );
    }

    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(let ([headers (hash-ref (buffer-viewport-ranges bid) 'headers)])
                 (if (equal? headers
                             (list (hash 'start-line 0 'end-line 2)
                                   (hash 'start-line 34 'end-line 35)))
                     "joined"
                     (to-string headers)))"#
        ),
        "joined"
    );
}

/// With `fn b` as the top line, header row 1 draws that line itself:
/// `viewport-range` starts at the line on the first row below the header
/// rows, so it holds neither header line.
#[test]
fn viewport_range_starts_on_the_first_body_row() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    wheel_down(&mut ed, pid, 34);
    let drawn = drawn_lines(&mut ed, pid, 3);
    assert_eq!(
        drawn[..2],
        [0, top_line(&ed, pid)],
        "premise: header row 1 draws the top line"
    );

    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(number->string (hash-ref (viewport-range bid) 'start-line))"#
        ),
        drawn[2].to_string()
    );
}

#[test]
fn pane_of_height_one_shows_no_header() {
    let source = nested_markdown(3);
    let mut ed = markdown_context_editor(&source);
    seek_to_line(&mut ed, 25);
    ed.render_to_buf(Rect::new(0, 0, 20, 2));
    let pid = ed.view.focused_pane();
    let top = top_line(&ed, pid);
    assert_eq!(drawn_lines(&mut ed, pid, 1), [top]);
}

/// A tall virtual block under line 10 leaves the cursor there while the
/// wheel scrolls the view: at top line 9 the cursor's line is under header
/// row 1, and its caret is hidden as the line is.
#[test]
fn a_parked_cursor_under_a_header_row_has_no_caret() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    seek_to_column(&mut ed, 10, STATEMENT_X);
    ed.view.panes[pid].providers.add_decoration_source(Box::new(
        hume_engine::test_doubles::VirtualLineBlock::uniform(
            hume_core::Side::Before,
            hume_rope::line::ContentLine::new(11),
            40,
            "V",
        ),
    ));
    wheel_down(&mut ed, pid, 7);

    assert_eq!(top_line(&ed, pid), 9, "premise: the view scrolled");
    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 10, STATEMENT_X),
        "premise: the cursor stayed on line 10"
    );
    assert_eq!(
        drawn_lines(&mut ed, pid, 2),
        [0, 1],
        "premise: two header rows cover rows 0 and 1"
    );
    assert_eq!(frame(&mut ed, 80, 25).cursor_content_pos, None);
}

/// A pane parked behind a tall virtual block under the cursor's `fn` line,
/// so that line is a header row while the primary cursor is still on it: its
/// number is drawn as any other line's, not as the cursor line's.
#[test]
fn a_header_row_on_the_cursors_line_does_not_mark_its_number() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    ed.view.theme = crate::testing::build_snapshot_theme();
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    seek_to_line(&mut ed, 1);
    ed.view.panes[pid].providers.add_decoration_source(Box::new(
        hume_engine::test_doubles::VirtualLineBlock::uniform(
            hume_core::Side::Before,
            hume_rope::line::ContentLine::new(2),
            40,
            "V",
        ),
    ));
    let buf = wheel_down(&mut ed, pid, 3);

    let fn_line_start = char_at_column(&ed, 1, 0);
    assert_eq!(
        ed.current_view().primary().head().offset(),
        fn_line_start,
        "premise: the cursor stays on the fn line"
    );
    assert_eq!(
        top_line(&ed, pid),
        2,
        "premise: the view parks inside the block"
    );
    assert_eq!(
        drawn_lines(&mut ed, pid, 2),
        [0, 1],
        "premise: the fn line is header row 1"
    );

    let theme = &ed.view.theme;
    let linenr = theme
        .resolve_by_name(hume_engine::types::Scope("ui.linenr"))
        .fg;
    let selected = theme
        .resolve_by_name(hume_engine::types::Scope("ui.linenr.selected"))
        .fg;
    assert_ne!(linenr, selected, "premise: the theme tells the two apart");
    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    let number = (rect.x..rect.x + gutter)
        .find(|&x| cell(&buf, x, rect.y + 1) == "2")
        .expect("header row 1 numbers its line 2");
    assert_eq!(buf[(number, rect.y + 1)].style().fg, linenr);
}

/// The focused pane after a settled `view`-sized frame: where its cursor is
/// drawn, as `(row, text of the cell there)`, and how many of its top rows
/// are scope headers (leading rows whose text is a line above the top line).
fn cursor_row_and_headers(ed: &mut Editor, view: Rect) -> (u16, String, usize) {
    let (cursor_x, row) = frame(ed, view.width, view.height)
        .cursor_content_pos
        .expect("the cursor is on screen");
    let buf = ed.render_to_buf(view);
    let pid = ed.view.focused_pane();
    let (rect, gutter) = drawn_pane(ed, &buf, pid);
    let shown = cell(&buf, rect.x + gutter + cursor_x, rect.y + row);
    let top = top_line(ed, pid);
    let headers = drawn_lines(ed, pid, 3)
        .into_iter()
        .take_while(|&line| line < top)
        .count();
    (row, shown, headers)
}

/// The text of the cluster under the focused pane's primary cursor, as a
/// cell shows it.
fn cursor_cluster(ed: &Editor) -> String {
    let cluster = ed.current_view().primary().slice().to_string();
    if cluster == "\n" {
        " ".to_owned()
    } else {
        cluster
    }
}

/// The column of the `x` of a statement in [`two_fn_impl`]'s bodies.
const STATEMENT_X: usize = 12;

/// Place the cursor on column `col` of line `line`, a line of ASCII.
fn seek_to_column(ed: &mut Editor, line: usize, col: usize) {
    let target = char_at_column(ed, line, col).index();
    set_cursor(ed, target);
}

#[test]
fn landed_parse_resettles_the_view() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor_undrained(&source);
    ed.state.settings.scroll_margin = 1;
    let pid = ed.view.focused_pane();
    let frame_without_settle = |ed: &mut Editor| {
        ed.resize_terminal(80, 25);
        ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());
    };

    seek_to_column(&mut ed, 50, STATEMENT_X);
    frame_without_settle(&mut ed);
    let top = top_line(&ed, pid);
    seek_to_column(&mut ed, top + 1, STATEMENT_X);
    frame_without_settle(&mut ed);
    assert_eq!(
        top_line(&ed, pid),
        top,
        "premise: the cursor is in the band"
    );

    let (row, shown, headers) = cursor_row_and_headers(&mut ed, VIEW);
    assert!(headers >= 1, "premise: the landed parse shows headers");
    assert!(
        usize::from(row) > headers,
        "row {row} is not below {headers} headers"
    );
    assert_eq!(shown, cursor_cluster(&ed));
    assert!(!ed.state.panes.entries.shown(&ed.view.panes, pid).parked);
}

#[test]
fn cursor_moving_up_never_lands_on_a_header_row() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    ed.state.settings.scroll_margin = 1;
    seek_to_column(&mut ed, 50, STATEMENT_X);
    let mut deepest = 0;
    for step in 0..40 {
        press(&mut ed, "k");
        let (row, shown, headers) = cursor_row_and_headers(&mut ed, VIEW);
        deepest = deepest.max(headers);
        assert!(
            usize::from(row) > headers,
            "step {step}: row {row} is not below {headers} headers"
        );
        assert_eq!(shown, cursor_cluster(&ed), "step {step}");
    }
    assert!(deepest >= 2, "premise: the walk crosses a two-header view");
}

/// A 3-row pane under the statusline: one row of text below the most
/// headers it shows, which the margin's clamp to that height still clears.
#[test]
fn cursor_in_a_short_pane_stays_below_the_headers() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    seek_to_column(&mut ed, 50, STATEMENT_X);
    let mut deepest = 0;
    for step in 0..40 {
        press(&mut ed, "k");
        let (row, shown, headers) = cursor_row_and_headers(&mut ed, Rect::new(0, 0, 80, 4));
        deepest = deepest.max(headers);
        assert!(
            usize::from(row) >= headers,
            "step {step}: row {row} is under {headers} headers"
        );
        assert_eq!(shown, cursor_cluster(&ed), "step {step}");
    }
    assert!(
        deepest >= 1,
        "premise: the walk crosses a view with a header"
    );
}

#[test]
fn settling_twice_does_not_scroll() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    ed.state.settings.scroll_margin = 1;
    let pid = ed.view.focused_pane();
    seek_to_column(&mut ed, 50, STATEMENT_X);
    for _ in 0..5 {
        press(&mut ed, "k");
    }
    draw(&mut ed);
    let settled = ed.pane_viewport(pid).top();
    draw(&mut ed);
    assert_eq!(ed.pane_viewport(pid).top(), settled);
}

/// An `on-viewport-change` handler gets the lines the pane's header rows
/// draw, after the pane's body range: a range of one line per row, top row
/// first.
#[test]
fn on_viewport_change_hands_the_header_lines_to_its_handler() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    ed.state.settings.hook_viewport_debounce_ms = 0;
    let pid = ed.view.focused_pane();
    run(
        &mut ed,
        tmp.path(),
        r#"(define seen "none")
           (register-hook! 'on-viewport-change
             (lambda (pane range headers)
               (set! seen (to-string (map (lambda (r) (list (hash-ref r 'start-line)
                                                            (hash-ref r 'end-line)))
                                          headers)))))"#,
    );
    wheel_down(&mut ed, pid, 10);
    ed.drain_async_sources();
    ed.settle();
    let drawn = drawn_lines(&mut ed, pid, 2);
    assert_eq!(drawn, [0, 1], "premise: two header rows");

    assert_eq!(
        log_probe(&mut ed, tmp.path(), "seen"),
        format!(
            "(({} {}) ({} {}))",
            drawn[0],
            drawn[0] + 1,
            drawn[1],
            drawn[1] + 1
        )
    );
}

/// With more scopes than header rows, `context-outer-level` swaps which
/// header the rows keep over the same body lines, and that alone fires
/// `on-viewport-change` again.
#[test]
fn on_viewport_change_fires_when_only_the_header_rows_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_context_editor(&nested_markdown(4));
    ed.state.settings.hook_viewport_debounce_ms = 0;
    let pid = ed.view.focused_pane();
    seek_to_line(&mut ed, 40);
    ed.render_to_buf(VIEW);
    assert_eq!(
        drawn_lines(&mut ed, pid, 3),
        [1, 2, 3],
        "premise: the inner three headers"
    );
    run(
        &mut ed,
        tmp.path(),
        r#"(define fires 0)
           (register-hook! 'on-viewport-change (lambda (pane range headers) (set! fires (+ fires 1))))
           (define-typed-command! "fires" "" (lambda (bid) (log! 'info (number->string fires))))"#,
    );
    let fires = |ed: &mut Editor| -> usize {
        type_cmd(ed, ":fires");
        status(ed).parse().expect("a count")
    };
    ed.drain_async_sources();
    ed.settle();
    let body = crate::editor::geometry::shown_lines(&mut ed.state, &mut ed.view, pid).visible;
    let before = fires(&mut ed);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    ed.render_to_buf(VIEW);
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        drawn_lines(&mut ed, pid, 3),
        [0, 2, 3],
        "premise: the outer header kept"
    );
    assert_eq!(
        crate::editor::geometry::shown_lines(&mut ed.state, &mut ed.view, pid).visible,
        body,
        "premise: the body shows the same lines"
    );
    assert_eq!(fires(&mut ed), before + 1);
}

#[test]
fn context_enabled_off_keeps_the_margin() {
    let source = two_fn_impl();
    let mut with_grammar = rust_context_editor(&source);
    run_set(&mut with_grammar, "buffer context-enabled=false").expect("the setting applies");
    let mut without = editor_from(&cursor_on_first_cluster(&source));
    for ed in [&mut with_grammar, &mut without] {
        ed.state.settings.scroll_margin = 1;
        ed.state.settings.indent_guides = false;
        seek_to_column(ed, 50, STATEMENT_X);
    }
    for step in 0..40 {
        for ed in [&mut with_grammar, &mut without] {
            press(ed, "k");
        }
        let with_row = cursor_row_and_headers(&mut with_grammar, VIEW).0;
        let without_row = cursor_row_and_headers(&mut without, VIEW).0;
        assert_eq!(with_row, without_row, "step {step}");
    }
}

// ── Mouse ────────────────────────────────────────────────────────────

/// Press at content cell `(x, y)` of pane `pid` on the frame `buf` drew.
fn press_at(ed: &mut Editor, buf: &Grid, pid: PaneId, x: u16, y: u16) {
    let (rect, gutter) = drawn_pane(ed, buf, pid);
    ed.handle_input(mouse_left_down(rect.x + gutter + x, rect.y + y));
}

/// The char offset of column `col` of line `line`, a line of ASCII.
fn char_at_column(ed: &Editor, line: usize, col: usize) -> hume_rope::offset::CharOffset {
    let start = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(line),
    );
    co(start.index() + col)
}

#[test]
fn press_on_a_header_row_selects_that_line_and_reveals_it() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    let buf = wheel_down(&mut ed, pid, 10);
    assert_eq!(drawn_lines(&mut ed, pid, 3), [0, 1, 12]);

    press_at(&mut ed, &buf, pid, 4, 1);

    let sel = ed.current_view().primary();
    assert_eq!(sel.anchor().offset(), char_at_column(&ed, 1, 4));
    assert_eq!(sel.head().offset(), char_at_column(&ed, 1, 4));
    let pressed = state(&ed);
    ed.render_to_buf(VIEW);
    assert_eq!(top_line(&ed, pid), 0);
    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 1]);

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), pressed);
}

/// [`rust_context_editor`] opened as startup opens an editor, so its pane
/// carries the conceal provider. The returned guard keeps the file alive.
fn decorated_rust_context_editor(source: &str) -> (Editor, tempfile::TempPath) {
    require_grammars(&["rust"]);
    let files = QueryFiles::new(grammar_query_path("rust")).with(
        OptionalQuery::Context,
        require_helix_query("rust", OptionalQuery::Context.file_name()),
    );
    let (mut ed, file) = super::conceal_bridge::decorated_editor(&cursor_on_first_cluster(source));
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar_with(&mut ed, "rust", "tree_sitter_rust", &files);
    super::language::attach_host(&mut ed, "");
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language_explicit(bid, Some(lang), ConfigOrigin::User)
        .expect_open();
    ed.state.settings.indent_guides = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while ed.doc().current_layers().is_none() {
        ed.render_to_buf(VIEW);
        assert!(
            std::time::Instant::now() < deadline,
            "the first parse never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (ed, file)
}

#[test]
fn press_on_a_header_row_after_conceals_clear_leaves_the_header_raw_next_frame() {
    let source = two_fn_impl();
    let (mut ed, _file) = decorated_rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    let fn_kw = char_at_column(&ed, 1, 4).index();
    super::conceal_bridge::set(
        &mut ed,
        "md",
        vec![super::conceal_bridge::hiding(fn_kw, fn_kw + 3)],
    );
    let buf = wheel_down(&mut ed, pid, 10);
    let line_1 = char_at_column(&ed, 1, 0).index();
    assert_eq!(
        drawn_rows(&mut ed, pid, 2)[1],
        DrawnRow {
            line: 1,
            shown: vec![line_1..fn_kw, fn_kw + 3..line_1 + 12],
        },
        "premise: the header row draws without `fn `"
    );

    // No frame between the clear and the press, so the pane's conceal
    // mirror still holds the entry.
    super::conceal_bridge::set(&mut ed, "md", vec![]);
    press_at(&mut ed, &buf, pid, 4, 1);
    set_cursor(&mut ed, 0);
    wheel_down(&mut ed, pid, 10);

    assert_eq!(
        drawn_rows(&mut ed, pid, 2)[1],
        drawn(1, line_1..line_1 + 12)
    );
}

#[test]
fn press_on_a_header_row_lands_on_the_cluster_its_concealed_row_draws() {
    let source = two_fn_impl();
    let (mut ed, _file) = decorated_rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    let fn_kw = char_at_column(&ed, 1, 4).index();
    super::conceal_bridge::set(
        &mut ed,
        "md",
        vec![super::conceal_bridge::hidden(fn_kw, fn_kw + 3, "λ")],
    );
    let buf = wheel_down(&mut ed, pid, 10);
    let line_1 = char_at_column(&ed, 1, 0).index();
    assert_eq!(
        drawn_rows(&mut ed, pid, 2)[1],
        DrawnRow {
            line: 1,
            shown: vec![line_1..fn_kw, fn_kw + 3..line_1 + 12],
        },
        "premise: the header row draws `λ` in place of `fn `"
    );
    insta::assert_snapshot!(pane_rows(&ed, &buf, pid, 2)[1], @"    λa() {");

    // Column 5 is `a` on the drawn row; raw, it is the `n` of `fn`.
    press_at(&mut ed, &buf, pid, 5, 1);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 1, 7)
    );
}

#[test]
fn press_on_a_header_counts_columns_from_zero_under_horizontal_scroll() {
    let mut ed = snapshot_editor(true, 12);
    pin_no_wrap(&mut ed);
    seek_to_column(&mut ed, 12, 150);
    let buf = ed.render_to_buf(SNAP_VIEW);
    let pid = ed.view.focused_pane();
    assert!(ed.pane_viewport(pid).horizontal_offset().get() > 0);
    insta::assert_snapshot!(top_rows(&ed, &buf, 1).join("\n"), @"impl Iterator for AVery");

    press_at(&mut ed, &buf, pid, 5, 0);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 0, 5)
    );
}

#[test]
fn press_on_a_wrapped_header_lands_where_the_row_draws_it() {
    let mut ed = snapshot_editor(false, 15);
    let buf = ed.render_to_buf(SNAP_VIEW);
    let pid = ed.view.focused_pane();
    insta::assert_snapshot!(top_rows(&ed, &buf, 1).join("\n"), @"impl Iterator for AVery");

    // Column 20 is past where the line's first wrap segment breaks.
    press_at(&mut ed, &buf, pid, 20, 0);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 0, 20)
    );
}

#[test]
fn drag_onto_a_header_row_extends_to_its_line() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    ed.state.settings.mouse_select = true;
    let pid = ed.view.focused_pane();
    let buf = wheel_down(&mut ed, pid, 10);
    press_at(&mut ed, &buf, pid, 12, 5);
    let anchor = ed.current_view().primary().anchor().offset();
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(drawn_lines(&mut ed, pid, 2), [0, 1]);

    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    ed.handle_input(mouse_drag(rect.x + gutter + 4, rect.y + 1));

    let sel = ed.current_view().primary();
    assert_eq!(sel.anchor().offset(), anchor);
    assert_eq!(sel.head().offset(), char_at_column(&ed, 1, 4));
}

#[test]
fn press_on_row_zero_with_headers_off_selects_the_top_line() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    run_set(&mut ed, "buffer context-enabled=false").expect("the setting applies");
    let pid = ed.view.focused_pane();
    let buf = wheel_down(&mut ed, pid, 10);
    assert_eq!(drawn_lines(&mut ed, pid, 1), [10]);

    press_at(&mut ed, &buf, pid, 0, 0);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 10, 0)
    );
}

/// A diagnostic over `start..end` of `bid`, as the store holds one.
fn error_diagnostic(
    start: hume_rope::offset::CharOffset,
    end: hume_rope::offset::CharOffset,
) -> crate::editor::lsp::diagnostics::StoredDiag {
    crate::editor::lsp::diagnostics::StoredDiag {
        start,
        end,
        severity: hume_core::DiagSeverity::Error,
        message: "boom".to_owned(),
        code: None,
        source: None,
        raw: std::sync::Arc::new(serde_json::Value::Null),
        server: hume_scripting::ServerRef {
            id: hume_lsp::backend::ServerId(0),
            name: hume_core::ServerName::parse("test-server").unwrap(),
        },
        encoding: hume_rope::position_encoding::PositionEncoding::Utf16,
    }
}

/// Line 0 (`impl S {`) heads row 0 from above the viewport: its search
/// match over `impl` and its diagnostic over `S` both draw on the row.
#[test]
fn header_row_above_the_viewport_draws_search_and_diagnostic_highlights() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    pin_no_wrap(&mut ed);
    ed.state.buffers.get_mut(bid).search_pattern =
        crate::editor::search::SearchPattern::compile("impl");
    ed.state.buffer_positions.diagnostics.replace(
        hume_lsp::backend::ServerId(0),
        bid,
        vec![error_diagnostic(
            char_at_column(&ed, 0, 5),
            char_at_column(&ed, 0, 6),
        )],
    );
    let resolve = |name| {
        ed.view
            .theme
            .resolve_by_name(hume_engine::types::Scope(name))
    };
    let search = resolve(hume_engine::theme::CURSOR_MATCH_SEARCH);
    let diagnostic = resolve(hume_engine::theme::diagnostic_scopes::highlight(
        hume_core::DiagSeverity::Error,
    ));
    assert!(
        search.bg.is_some() && diagnostic.underline != Default::default(),
        "premise: the match has a background and the diagnostic an underline"
    );

    let buf = wheel_down(&mut ed, pid, 10);
    assert_eq!(top_line(&ed, pid), 10, "premise: line 0 is above the top");
    assert_eq!(
        drawn_lines(&mut ed, pid, 1),
        [0],
        "premise: row 0 is line 0's header"
    );

    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    let style = |col: u16| buf[(rect.x + gutter + col, rect.y)].style();
    assert_eq!(
        (0..4).map(|col| style(col).bg).collect::<Vec<_>>(),
        [search.bg; 4],
        "`impl`, the search match"
    );
    assert_eq!(
        style(5).underline,
        diagnostic.underline,
        "`S`, the diagnostic"
    );
    assert_eq!(style(4).underline, Default::default(), "the space between");
}

/// The search-match mirror of `pid` as `(line, start byte, end byte)`.
fn search_match_pieces(ed: &Editor, pid: PaneId) -> Vec<(usize, usize, usize)> {
    pane_highlights(ed, pid, hume_engine::providers::HighlightTier::SearchMatch)
        .into_iter()
        .map(|(line, start, end, _)| (line.index(), start.index(), end.index()))
        .collect()
}

/// A match that starts on the line above the viewport still paints its
/// part on the top line.
#[test]
fn search_match_starting_above_the_viewport_paints_its_part_in_it() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    pin_no_wrap(&mut ed);
    // Line 9 is `let x7 = 7;` and line 10 `let x8 = 8;`.
    ed.state.buffers.get_mut(bid).search_pattern =
        crate::editor::search::SearchPattern::compile("x7 = 7;\n        let x8");
    wheel_down(&mut ed, pid, 10);
    assert_eq!(top_line(&ed, pid), 10, "premise: line 9 is above the top");

    assert_eq!(search_match_pieces(&ed, pid), [(10, 0, 14)]);
}

/// A match that starts on a header line paints that header line and the
/// lines it covers that are drawn, and no line between the headers and the
/// top.
#[test]
fn search_match_starting_on_a_header_line_paints_only_the_drawn_lines() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    pin_no_wrap(&mut ed);
    // From `S` on line 0 through `let x0` on line 2.
    ed.state.buffers.get_mut(bid).search_pattern =
        crate::editor::search::SearchPattern::compile("S \\{\n    fn a\\(\\) \\{\n        let x0");
    wheel_down(&mut ed, pid, 10);
    assert_eq!(
        drawn_lines(&mut ed, pid, 3),
        [0, 1, 12],
        "premise: lines 0 and 1 head rows 0 and 1, and the top is line 10"
    );

    assert_eq!(search_match_pieces(&ed, pid), [(0, 5, 8), (1, 0, 12)]);
}

/// The extra-highlight mirror of a pane covers the viewport, one row more,
/// and the pane's header lines, however many lines carry a highlight.
#[test]
fn paint_mirrors_cover_at_most_the_viewport_plus_one_row_and_the_header_lines() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let bid = ed.focused_buffer_id();
    let pid = focused_split(&mut ed, bid);
    pin_no_wrap(&mut ed);
    let marked = ed.view.registry.intern("ui.selection");
    let line_count = source.lines().count();
    ed.state.config.decorations.set_extra_highlights(
        "test".to_owned(),
        bid,
        (0..line_count)
            .map(|line| hume_decorations::ExtraHighlightEntry {
                start: char_at_column(&ed, line, 0),
                end: char_at_column(&ed, line, 1),
                scope: marked,
            })
            .collect(),
    );

    let buf = wheel_down(&mut ed, pid, 10);
    assert_eq!(
        drawn_lines(&mut ed, pid, 3),
        [0, 1, 12],
        "premise: lines 0 and 1 head rows 0 and 1, and the top is line 10"
    );
    let (rect, _) = drawn_pane(&ed, &buf, pid);
    let mut covered: Vec<usize> = ed.state.panes.entries[pid]
        .render
        .highlights(hume_engine::providers::HighlightTier::Extra)
        .iter()
        .map(|&(line, ..)| line.index())
        .collect();
    covered.dedup();

    assert!(
        covered.contains(&0) && covered.contains(&1),
        "the header lines are covered: {covered:?}"
    );
    assert!(
        covered.iter().all(|&line| !(2..10).contains(&line)),
        "no line between the headers and the top: {covered:?}"
    );
    assert!(
        covered.len() <= usize::from(rect.height) + 1 + 2,
        "{} lines for a {}-row pane with two headers",
        covered.len(),
        rect.height
    );
}
