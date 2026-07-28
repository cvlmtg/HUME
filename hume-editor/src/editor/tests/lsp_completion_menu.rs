// In-buffer completion menu + Insert-mode dispatch: the
// `handle_completion_key`/`refilter_lsp_completion_after_edit` guard in
// `mappings/insert.rs`, and `sync_completion_menu_view`'s write side (reusing
// the popup/selection-menu widgets' `PopupState`/`PopupOverlay`).
//
// Sessions are constructed directly via `CompletionSession::begin` (bypassing
// `completion-begin!`'s Steel/wire path, already covered by
// `lsp_completion.rs`) — these tests are about the Insert-mode key routing
// and render path, not the filter/rank logic.

use super::*;
use crate::editor::lsp::completion::{CompletionSession, StoredCompletionItem};
use hume_engine::pipeline::RenderContext;
use ratatui::layout::Rect;

fn begin_session(ed: &mut Editor, items: &[(&str, Option<&str>)]) {
    let bid = ed.focused_buffer_id();
    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|(label, detail)| {
            let mut v = serde_json::json!({"label": label});
            if let Some(d) = detail {
                v["detail"] = serde_json::Value::String(d.to_string());
            }
            v
        })
        .collect();
    let items: Vec<StoredCompletionItem> = items_json
        .iter()
        .map(|v| StoredCompletionItem::from_json(v).expect("test item"))
        .collect();
    let session = CompletionSession::begin(&ed.state, bid, items, false).unwrap();
    ed.lsp.completion = Some(session);
}

// ── Pane-fit clamp: menu must render (clamped), never vanish ────────────────
//
// Regression coverage for `resolve_popup_geometry`'s size clamp: before it
// existed, `sync_completion_menu_view` sized the popup against the full
// candidate list with no bound on the pane's actual width/height, and
// `PopupOverlay`'s defensive bounds check silently dropped the *entire*
// popup — not just the overflowing part — whenever the box didn't fit.

#[test]
fn completion_menu_clamps_to_a_short_pane_instead_of_vanishing() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    // MAX_MENU_ROWS is 10 — 12 items would size an unclamped box to
    // 12 rows (+2 frame), taller than the short pane below.
    let labels: Vec<String> = (0..12).map(|i| format!("item{i}")).collect();
    let items: Vec<(&str, Option<&str>)> = labels.iter().map(|l| (l.as_str(), None)).collect();
    begin_session(&mut ed, &items);

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 6, &mut ctx);

    let pane_rect = ed
        .view
        .pane_rect(ed.state.focused_pane_id)
        .expect("focused pane has a rect after prepare_frame");
    let view = ed.state.completion_menu_view.read().unwrap();
    let state = view
        .as_ref()
        .expect("popup must still render, clamped to fit, not vanish");
    assert!(
        state.outer_h <= pane_rect.height,
        "outer_h {} must not exceed pane height {}",
        state.outer_h,
        pane_rect.height
    );
}

#[test]
fn completion_menu_clamps_to_a_narrow_pane_instead_of_vanishing() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    // A label wide enough to overflow the narrow pane below.
    begin_session(
        &mut ed,
        &[("a_very_long_candidate_label_that_overflows_the_pane", None)],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(20, 8, &mut ctx);

    let pane_rect = ed
        .view
        .pane_rect(ed.state.focused_pane_id)
        .expect("focused pane has a rect after prepare_frame");
    let view = ed.state.completion_menu_view.read().unwrap();
    let state = view
        .as_ref()
        .expect("popup must still render, clamped to fit, not vanish");
    assert!(
        state.outer_w <= pane_rect.width,
        "outer_w {} must not exceed pane width {}",
        state.outer_w,
        pane_rect.width
    );
}

// ── Menu appears / typing narrows ─────────────────────────────────────────────

#[test]
fn menu_appears_with_top_items_after_begin() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.feed_key(key('i'));
    begin_session(
        &mut ed,
        &[
            ("foo", Some("fn")),
            ("foobar", None),
            ("food", Some("field")),
        ],
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);
    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 8));
    insta::assert_snapshot!(snap);
}

#[test]
fn typing_narrows_the_filtered_items() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("foobar", None), ("grape", None)]);

    // "g" narrows to just "grape" — must be reflected in the session's own
    // filtered ranking (the render path is exercised separately by the
    // snapshot test above).
    ed.feed_key(key('g'));

    let session = ed.lsp.completion.as_ref().unwrap();
    let top: Vec<String> = session
        .top(10)
        .iter()
        .map(|v| v["label"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(top, vec!["grape"]);
}

// ── Enter: accept / regression when no session ───────────────────────────────

#[test]
fn enter_applies_the_selected_edit_and_closes_the_session() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    ed.feed_key(key_enter());

    assert!(
        ed.lsp.completion.is_none(),
        "session must close after accept"
    );
    assert!(ed.state.completion_menu_view.read().unwrap().is_none());
    let text = ed.doc().text().to_string();
    assert_eq!(text, "foo\n", "insert_text must be applied at the anchor");
}

#[test]
fn enter_with_no_session_inserts_a_newline_regression() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    assert!(ed.lsp.completion.is_none(), "sanity: no session open");

    ed.feed_key(key_enter());

    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "a\n\n",
        "Enter must still insert a newline with no session"
    );
}

// ── Esc: keeps typed text, stays in Insert ────────────────────────────────────

#[test]
fn esc_dismisses_the_session_but_keeps_typed_text_and_stays_in_insert() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);
    ed.feed_key(key('f'));

    ed.feed_key(key_esc());

    assert!(ed.lsp.completion.is_none());
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Insert,
        "Esc dismisses the session, not Insert mode itself"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(text, "f\n", "the typed filter char must not be reverted");
}

// ── Refilter to zero matches: doesn't trap Esc/Enter/Tab ─────────────────────

/// Regression: an open-but-empty session (narrowed to zero matches by
/// continued typing) must not intercept Enter. Before the
/// `handle_completion_key` empty-session guard, this hit `accept(0)` on an
/// empty `filtered` list, reported an "index out of range" error, and
/// swallowed the newline.
#[test]
fn enter_at_zero_matches_inserts_a_newline_instead_of_erroring() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    // "z" isn't a subsequence of "foo" — filtered to empty, session survives.
    ed.feed_key(key('z'));
    assert!(ed.lsp.completion.is_some(), "sanity: session survives");

    ed.feed_key(key_enter());

    assert_eq!(
        ed.state.status_msg, None,
        "must not report completion-accept!'s index-out-of-range error"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "z\n\n",
        "Enter must insert a newline, not be swallowed"
    );
}

/// Same regression for Tab: an empty session must not intercept it into a
/// selection-move no-op (or, before the guard, its own would-be-dead
/// `n == 0` early return) — Tab falls through to normal Insert dispatch,
/// which inserts an indent (`\t` or spaces, depending on `tab-style`).
#[test]
fn tab_at_zero_matches_falls_through_to_normal_insert() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    ed.feed_key(key('z'));
    assert!(ed.lsp.completion.is_some(), "sanity: session survives");
    let before = ed.doc().text().to_string();

    ed.feed_key(key_tab());

    assert!(
        ed.lsp.completion_ui.is_none(),
        "Tab must not create selection UI for a menu that isn't shown"
    );
    let text = ed.doc().text().to_string();
    assert_ne!(
        text, before,
        "Tab must fall through to normal Insert dispatch and change the buffer"
    );
}

#[test]
fn typing_to_zero_matches_keeps_the_session_but_a_single_esc_still_exits_insert() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    // "z" isn't a subsequence of "foo" — the refiltered list is empty, but
    // the session survives (typing more, then backspacing back, must be
    // able to self-heal it — see `backspace_within_the_token_refilters_and_
    // keeps_the_session_open`).
    ed.feed_key(key('z'));
    assert!(
        ed.lsp.completion.is_some(),
        "a transient zero-match refilter must not kill the session outright"
    );

    // Esc must not intercept-and-swallow while nothing is visibly shown —
    // it falls through to the trie's exit-insert leaf, which reaches
    // `set_mode` and dismisses the session as a side effect. A *single* Esc
    // reaching Normal is the regression this guards: the bug was a second,
    // invisible session trapping the first Esc.
    ed.feed_key(key_esc());
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert!(ed.lsp.completion.is_none());
}

// ── Backspace: within token refilters, past anchor dismisses ────────────────

#[test]
fn backspace_within_the_token_refilters_and_keeps_the_session_open() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("grape", None)]);
    ed.feed_key(key('g'));
    ed.feed_key(key('g')); // filter now "gg" — matches neither by prefix

    ed.feed_key(key_backspace());
    assert!(
        ed.lsp.completion.is_some(),
        "backspace within the token must not dismiss the session"
    );
    let session = ed.lsp.completion.as_ref().unwrap();
    let top: Vec<String> = session
        .top(10)
        .iter()
        .map(|v| v["label"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        top,
        vec!["grape"],
        "filter back down to \"g\" must re-match grape"
    );
}

#[test]
fn backspace_past_the_anchor_dismisses_the_session() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    ed.feed_key(key('x')); // one char BEFORE the session's anchor
    begin_session(&mut ed, &[("foo", None)]);

    // No filter chars typed yet — cursor sits exactly at the anchor.
    // Backspace now would delete "x", which lies before the anchor.
    ed.feed_key(key_backspace());

    assert!(
        ed.lsp.completion.is_none(),
        "backspace at the anchor must dismiss the session"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "\n",
        "the backspace itself must still delete \"x\" normally"
    );
}

// ── Tab/Shift+Tab: selection wraps at the boundaries ─────────────────────────

#[test]
fn tab_at_last_item_wraps_to_first() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("bar", None), ("baz", None)]);

    // Three items, indices 0..2. Two Tabs land on the last item (index 2);
    // a third must wrap back to 0 instead of staying clamped at 2.
    ed.feed_key(key_tab());
    ed.feed_key(key_tab());
    assert_eq!(ed.lsp.completion_ui.as_ref().unwrap().selected, 2);

    ed.feed_key(key_tab());
    assert_eq!(
        ed.lsp.completion_ui.as_ref().unwrap().selected,
        0,
        "Tab past the last item must wrap to the first"
    );
}

#[test]
fn shift_tab_at_first_item_wraps_to_last() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("bar", None), ("baz", None)]);

    // No Tab pressed yet — `lsp_completion_ui` is lazily created on first
    // move, defaulting to index 0, which is what BackTab must wrap from.
    ed.feed_key(KeyEvent::new(KeyCode::BackTab, Modifiers::SHIFT));
    assert_eq!(
        ed.lsp.completion_ui.as_ref().unwrap().selected,
        2,
        "Shift+Tab before the first item must wrap to the last"
    );
}

// ── Mode change dismisses ──────────────────────────────────────────────────────

#[test]
fn ctrl_c_exits_insert_and_dismisses_the_session() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    ed.feed_key(KeyEvent::new(KeyCode::Char('c'), Modifiers::CONTROL));

    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert!(ed.lsp.completion.is_none());
    assert!(ed.state.completion_menu_view.read().unwrap().is_none());
}

/// `set_mode` only has `&mut EditorState` — it can't reach `LspState`
/// directly, so a mode change from outside the normal key/mouse dispatch
/// path (e.g. a Steel builtin) can only set the deferred-dismiss flag. Pins
/// that the session survives until the flag is actually consumed, and that
/// `prepare_frame` (the render-time safety net) does consume it.
#[test]
fn mode_change_outside_key_dispatch_dismisses_the_session_by_the_next_frame() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);
    assert!(ed.lsp.completion.is_some(), "sanity: session open");

    ed.state.set_mode(hume_engine::types::EditorMode::Normal);
    assert!(
        ed.lsp.completion.is_some(),
        "the session must survive until the flag is consumed, not disappear on set_mode itself"
    );
    assert!(ed.state.lsp_completion_dismiss_pending);

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);

    assert!(
        ed.lsp.completion.is_none(),
        "prepare_frame must consume the deferred dismissal before rendering"
    );
    assert!(ed.state.completion_menu_view.read().unwrap().is_none());
}

// ── Regression: typing after accept must not desync the edit group ──────────

#[test]
fn typing_after_accept_composes_into_the_open_edit_group_without_panicking() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "DEFAULT_".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("DEFAULT_WIDTH", None)]);

    // Accept applies the completion's (longer) insert_text through
    // `apply_text_edits` while the insert session's edit group is still
    // open — that edit must compose into the group, not record a
    // standalone revision, or the very next keystroke's `ChangeSet::compose`
    // panics on a length mismatch.
    ed.feed_key(key_enter());
    assert!(
        ed.lsp.completion.is_none(),
        "session must close after accept"
    );

    ed.feed_key(key(','));
    let text = ed.doc().text().to_string();
    assert_eq!(text, "DEFAULT_WIDTH,\n");

    // One undo reverts the whole insert session, including the completion
    // accept — it composed into the same group as everything else typed.
    ed.feed_key(key_esc());
    ed.feed_key(key('u'));
    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "\n",
        "undo must revert the entire session as one step"
    );
}

#[test]
fn typing_after_moving_the_cursor_before_the_anchor_dismisses_instead_of_panicking() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abc".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("abc", None)]);

    // Left isn't intercepted by `handle_completion_key` (only Tab/BackTab/
    // Up/Down/Enter/Esc/Backspace are) — it's handled by the insert trie and
    // returns before reaching the refilter guard, so the session survives
    // with its anchor now stale relative to the cursor. Two presses land the
    // cursor two chars before the anchor, so the very next char inserted
    // still leaves `head < anchor` — the inverted-range case.
    ed.feed_key(KeyEvent::new(KeyCode::Left, Modifiers::NONE));
    ed.feed_key(KeyEvent::new(KeyCode::Left, Modifiers::NONE));
    assert!(
        ed.lsp.completion.is_some(),
        "sanity: Left does not itself dismiss the session"
    );

    ed.feed_key(key('x'));
    assert!(
        ed.lsp.completion.is_none(),
        "a stale anchor past the cursor must dismiss the session, not panic"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(text, "axbc\n");
}

// ── Regression: minibuffer `:e <Tab>` completion untouched ───────────────────

#[test]
fn minibuffer_e_tab_completion_is_unaffected_by_the_lsp_completion_guard() {
    let tmp = safe_tempdir();
    let file_path = tmp.path().join("hello.rs");
    std::fs::write(&file_path, "").unwrap();

    let mut ed = editor_from("-[x]>\n");
    type_cmd(&mut ed, &format!(":e {}", tmp.path().display()));
    ed.feed_key(key_tab());

    // Whatever the minibuffer's own completion produces, dispatch must not
    // have been intercepted or altered by the LSP completion guard (no
    // session exists in Command mode at all).
    assert!(ed.lsp.completion.is_none());
}
