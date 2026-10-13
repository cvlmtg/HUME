//! The menu's geometry (`sync_completion_menu_view`/
//! `sync_minibuf_completion_view` through `hume_ui::popup::menu_window` +
//! `resolve_menu`), the documentation box beside it, and the three
//! full-frame snapshots.

use super::*;
use crate::editor::{commands, cursor};
use hume_engine::pane::WrapMode;
use hume_grid::Rect;

// ── Pane-fit clamp: the menu renders clamped, never vanishes ────────────────
//
// `PopupOverlay` paints the rect the write side placed, so the write side
// clamps the box into the pane.

#[test]
fn completion_menu_clamps_to_a_short_pane_instead_of_vanishing() {
    let mut ed = open_headless(None);
    ed.feed_key(key('i'));
    // MAX_MENU_ROWS is 10: 12 items would size an unclamped box to 12
    // rows (+2 frame), taller than the short pane below.
    let labels: Vec<String> = (0..12).map(|i| format!("item{i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    open_completion_session(&mut ed, &labels);

    frame(&mut ed, 40, 6);

    let pane_rect = ed.view.focused_rect();
    let view = ed.state.views.completion_menu.read();
    let state = view
        .as_ref()
        .expect("popup must render clamped, not vanish");
    assert!(state.rect.height <= pane_rect.height);
}

#[test]
fn completion_menu_clamps_to_a_narrow_pane_instead_of_vanishing() {
    let mut ed = open_headless(None);
    ed.feed_key(key('i'));
    open_completion_session(
        &mut ed,
        &["a_very_long_candidate_label_that_overflows_the_pane"],
    );

    frame(&mut ed, 20, 8);

    let pane_rect = ed.view.focused_rect();
    let view = ed.state.views.completion_menu.read();
    let state = view
        .as_ref()
        .expect("popup must render clamped, not vanish");
    assert!(state.rect.width <= pane_rect.width);
}

// ── Snapshots ─────────────────────────────────────────────────────────────────

/// The Insert-mode menu with `completion-docs` off: two columns (`label`,
/// then `detail` right-aligned), sized to its visible rows.
#[test]
fn menu_appears_with_top_items_after_the_source_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state.settings.completion_docs = false;
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foo" "detail" "fn")
                 (hash "label" "foobar")
                 (hash "label" "food" "detail" "field"))"#,
    );

    frame(&mut ed, 40, 8);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

/// The `:` line's popup renders through the same generic menu path as every
/// other menu-shaped overlay, above the statusline.
#[test]
fn minibuf_completion_popup_renders_above_the_statusline() {
    // `Editor::open`, not `editor_from`: `Editor::for_testing` never goes
    // through `build_pane`, so no overlay providers are registered and the
    // popup would silently not paint.
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();

    // "w" matches exactly three canonical command names: write, write-all,
    // write-quit. `complete_command` omits aliases and the exact-prefix
    // match, so this candidate set is stable against new commands.
    ed.feed_key(key(':'));
    ed.feed_key(key('w'));
    ed.feed_key(key_tab());

    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 10));
    insta::assert_snapshot!(snap);
}

// ── Documentation box ────────────────────────────────────────────────────────

const DOCUMENTED: &str = r#"(list (hash "label" "foo" "detail" "fn foo()" "documentation" "Does foo.")
                                (hash "label" "foobar" "documentation" "Does foobar."))"#;

fn doc_box(ed: &Editor) -> Option<(Rect, Vec<String>)> {
    let view = ed.state.views.completion_doc.read();
    let state = view.as_ref()?;
    Some((state.rect, state.lines[state.visible.clone()].to_vec()))
}

fn menu_rect(ed: &Editor) -> Rect {
    ed.state
        .views
        .completion_menu
        .read()
        .as_ref()
        .expect("menu showing")
        .rect
}

/// The selected candidate's `detail`, then its documentation, in a box to
/// the right of the menu.
#[test]
fn doc_box_appears_right_of_the_menu_for_the_selected_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);

    frame(&mut ed, 60, 16);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 60, 16));
    insta::assert_snapshot!(snap);
}

#[test]
fn doc_box_follows_the_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);
    frame(&mut ed, 60, 16);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines, ["fn foo()", "", "Does foo."]);

    ed.feed_key(key_tab());
    frame(&mut ed, 60, 16);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines, ["Does foobar."]);
}

/// Going back to a candidate reuses the box built for it, markdown parse
/// included.
#[test]
fn doc_box_content_is_built_once_per_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);
    frame(&mut ed, 60, 16);
    for _ in 0..3 {
        ed.feed_key(key_tab());
        frame(&mut ed, 60, 16);
    }
    let builds = ed
        .state
        .input
        .find::<crate::editor::input_stack::BufferCompletionLayer>()
        .map(|layer| layer.doc_builds());
    assert_eq!(builds, Some(2));
}

/// The box's content bakes the theme's colors in, so a theme change builds
/// it again. Checked by object identity: every build allocates its own
/// wrapped lines, styled or plain.
#[test]
fn doc_box_content_is_rebuilt_after_a_theme_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.state.dirs = repo_runtime_dirs();
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);
    let doc_lines = |ed: &Editor| {
        ed.state
            .views
            .completion_doc
            .read()
            .as_ref()
            .map(|state| std::sync::Arc::clone(&state.lines))
    };
    frame(&mut ed, 60, 16);
    let first = doc_lines(&ed).expect("doc box showing");

    let fp = FocusedPane::current(&ed.view);
    commands::typed_theme(&mut ed, fp, Some("gruvbox"), false)
        .expect(":theme gruvbox must succeed");
    frame(&mut ed, 60, 16);
    let second = doc_lines(&ed).expect("doc box still showing");
    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn doc_box_flips_left_of_a_menu_near_the_right_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    run(
        &mut ed,
        tmp.path(),
        &completion_source("test", DOCUMENTED, ""),
    );
    ed.feed_key(key('i'));
    type_chars(&mut ed, &format!("{} ", "x".repeat(40)));
    trigger(&mut ed);

    frame(&mut ed, 60, 8);
    let (rect, _) = doc_box(&ed).expect("doc box showing");
    assert_eq!(rect.right(), menu_rect(&ed).x);
    assert_eq!(rect.y, menu_rect(&ed).y);
}

#[test]
fn no_doc_box_without_room_beside_the_menu() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);

    frame(&mut ed, 30, 8);
    assert!(
        ed.state.views.completion_menu.read().is_some(),
        "sanity: menu showing"
    );
    assert_eq!(doc_box(&ed), None);
}

/// With no room for the box, the menu keeps `detail` rather than drop it.
#[test]
fn menu_shows_detail_when_the_doc_box_has_no_room() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);

    frame(&mut ed, 30, 8);
    assert_eq!(doc_box(&ed), None);
    let menu = ed.state.views.completion_menu.read();
    let lines = &menu.as_ref().expect("menu showing").lines;
    assert!(
        lines.iter().any(|line| line.contains("fn foo()")),
        "menu lines: {lines:?}"
    );
}

/// With `completion-docs` on, `detail` shows in the box, never in the menu.
#[test]
fn menu_leaves_detail_to_the_doc_box() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);

    frame(&mut ed, 60, 16);
    let menu = ed.state.views.completion_menu.read();
    let lines = &menu.as_ref().expect("menu showing").lines;
    assert!(lines.iter().all(|line| !line.contains("fn foo()")));
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines, ["fn foo()", "", "Does foo."]);
}

#[test]
fn doc_box_shows_a_detail_without_documentation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foo" "detail" "fn foo()"))"#,
    );

    frame(&mut ed, 60, 8);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines, ["fn foo()"]);
}

#[test]
fn no_doc_box_for_a_candidate_without_detail_or_documentation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), r#"(list (hash "label" "foo"))"#);

    frame(&mut ed, 60, 8);
    assert!(
        ed.state.views.completion_menu.read().is_some(),
        "sanity: menu showing"
    );
    assert_eq!(doc_box(&ed), None);
}

#[test]
fn no_doc_box_with_completion_docs_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.state.settings.completion_docs = false;
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);

    frame(&mut ed, 60, 8);
    assert!(
        ed.state.views.completion_menu.read().is_some(),
        "sanity: menu showing"
    );
    assert_eq!(doc_box(&ed), None);
}

#[test]
fn doc_box_closes_with_the_menu() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);
    frame(&mut ed, 60, 8);
    assert!(doc_box(&ed).is_some(), "sanity: doc box showing");

    ed.feed_key(key_esc());
    frame(&mut ed, 60, 8);
    assert_eq!(doc_box(&ed), None);
}

/// Two candidates, the first with 20 one-word documentation lines
/// (`line0`..`line19`), more than the box shows in a 60x30 frame.
fn long_doc_items() -> String {
    let doc: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    format!(
        r#"(list (hash "label" "foo" "documentation" "{}") (hash "label" "foobar"))"#,
        doc.join("\\n")
    )
}

fn doc_box_inner_height(ed: &Editor) -> usize {
    ed.state
        .views
        .completion_doc
        .read()
        .as_ref()
        .expect("doc box showing")
        .inner_height()
}

#[test]
fn ctrl_d_and_ctrl_u_scroll_the_doc_box() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), &long_doc_items());
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines[0], "line0");
    let inner = doc_box_inner_height(&ed);
    assert!(inner < 20, "sanity: the box cuts the documentation off");
    let before = state(&ed);

    ed.feed_key(key_ctrl('d'));
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box still showing");
    assert_eq!(lines[0], format!("line{}", inner / 2));
    assert_eq!(selected_row(&ed), 0);
    assert_eq!(state(&ed), before);

    ed.feed_key(key_ctrl('u'));
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box still showing");
    assert_eq!(lines[0], "line0");
}

#[test]
fn ctrl_d_stops_the_doc_box_at_its_end() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), &long_doc_items());
    frame(&mut ed, 60, 30);

    for _ in 0..20 {
        ed.feed_key(key_ctrl('d'));
        frame(&mut ed, 60, 30);
    }
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines.last().map(String::as_str), Some("line19"));

    ed.feed_key(key_ctrl('u'));
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_ne!(lines.last().map(String::as_str), Some("line19"));
}

#[test]
fn doc_box_scroll_resets_when_the_selection_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), &long_doc_items());
    frame(&mut ed, 60, 30);
    ed.feed_key(key_ctrl('d'));
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_ne!(lines[0], "line0", "sanity: scrolled");

    ed.feed_key(key_tab());
    frame(&mut ed, 60, 30);
    ed.feed_key(key_shift_tab());
    frame(&mut ed, 60, 30);
    let (_, lines) = doc_box(&ed).expect("doc box showing");
    assert_eq!(lines[0], "line0");
}

#[test]
fn ctrl_d_on_a_doc_box_that_fits_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    insert_with_source(&mut ed, tmp.path(), DOCUMENTED);
    frame(&mut ed, 60, 16);
    let before = state(&ed);

    ed.feed_key(key_ctrl('d'));
    frame(&mut ed, 60, 16);
    let (_, lines) = doc_box(&ed).expect("doc box still showing");
    assert_eq!(lines, ["fn foo()", "", "Does foo."]);
    assert_eq!(selected_row(&ed), 0);
    assert_eq!(state(&ed), before);
}

/// The fence around `detail` names the buffer's language, so the markdown
/// grammar highlights it as that language's code.
#[test]
fn doc_box_fences_detail_in_the_buffers_language() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    let rust = ed
        .state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foo" "detail" "fn foo()"
                        "documentation" (hash "kind" "markdown" "value" "Does foo.")))"#,
    );
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_explicit(bid, Some(rust));

    frame(&mut ed, 60, 10);
    let view = ed.state.views.completion_doc.read();
    let state = view.as_ref().expect("doc box showing");
    assert_eq!(
        state.lines.as_slice(),
        ["```rust", "fn foo()", "```", "", "Does foo."]
    );
}

/// Markdown documentation is highlighted, its `detail` fenced above it.
#[test]
fn markdown_documentation_is_highlighted_below_a_fenced_detail() {
    test_fixtures::require_grammars(&["markdown"]);
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    register_markdown(&mut ed);
    insert_with_source(
        &mut ed,
        tmp.path(),
        r##"(list (hash "label" "foo" "detail" "fn foo()"
                        "documentation" (hash "kind" "markdown" "value" "# Foo")))"##,
    );

    frame(&mut ed, 60, 10);
    let view = ed.state.views.completion_doc.read();
    let state = view.as_ref().expect("doc box showing");
    assert_eq!(
        state.lines.as_slice(),
        ["```", "fn foo()", "```", "", "# Foo"]
    );
    assert!(state.styled_rows.is_some(), "markdown content is styled");
}

// ── The anchor reuses the scroll pass's cached cursor cell ──────────────────
//
// `popup_placement` takes a fast path when its anchor is the focused
// cursor: it reuses `ctx.cursor_content_pos` from `scroll_into_view` instead
// of re-walking the display-line list. Pins that the reused cell agrees
// with a full, independent walk, in wrap mode, where a wrong cache would
// show up as a silently-misplaced popup, not a panic.

#[test]
fn completion_popup_anchor_matches_an_independent_content_pos_walk_when_wrapped() {
    let mut ed = open_headless(None);
    let pid = ed.view.focused_pane();
    pin_wrap(&mut ed, WrapMode::Soft { width: 6 });

    ed.feed_key(key('i'));
    type_chars(&mut ed, "abcdefghijklmnopqrstuvwxyz0123456789");
    // A non-word char right before triggering: the word token is empty,
    // exactly at the cursor. The fast path this test pins is only taken
    // when the anchor and the cursor agree. Still many display lines into
    // the wrapped text.
    ed.feed_key(key(';'));
    open_completion_session(&mut ed, &["candidate"]);

    // Ample room on every side: no flip, no clamp, so the popup's (x, y) is
    // exactly (anchor_x, anchor_y + 1).
    frame(&mut ed, 80, 24);

    let (x, y) = {
        let view = ed.state.views.completion_menu.read();
        let state = view.as_ref().expect("popup must be showing");
        (state.rect.x, state.rect.y)
    };

    // Re-derive the cell independently via a fresh `DisplayLineMap`
    // and `cursor::content_pos`, bypassing `ctx.cursor_content_pos`.
    let bid = ed.focused_buffer_id();
    let anchor = ed
        .state
        .input
        .buffer_completion()
        .unwrap()
        .menu_anchor(ed.doc().text())
        .unwrap();
    let pane_rect = ed.view.focused_rect();
    let buf = ed.state.buffers.get(bid);
    let gutter_w = cursor::gutter_width(
        ed.view.panes[pid].providers.gutter_columns(),
        buf.text().last_ropey_line(),
    );
    let Editor { state, view, .. } = &mut ed;
    let key = state.format_key(&view.panes[pid]);
    let (mut dlm, vp) =
        commands::pane_display_lines(state.buffers.get(bid), &mut view.panes[pid], key);
    let (content_x, row) = cursor::content_pos(vp, &mut dlm, anchor).expect("visible");
    let expected_x = content_x + gutter_w + pane_rect.x;
    let expected_y = row + pane_rect.y + 1;

    assert_eq!((x, y), (expected_x, expected_y));
}
