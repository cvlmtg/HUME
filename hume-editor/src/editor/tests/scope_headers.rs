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

/// An editor on `source` with the `rust` grammar and its `context.scm`, the
/// initial parse drained. `source` is plain text; the cursor starts on its
/// first cluster. Indent guides are off, so a row reads as its line's text.
fn rust_context_editor(source: &str) -> Editor {
    let marked = format!("-[{}]>{}", &source[..1], &source[1..]);
    let mut ed = super::structural::grammar_editor_undrained(
        "rust",
        "tree_sitter_rust",
        &[OptionalQuery::Context],
        &marked,
    );
    ed.state.settings.indent_guides = false;
    ed.reparse_stale_buffers();
    ed
}

/// An editor on `source` as a markdown buffer with markdown's injections and
/// `context.scm` and rust's `context.scm`, the initial parse drained.
fn markdown_context_editor(source: &str) -> (Editor, BufferId) {
    require_grammars(&["markdown", "markdown.inline", "rust"]);
    require_helix_query("markdown", OptionalQuery::Context.file_name());
    require_helix_query("rust", OptionalQuery::Context.file_name());
    super::injections_editor::markdown_editor_with_rust_queries(
        source,
        &Dirs::none(),
        &[],
        &[OptionalQuery::Injections, OptionalQuery::Context],
        &[OptionalQuery::Context],
    )
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

/// Turn the wheel `ticks` notches down over pane `pid`, one line a notch.
fn wheel_down(ed: &mut Editor, buf: &Grid, pid: PaneId, ticks: usize) {
    let (rect, _) = drawn_pane(ed, buf, pid);
    ed.state.settings.mouse_scroll_lines = 1;
    for _ in 0..ticks {
        ed.handle_input(mouse_wheel_at(rect.x, rect.y, true));
    }
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
    let rect = Rect::new(0, 0, 80, 25);
    let buf = ed.render_to_buf(rect);

    wheel_down(&mut ed, &buf, pid, 10);
    let buf = ed.render_to_buf(rect);
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
    wheel_down(&mut ed, &buf, pid, 21);
    let buf = ed.render_to_buf(rect);
    assert_eq!(top_line(&ed, pid), 31);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 32)]
    );

    // The blank line after it is at row 1: still only the `impl` header.
    wheel_down(&mut ed, &buf, pid, 1);
    let buf = ed.render_to_buf(rect);
    assert_eq!(top_line(&ed, pid), 32);
    assert_eq!(
        top_rows(&ed, &buf, 2),
        [line_of(&source, 0), line_of(&source, 33)]
    );
}

#[test]
fn short_pane_keeps_two_headers_by_the_outer_level_setting() {
    let (mut ed, _) = markdown_context_editor(&nested_markdown(3));
    seek_to_line(&mut ed, 25);
    let rect = Rect::new(0, 0, 20, 4);

    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 2), ["## b", "### c"]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 2), ["# a", "### c"]);
}

#[test]
fn tall_pane_keeps_three_headers_by_the_outer_level_setting() {
    let (mut ed, _) = markdown_context_editor(&nested_markdown(4));
    seek_to_line(&mut ed, 40);
    let rect = Rect::new(0, 0, 80, 25);

    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 3), ["## b", "### c", "#### d"]);

    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");
    let buf = ed.render_to_buf(rect);
    assert_eq!(top_rows(&ed, &buf, 3), ["# a", "### c", "#### d"]);
}

#[test]
fn outer_level_is_per_buffer() {
    let source = nested_markdown(4);
    let (mut ed, _) = markdown_context_editor(&source);
    let first = ed.view.focused_pane();
    seek_to_line(&mut ed, 40);
    let second = split_markdown_buffer(&mut ed, &source);
    seek_to_line(&mut ed, 40);
    run_set(&mut ed, "buffer context-outer-level=true").expect("the setting applies");

    let buf = ed.render_to_buf(Rect::new(0, 0, 80, 25));
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
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 30, 16));
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_snapshot_wrap_on() {
    let mut ed = snapshot_editor(false, 15);
    run_set(&mut ed, "buffer gutter-line-numbers=absolute").expect("the setting applies");
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 30, 16));
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_ignore_horizontal_scroll() {
    let mut ed = snapshot_editor(true, 12);
    pin_no_wrap(&mut ed);
    let line_start = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(12),
    )
    .index();
    set_cursor(&mut ed, line_start + 150);
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 30, 16));
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_show_no_selection_cursor_or_cursorline() {
    let mut ed = snapshot_editor(false, 15);
    let impl_line_len = line_of(&wide_impl(false), 0).chars().count();
    let deep = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(15),
    )
    .index();
    select(&mut ed, &[(0, impl_line_len - 1), (deep, deep)], 1);
    let covered: Vec<String> = ed
        .current_view()
        .iter()
        .map(|sel| sel.slice().to_string())
        .collect();
    assert_eq!(
        covered,
        [line_of(&wide_impl(false), 0), " ".to_owned()],
        "premise: a secondary selection covers the impl line"
    );
    let snap = super::render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 30, 16));
    insta::assert_snapshot!(snap);
}

#[test]
fn header_rows_use_popup_background_when_ui_context_is_unset() {
    let mut ed = snapshot_editor(false, 15);
    pin_no_wrap(&mut ed);
    let pid = ed.view.focused_pane();
    let buf = ed.render_to_buf(Rect::new(0, 0, 30, 16));
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
    assert_eq!(
        pane_rows(&ed, &buf, pid, 2),
        [
            line_of(&wide_impl(false), 0)[..usize::from(rect.width - gutter)].to_owned(),
            line_of(&wide_impl(false), 1)
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

#[test]
fn two_panes_on_one_buffer_show_their_own_headers() {
    let source = two_fn_impl();
    let mut ed = rust_context_editor(&source);
    let first = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let second = ed.split_onto(bid, Direction::Horizontal);
    let rect = Rect::new(0, 0, 80, 25);
    let buf = ed.render_to_buf(rect);

    wheel_down(&mut ed, &buf, first, 33);
    wheel_down(&mut ed, &buf, second, 10);
    let buf = ed.render_to_buf(rect);
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
    let source = nested_markdown(4);
    let (mut ed, _) = markdown_context_editor(&source);
    let first = ed.view.focused_pane();
    seek_to_line(&mut ed, 40);
    let second = split_markdown_buffer(&mut ed, &source);
    seek_to_line(&mut ed, 40);
    run_set(&mut ed, "buffer context-enabled=false").expect("the setting applies");

    let buf = ed.render_to_buf(Rect::new(0, 0, 80, 25));
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
    let rect = Rect::new(0, 0, 80, 25);
    let buf = ed.render_to_buf(rect);
    wheel_down(&mut ed, &buf, pid, 10);

    let buf = ed.render_to_buf(rect);
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
    let (mut ed, _) = markdown_context_editor(&source);
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
    let rect = Rect::new(0, 0, 80, 25);
    let buf = ed.render_to_buf(rect);
    wheel_down(&mut ed, &buf, pid, 3);
    let buf = ed.render_to_buf(rect);

    let fn_line_start = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(1),
    );
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
