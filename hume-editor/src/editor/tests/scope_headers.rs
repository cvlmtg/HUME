// Scope header rows: the first lines of the scopes a pane's top rows are
// inside, painted over those rows from the grammar's `context.scm`.
//
// Requires the `rust`, `markdown` and `markdown.inline` grammar fixtures
// plus their `context.scm`: run scripts/fetch-test-grammars.sh.

use super::*;
use hume_core::OptionalQuery;
use hume_engine::pipeline::Direction;
use hume_grid::{Grid, Position, Rect};
use test_fixtures::{require_grammars, require_helix_query};

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
fn drawn_pane(ed: &Editor, buf: &Grid, pid: PaneId) -> (Rect, u16) {
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
fn pane_rows(ed: &Editor, buf: &Grid, pid: PaneId, n: u16) -> Vec<String> {
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

    let buf = wheel_down(&mut ed, pid, 10);
    assert_eq!(top_line(&ed, pid), 10);
    assert_eq!(
        top_rows(&ed, &buf, 3),
        [
            line_of(&source, 0),
            line_of(&source, 1),
            line_of(&source, 12)
        ]
    );

    // The first fn's `}` (line 32) is the line at row 1.
    let buf = wheel_down(&mut ed, pid, 21);
    assert_eq!(top_line(&ed, pid), 31);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 32)]
    );

    // The blank line after it is at row 1: still only the `impl` header.
    let buf = wheel_down(&mut ed, pid, 1);
    assert_eq!(top_line(&ed, pid), 32);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 33)]
    );
}

#[test]
fn short_pane_keeps_two_headers_by_the_outer_level_setting() {
    let mut ed = markdown_context_editor(&nested_markdown(3));
    seek_to_line(&mut ed, 25);
    let rect = Rect::new(0, 0, 20, 6);

    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 2), ["## b", "### c"]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 2), ["# a", "### c"]);
}

#[test]
fn tall_pane_keeps_three_headers_by_the_outer_level_setting() {
    let mut ed = markdown_context_editor(&nested_markdown(4));
    seek_to_line(&mut ed, 40);

    let buf = ed.render_to_buf(VIEW);
    assert_eq!(top_rows(&ed, &buf, 3), ["## b", "### c", "#### d"]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(top_rows(&ed, &buf, 3), ["# a", "### c", "#### d"]);
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

    let buf = ed.render_to_buf(VIEW);
    assert_eq!(pane_rows(&ed, &buf, first, 3), ["## b", "### c", "#### d"]);
    assert_eq!(pane_rows(&ed, &buf, second, 3), ["# a", "### c", "#### d"]);
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
            .raw_contains(hume_engine::theme::ui_scopes::CONTEXT),
        "premise: gruvbox leaves ui.context unset"
    );

    let (rect, gutter) = drawn_pane(&ed, &buf, pid);
    assert!(gutter > 0, "premise: the split pane draws a gutter");
    let source = wide_impl(false);
    assert_eq!(
        pane_rows(&ed, &buf, pid, 2),
        [
            line_of(&source, 0)[..usize::from(rect.width - gutter)].to_owned(),
            line_of(&source, 1)
        ]
    );
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
/// Line 5 is a body row and shows each decoration.
#[test]
fn header_rows_draw_their_lines_without_decorations() {
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
    let header_bg = ed.view.theme.ui.context.bg;
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
                text: "H:".to_owned(),
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
                text: " EOL".to_owned(),
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
    let body = pane_rows(&ed, &buf, pid, 5).remove(4);
    assert_eq!(
        (body.as_str(), gutter_text(4).contains('!')),
        ("    H:    let x3 = 3; EOL", true),
        "premise: a body row draws its line's hint, EOL text and sign"
    );
    assert_eq!(
        pane_rows(&ed, &buf, pid, 2),
        [line_of(&source, 0), line_of(&source, 1)]
    );
    for row in 0..2 {
        assert!(!gutter_text(row).contains('!'), "header row {row}'s gutter");
        for x in rect.x + gutter..rect.right() {
            assert_eq!(
                buf[(x, rect.y + row)].style().bg,
                header_bg,
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
    let buf = wheel_down(&mut ed, second, 10);
    assert_eq!(top_line(&ed, first), 33);
    assert_eq!(top_line(&ed, second), 10);

    assert_eq!(
        pane_rows(&ed, &buf, first, 2),
        [line_of(&source, 0), line_of(&source, 34)]
    );
    assert_eq!(
        pane_rows(&ed, &buf, second, 3),
        [
            line_of(&source, 0),
            line_of(&source, 1),
            line_of(&source, 12)
        ]
    );
}

#[test]
fn context_enabled_off_on_one_buffer_only() {
    let (mut ed, first, second, source) = two_markdown_panes("buffer context-enabled=false");

    let buf = ed.render_to_buf(VIEW);
    assert_eq!(pane_rows(&ed, &buf, first, 1), ["## b"]);
    assert_eq!(
        pane_rows(&ed, &buf, second, 1),
        [line_of(&source, top_line(&ed, second))]
    );
}

#[test]
fn defaults_show_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    let buf = wheel_down(&mut ed, pid, 10);

    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 1)]
    );
    assert_eq!(
        log_probe(
            &mut ed,
            tmp.path(),
            r#"(if (get-buffer-option bid "context-outer-level") "true" "false")"#
        ),
        "false"
    );
}

#[test]
fn pane_of_height_one_shows_no_header() {
    let source = nested_markdown(3);
    let mut ed = markdown_context_editor(&source);
    seek_to_line(&mut ed, 25);
    let buf = ed.render_to_buf(Rect::new(0, 0, 20, 2));
    let pid = ed.view.focused_pane();
    assert_eq!(
        top_rows(&ed, &buf, 1),
        [line_of(&source, top_line(&ed, pid))]
    );
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
        pane_rows(&ed, &buf, pid, 2),
        [line_of(&source, 0), line_of(&source, 1)],
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
fn cursor_row_and_headers(ed: &mut Editor, source: &str, view: Rect) -> (u16, String, usize) {
    let (cursor_x, row) = frame(ed, view.width, view.height)
        .cursor_content_pos
        .expect("the cursor is on screen");
    let buf = ed.render_to_buf(view);
    let pid = ed.view.focused_pane();
    let (rect, gutter) = drawn_pane(ed, &buf, pid);
    let shown = cell(&buf, rect.x + gutter + cursor_x, rect.y + row);
    let above: Vec<String> = (0..top_line(ed, pid))
        .map(|line| line_of(source, line))
        .collect();
    let headers = top_rows(ed, &buf, 3)
        .iter()
        .take_while(|text| above.contains(text))
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

    let (row, shown, headers) = cursor_row_and_headers(&mut ed, &source, VIEW);
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
        let (row, shown, headers) = cursor_row_and_headers(&mut ed, &source, VIEW);
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
        let (row, shown, headers) =
            cursor_row_and_headers(&mut ed, &source, Rect::new(0, 0, 80, 4));
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
        let with_row = cursor_row_and_headers(&mut with_grammar, &source, VIEW).0;
        let without_row = cursor_row_and_headers(&mut without, &source, VIEW).0;
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
    assert_eq!(
        top_rows(&ed, &buf, 3),
        [
            line_of(&source, 0),
            line_of(&source, 1),
            line_of(&source, 12)
        ]
    );

    press_at(&mut ed, &buf, pid, 4, 1);

    let sel = ed.current_view().primary();
    assert_eq!(sel.anchor().offset(), char_at_column(&ed, 1, 4));
    assert_eq!(sel.head().offset(), char_at_column(&ed, 1, 4));
    let pressed = state(&ed);
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(top_line(&ed, pid), 0);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 1)]
    );

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), pressed);
}

#[test]
fn press_on_a_header_counts_columns_from_zero_under_horizontal_scroll() {
    let mut ed = snapshot_editor(true, 12);
    pin_no_wrap(&mut ed);
    seek_to_column(&mut ed, 12, 150);
    let buf = ed.render_to_buf(SNAP_VIEW);
    let pid = ed.view.focused_pane();
    assert!(ed.pane_viewport(pid).horizontal_offset().get() > 0);
    assert!(top_rows(&ed, &buf, 1)[0].starts_with("impl"));

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
    assert_eq!(top_rows(&ed, &buf, 1), ["impl Iterator for AVeryLo"]);

    // Column 24 is past where the line's first wrap segment breaks.
    press_at(&mut ed, &buf, pid, 24, 0);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 0, 24)
    );
}

#[test]
fn drag_onto_a_header_row_extends_to_its_line() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let pid = ed.view.focused_pane();
    let buf = wheel_down(&mut ed, pid, 10);
    press_at(&mut ed, &buf, pid, 12, 5);
    let anchor = ed.current_view().primary().anchor().offset();
    let buf = ed.render_to_buf(VIEW);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 1)]
    );

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
    assert_eq!(top_rows(&ed, &buf, 1), [line_of(&source, 10)]);

    press_at(&mut ed, &buf, pid, 0, 0);

    assert_eq!(
        ed.current_view().primary().head().offset(),
        char_at_column(&ed, 10, 0)
    );
}
