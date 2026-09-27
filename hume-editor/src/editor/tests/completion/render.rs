//! The menu's geometry (`sync_completion_menu_view`/
//! `sync_minibuf_completion_view` through `hume_ui::popup::menu_window` +
//! `resolve_menu`) and the two full-frame snapshots.

use super::*;
use crate::editor::{commands, cursor};
use hume_engine::pane::WrapMode;
use hume_grid::Rect;

// ── Pane-fit clamp: the menu renders clamped, never vanishes ────────────────
//
// `PopupOverlay`'s defensive bounds check silently drops the *entire* popup
// whenever the box doesn't fit — so the write side must clamp first.

#[test]
fn completion_menu_clamps_to_a_short_pane_instead_of_vanishing() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    // MAX_MENU_ROWS is 10 — 12 items would size an unclamped box to 12
    // rows (+2 frame), taller than the short pane below.
    let labels: Vec<String> = (0..12).map(|i| format!("item{i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    open_completion_session(&mut ed, &labels);

    frame(&mut ed, 40, 6);

    let pane_rect = ed.view.pane_rect(ed.state.focus.id()).expect("rect");
    let view = ed.state.views.completion_menu.read();
    let state = view
        .as_ref()
        .expect("popup must render clamped, not vanish");
    assert!(state.rect.height <= pane_rect.height);
}

#[test]
fn completion_menu_clamps_to_a_narrow_pane_instead_of_vanishing() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    open_completion_session(
        &mut ed,
        &["a_very_long_candidate_label_that_overflows_the_pane"],
    );

    frame(&mut ed, 20, 8);

    let pane_rect = ed.view.pane_rect(ed.state.focus.id()).expect("rect");
    let view = ed.state.views.completion_menu.read();
    let state = view
        .as_ref()
        .expect("popup must render clamped, not vanish");
    assert!(state.rect.width <= pane_rect.width);
}

// ── Snapshots ─────────────────────────────────────────────────────────────────

/// The Insert-mode menu: two columns (`label`, then `detail` right-aligned),
/// sized to its visible rows.
#[test]
fn menu_appears_with_top_items_after_the_source_answers() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();
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
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
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

// ── The anchor reuses the scroll pass's cached cursor cell ──────────────────
//
// `popup_placement` takes a fast path when its anchor is the focused
// cursor: it reuses `ctx.cursor_content_pos` from `scroll_into_view` instead
// of re-walking the display-line list. Pins that the reused cell agrees
// with a full, independent walk — in wrap mode, where a wrong cache would
// show up as a silently-misplaced popup, not a panic.

#[test]
fn completion_popup_anchor_matches_an_independent_content_pos_walk_when_wrapped() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let pid = ed.state.focus.id();
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 6 }),
        saved: None,
    });

    ed.feed_key(key('i'));
    type_chars(&mut ed, "abcdefghijklmnopqrstuvwxyz0123456789");
    // A non-word char right before triggering: the word token is empty,
    // exactly at the cursor — the fast path this test pins is only taken
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
        .menu_anchor_char()
        .unwrap();
    let pane_rect = ed.view.pane_rect(pid).expect("rect");
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
