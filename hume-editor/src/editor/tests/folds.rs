//! Closed folds: what they hide, how they follow edits, and the rule that
//! no selection ends out of sight.

use super::*;
use crate::editor::settings::{SettingText, SignColumnConfig};
use hume_engine::folds::Fold;
use hume_rope::line::ContentLine;
use pretty_assertions::assert_eq;

/// A function whose body (lines 1 and 2) a closed fold hides, its closing
/// brace staying shown, then two more lines.
const FN: &str = "fn f() {\n    abc\n    d\n}\nz\ny\n";

/// The region of [`FN`]'s function.
fn fn_fold() -> Fold {
    fold(0, 3, true)
}

fn fold(first: usize, last: usize, shows_last: bool) -> Fold {
    Fold::new(ContentLine::new(first), ContentLine::new(last), shows_last)
        .expect("the region hides a line")
}

/// An editor on `input`, laid out without wrapping.
fn editor(input: &str) -> Editor {
    let mut ed = editor_from(input);
    pin_no_wrap(&mut ed);
    ed
}

/// Close `folds` in pane `pid`.
fn close_in(ed: &mut Editor, pid: PaneId, folds: &[Fold]) {
    let text = ed
        .state
        .buffers
        .get(ed.view.panes[pid].buffer_id())
        .text()
        .clone();
    ed.state
        .panes
        .entries
        .shown_mut(&ed.view.panes, pid)
        .close_folds(folds.iter().copied(), &text);
}

/// Close `folds` in the focused pane.
fn close(ed: &mut Editor, folds: &[Fold]) {
    close_in(ed, ed.view.focused_pane(), folds);
}

/// The closed folds of pane `pid`, as `(first, last)` lines.
fn closed_in(ed: &Editor, pid: PaneId) -> Vec<(usize, usize)> {
    ed.state
        .panes
        .entries
        .shown(&ed.view.panes, pid)
        .closed_folds()
        .iter()
        .map(|f| (f.first().index(), f.last().index()))
        .collect()
}

/// The closed folds of the focused pane.
fn closed(ed: &Editor) -> Vec<(usize, usize)> {
    closed_in(ed, ed.view.focused_pane())
}

// Closing

#[test]
fn closing_moves_a_selection_inside_to_the_fold_line() {
    let mut ed = editor("fn f() {\n    a-[b]>c\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    assert_eq!(state(&ed), "-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    assert_eq!(closed(&ed), [(0, 3)]);
}

#[test]
fn closing_moves_only_the_end_inside() {
    let mut ed = editor("-[a\nb {\n    c]>\n    d\n}\n");
    close(&mut ed, &[fold(1, 4, true)]);
    assert_eq!(state(&ed), "-[a\nb]> {\n    c\n    d\n}\n");
}

#[test]
fn closing_nested_folds_moves_an_end_to_the_outer_line() {
    let input = "a {\n    b {\n        c\n        d\n    }\n}\nz\n";
    let mut ed = editor(&input.replace("        c", "        -[c]>"));
    close(&mut ed, &[fold(0, 5, true), fold(1, 4, true)]);
    assert_eq!(state(&ed), input.replacen("a", "-[a]>", 1));
    assert_eq!(closed(&ed), [(0, 5), (1, 4)]);
}

#[test]
fn a_selection_covering_the_hidden_lines_keeps_the_fold_closed() {
    let input = "fn f() {\n-[    abc\n    d\n]>}\nz\ny\n";
    let mut ed = editor(input);
    close(&mut ed, &[fn_fold()]);
    assert_eq!(state(&ed), input);
    assert_eq!(closed(&ed), [(0, 3)]);
}

// A selection landing on a hidden line

#[test]
fn a_goto_into_a_fold_opens_it() {
    let mut ed = editor("fn f() {\n    abc\n    d\n}\n-[z]>\ny\n");
    close(&mut ed, &[fn_fold()]);
    type_cmd(&mut ed, ":goto 3");
    assert_eq!(state(&ed), "fn f() {\n    abc\n-[ ]>   d\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

#[test]
fn a_search_match_inside_a_fold_opens_it() {
    let mut ed = editor("-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "/b");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "fn f() {\n    a-[b]>c\n    d\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

#[test]
fn a_plain_line_step_crosses_a_closed_fold() {
    let mut ed = editor("-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "j");
    assert_eq!(state(&ed), "fn f() {\n    abc\n    d\n-[}]>\nz\ny\n");
    assert_eq!(closed(&ed), [(0, 3)]);
}

#[test]
fn a_counted_line_step_into_a_fold_opens_it() {
    let mut ed = editor("-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "2j");
    assert_eq!(state(&ed), "fn f() {\n    abc\n-[ ]>   d\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

#[test]
fn copying_a_selection_onto_a_hidden_line_opens_its_fold() {
    let mut ed = editor("-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "C");
    assert_eq!(state(&ed), "-[f]>n f() {\n-{ }>   abc\n    d\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

#[test]
fn jumping_back_into_a_fold_opens_it() {
    let mut ed = editor("fn f() {\n    abc\n    -[d]>\n}\nz\ny\n");
    type_cmd(&mut ed, ":goto 5");
    close(&mut ed, &[fn_fold()]);
    ed.feed_key(key_ctrl('o'));
    assert_eq!(state(&ed), "fn f() {\n    abc\n    -[d]>\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

#[test]
fn an_undo_landing_inside_a_fold_opens_it() {
    let mut ed = editor("fn f() {\n    abc\n    -[d]>\n}\nz\ny\n");
    keys(&mut ed, "rX");
    type_cmd(&mut ed, ":goto 5");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "u");
    assert_eq!(state(&ed), "fn f() {\n    abc\n    -[d]>\n}\nz\ny\n");
    assert_eq!(closed(&ed), []);
}

// Following edits

/// [`FN`] with the cursor on `z`, its function closed in the first pane,
/// and a second pane on the same buffer, focused, with no fold.
fn split_with_fold_in_first() -> (Editor, PaneId) {
    let mut ed = editor("fn f() {\n    abc\n    d\n}\n-[z]>\ny\n");
    close(&mut ed, &[fn_fold()]);
    let first = ed.view.focused_pane();
    ed.execute_typed("split", None).expect("the split fits");
    ed.state
        .panes
        .entries
        .shown_mut(&ed.view.panes, ed.view.focused_pane())
        .open_all_folds();
    (ed, first)
}

#[test]
fn a_line_added_inside_a_fold_from_another_pane_stays_hidden() {
    let (mut ed, first) = split_with_fold_in_first();
    seek_to_line(&mut ed, 1);
    keys(&mut ed, "o");
    ed.feed_key(key_esc());
    assert_eq!(closed_in(&ed, first), [(0, 4)]);
}

#[test]
fn deleting_the_fold_line_from_another_pane_drops_the_fold() {
    let (mut ed, first) = split_with_fold_in_first();
    seek_to_line(&mut ed, 0);
    keys(&mut ed, "xd");
    assert_eq!(closed_in(&ed, first), []);
}

#[test]
fn joining_the_fold_line_from_another_pane_drops_the_fold() {
    let (mut ed, first) = split_with_fold_in_first();
    seek_to_line(&mut ed, 0);
    keys(&mut ed, "J");
    assert_eq!(closed_in(&ed, first), []);
}

#[test]
fn an_edit_below_a_fold_leaves_it() {
    let (mut ed, first) = split_with_fold_in_first();
    seek_to_line(&mut ed, 5);
    keys(&mut ed, "xd");
    assert_eq!(closed_in(&ed, first), [(0, 3)]);
}

#[test]
fn opening_a_line_below_the_fold_line_opens_the_fold() {
    let mut ed = editor("-[f]>n f() {\n    abc\n    d\n}\nz\ny\n");
    close(&mut ed, &[fn_fold()]);
    keys(&mut ed, "o");
    ed.feed_key(key_esc());
    assert_eq!(closed(&ed), []);
}

#[test]
fn a_reload_carries_the_folds_through_its_changes() {
    let mut ed = editor("fn f() {\n    abc\n    d\n}\n-[z]>\ny\n");
    close(&mut ed, &[fn_fold()]);
    reload_from_dump(&mut ed, &format!("// new\n{FN}"));
    assert_eq!(closed(&ed), [(1, 4)]);
}

#[test]
fn a_split_keeps_the_panes_folds() {
    let mut ed = editor("fn f() {\n    abc\n    d\n}\n-[z]>\ny\n");
    close(&mut ed, &[fn_fold()]);
    ed.execute_typed("split", None).expect("the split fits");
    assert_eq!(closed(&ed), [(0, 3)]);
}

#[test]
fn the_visible_range_reaches_past_a_fold() {
    let body = "    a\n".repeat(30);
    let tail = "z\n".repeat(10);
    let mut ed = editor(&format!("-[f]>n f() {{\n{body}}}\n{tail}"));
    close(&mut ed, &[fold(0, 31, true)]);
    let pid = ed.view.focused_pane();
    ed.resize_terminal(40, 8);
    ed.settle();
    let range = crate::editor::geometry::viewport_range(&mut ed.state, &mut ed.view, pid);
    assert!(
        range.end.index() > 31,
        "lines after the fold are in range: {range:?}"
    );
}

#[test]
fn an_auto_sign_column_stays_closed_for_a_sign_a_fold_hides() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    type_text(&mut ed, "a\nb\nc\nd\ne");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.gutter_signs =
        Some(SignColumnConfig::parse("auto").expect("a valid setting"));
    let source = r#"(define-typed-command! "arm" "" (lambda (bid)
        (register-sign-source! bid "linter" 3)
        (set-signs! bid "linter" (list (hash 'line 2 'text "!" 'scope "a")))))"#;
    run(&mut ed, tmp.path(), source);
    type_cmd(&mut ed, ":arm");
    close(&mut ed, &[fold(0, 3, false)]);
    render(&mut ed);
    let pid = ed.view.focused_pane();
    assert_eq!(ed.state.gutter_inputs(&ed.view, pid).sign_width, 0);
}

// Drawing

/// The frame the fold snapshots draw: the bare first pane over the split
/// pane with a gutter.
const SNAP: hume_grid::Rect = hume_grid::Rect::new(0, 0, 30, 18);

/// Two functions, the second holding an `if`.
const RS: &str = "fn a() {\n    let x = 1;\n    let y = 2;\n}\n\nfn b(v: u32) {\n    if v > 0 {\n        go();\n        stop();\n    }\n}\n";

/// An editor on `source` in `name`'s grammar with its `context.scm`, the
/// parse drained, focused on a split pane on the same buffer, which draws a
/// gutter, numbered absolute, with the snapshot theme and no indent guides.
fn drawn_editor(name: &str, symbol: &str, source: &str) -> Editor {
    let marked = format!("-[{}]>{}", &source[..1], &source[1..]);
    let mut ed = super::structural::grammar_editor_undrained(
        name,
        symbol,
        &[hume_core::OptionalQuery::Context],
        &marked,
    );
    ed.reparse_stale_buffers();
    ed.state.settings.indent_guides = false;
    ed.view.theme = crate::testing::build_snapshot_theme();
    let bid = ed.focused_buffer_id();
    let pid = ed.split_onto(bid, hume_engine::pipeline::Direction::Vertical);
    ed.state.focus.set_for_test(&mut ed.view, pid);
    run_set(&mut ed, "buffer gutter-line-numbers=absolute").expect("the setting applies");
    ed
}

fn rust_drawn(source: &str) -> Editor {
    drawn_editor("rust", "tree_sitter_rust", source)
}

/// Close the region the grammar starts at `line` in the focused pane.
fn close_at(ed: &mut Editor, line: usize) {
    let doc = ed.doc();
    let mut folds = Vec::new();
    doc.current_layers()
        .expect("the buffer is parsed")
        .all_folds(doc.text().rope(), &mut folds);
    let fold = folds
        .into_iter()
        .find(|fold| fold.first() == ContentLine::new(line))
        .expect("a region starts on the line");
    close(ed, &[fold]);
}

fn snapshot(ed: &mut Editor) -> String {
    super::render_snapshot::render_to_styled_string(ed, SNAP)
}

#[test]
fn carets_mark_each_line_a_region_starts_on() {
    let mut ed = rust_drawn(RS);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn a_closed_fold_shows_its_first_and_closing_lines() {
    let mut ed = rust_drawn(RS);
    close_at(&mut ed, 0);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn a_closed_fold_ending_in_content_shows_only_its_first_line() {
    let source = "def a():\n    x = 1\n    y = 2\n\ndef b():\n    z = 3\n";
    let mut ed = drawn_editor("python", "tree_sitter_python", source);
    close_at(&mut ed, 0);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn an_outer_fold_closed_over_an_inner_one_shows_one_caret() {
    let mut ed = rust_drawn(RS);
    close_at(&mut ed, 6);
    close_at(&mut ed, 5);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn a_wrapped_fold_line_draws_its_caret_once() {
    let source = "fn long_name(first: u32, second: u32) {\n    a();\n    b();\n}\n";
    let mut ed = rust_drawn(source);
    pin_wrap(&mut ed, hume_engine::pane::WrapMode::Soft { width: 0 });
    close_at(&mut ed, 0);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn a_header_row_draws_its_lines_caret() {
    let body: String = (0..20).map(|i| format!("    let v{i} = {i};\n")).collect();
    let mut ed = rust_drawn(&format!("fn a() {{\n{body}}}\n"));
    seek_to_line(&mut ed, 15);
    insta::assert_snapshot!(snapshot(&mut ed));
}

#[test]
fn gutter_folds_off_draws_no_caret() {
    let mut ed = rust_drawn(RS);
    run_set(&mut ed, "buffer gutter-folds=false").expect("the setting applies");
    insta::assert_snapshot!(snapshot(&mut ed));
}

// Commands

/// An editor on `marked` in the `rust` grammar with its `context.scm`, the
/// parse drained, laid out without wrapping.
fn rust_editor(marked: &str) -> Editor {
    let mut ed = super::structural::grammar_editor_undrained(
        "rust",
        "tree_sitter_rust",
        &[hume_core::OptionalQuery::Context],
        marked,
    );
    pin_no_wrap(&mut ed);
    ed.reparse_stale_buffers();
    ed
}

/// [`RS`] with a cursor on `x` and the primary one on `go`.
const RS_TWO_CURSORS: &str = "fn a() {\n    let -[x]> = 1;\n    let y = 2;\n}\n\nfn b(v: u32) {\n    if v > 0 {\n        -{g}>o();\n        stop();\n    }\n}\n";

#[test]
fn close_fold_closes_the_innermost_region_at_each_selection() {
    let mut ed = rust_editor(RS_TWO_CURSORS);
    keys(&mut ed, "zc");
    assert_eq!(closed(&ed), [(0, 3), (6, 9)]);
    assert_eq!(
        state(&ed),
        "-[f]>n a() {\n    let x = 1;\n    let y = 2;\n}\n\nfn b(v: u32) {\n    -{i}>f v > 0 {\n        go();\n        stop();\n    }\n}\n"
    );
}

#[test]
fn close_fold_on_a_closed_fold_line_closes_the_region_around_it() {
    let mut ed = rust_editor(&RS.replacen("go", "-[g]>o", 1));
    keys(&mut ed, "zczc");
    assert_eq!(closed(&ed), [(5, 10), (6, 9)]);
}

#[test]
fn open_fold_leaves_the_folds_inside_closed() {
    let mut ed = rust_editor(&RS.replacen("fn b", "-[f]>n b", 1));
    keys(&mut ed, "zC");
    assert_eq!(closed(&ed), [(0, 3), (5, 10), (6, 9)]);
    keys(&mut ed, "zo");
    assert_eq!(closed(&ed), [(0, 3), (6, 9)]);
}

#[test]
fn toggle_fold_closes_then_opens() {
    let mut ed = rust_editor(&RS.replacen("x", "-[x]>", 1));
    keys(&mut ed, "zt");
    assert_eq!(closed(&ed), [(0, 3)]);
    keys(&mut ed, "zt");
    assert_eq!(closed(&ed), []);
}

#[test]
fn open_all_folds_opens_every_fold() {
    let mut ed = rust_editor(&RS.replacen("x", "-[x]>", 1));
    keys(&mut ed, "zC");
    assert_eq!(closed(&ed), [(0, 3), (5, 10), (6, 9)]);
    keys(&mut ed, "zO");
    assert_eq!(closed(&ed), []);
}

#[test]
fn closing_without_a_parsed_tree_changes_nothing() {
    let marked = RS.replacen("x", "-[x]>", 1);
    let mut ed = super::structural::grammar_editor_undrained(
        "rust",
        "tree_sitter_rust",
        &[hume_core::OptionalQuery::Context],
        &marked,
    );
    keys(&mut ed, "zc");
    assert_eq!(closed(&ed), []);
    assert_eq!(state(&ed), marked);
    assert_eq!(status(&ed), "No syntax tree to fold with");
}

#[test]
fn a_folds_query_wins_over_the_scope_query() {
    let tmp = tempfile::tempdir().unwrap();
    let folds = tmp.path().join("folds.scm");
    std::fs::write(&folds, "(array_expression) @fold\n").unwrap();
    let files = QueryFiles::new(test_fixtures::grammar_query_path("rust"))
        .with(
            hume_core::OptionalQuery::Context,
            test_fixtures::require_helix_query("rust", "context.scm"),
        )
        .with(hume_core::OptionalQuery::Folds, folds);
    test_fixtures::require_grammars(&["rust"]);
    let mut ed = editor_from("fn f() {\n    let v = [\n        -[1]>,\n    ];\n}\n");
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar_with(&mut ed, "rust", "tree_sitter_rust", &files);
    super::language::attach_host(&mut ed, "");
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang)).expect_open();
    ed.reparse_stale_buffers();
    keys(&mut ed, "zc");
    assert_eq!(closed(&ed), [(1, 3)]);
}

#[test]
fn closing_the_scope_of_a_sticky_header_settles_on_its_line() {
    let body: String = (0..20).map(|i| format!("    let v{i} = {i};\n")).collect();
    let mut ed = rust_editor(&format!("-[f]>n a() {{\n{body}}}\n"));
    let pid = ed.view.focused_pane();
    seek_to_line(&mut ed, 15);
    frame(&mut ed, 40, 8);
    assert!(
        ed.pane_viewport(pid).top().line.index() > 0,
        "premise: line 0 is off screen"
    );
    keys(&mut ed, "zc");
    frame(&mut ed, 40, 8);
    assert_eq!(state(&ed), format!("-[f]>n a() {{\n{body}}}\n"));
    assert_eq!(ed.pane_viewport(pid).top().line.index(), 0);
}

// Clicks

/// Where pane `pid` was drawn in `buf`, and its gutter's width.
fn drawn_pane(ed: &Editor, buf: &hume_grid::Grid, pid: PaneId) -> (hume_grid::Rect, u16) {
    let (width, height) = buf.size();
    let (origin, size) = (0..height)
        .flat_map(|y| (0..width).map(move |x| hume_grid::Position::new(x, y)))
        .find_map(|pos| {
            ed.view
                .pane_hit(pos)
                .filter(|hit| hit.pane() == pid && hit.x() == 0 && hit.y() == 0)
                .map(|hit| (pos, hit.size()))
        })
        .expect("the pane was drawn");
    let gutter = ed.state.pane_geometry(&ed.view, pid, size).gutter_width;
    (
        hume_grid::Rect::new(origin.x, origin.y, size.width(), size.height()),
        gutter,
    )
}

/// Press on the fold column's caret cell of the focused pane's row `row`.
fn press_fold_lane(ed: &mut Editor, row: u16) {
    let buf = ed.render_to_buf(SNAP);
    let (rect, gutter) = drawn_pane(ed, &buf, ed.view.focused_pane());
    ed.handle_input(mouse_left_down(rect.x + gutter - 2, rect.y + row));
}

#[test]
fn a_press_on_a_caret_toggles_its_fold() {
    let mut ed = rust_drawn(RS);
    press_fold_lane(&mut ed, 0);
    assert_eq!(closed(&ed), [(0, 3)]);
    press_fold_lane(&mut ed, 0);
    assert_eq!(closed(&ed), []);
}

#[test]
fn a_press_on_a_header_rows_caret_toggles_its_fold() {
    let body: String = (0..20).map(|i| format!("    let v{i} = {i};\n")).collect();
    let mut ed = rust_drawn(&format!("fn a() {{\n{body}}}\n"));
    seek_to_line(&mut ed, 15);
    press_fold_lane(&mut ed, 0);
    assert_eq!(closed(&ed), [(0, 21)]);
}

#[test]
fn a_press_on_the_fold_lane_beside_no_caret_only_focuses() {
    let mut ed = rust_drawn(RS);
    let before = state(&ed);
    press_fold_lane(&mut ed, 1);
    assert_eq!(closed(&ed), []);
    assert_eq!(state(&ed), before);
}
