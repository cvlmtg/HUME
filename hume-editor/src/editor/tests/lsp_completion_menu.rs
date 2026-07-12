// In-buffer completion menu + Insert-mode dispatch: the
// `handle_completion_key`/`refilter_lsp_completion_after_edit` guard in
// `mappings/insert.rs`, and `sync_lsp_completion_view`'s write side (reusing
// the popup/selection-menu widgets' `PopupState`/`PopupOverlay`).
//
// Sessions are constructed directly via `CompletionSession::begin` (bypassing
// `completion-begin!`'s Steel/wire path, already covered by
// `lsp_completion.rs`) — these tests are about the Insert-mode key routing
// and render path, not the filter/rank logic.

use super::*;
use crate::editor::lsp::completion::CompletionSession;
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
    let session = CompletionSession::begin(&ed.state, bid, &items_json, false).unwrap();
    ed.state.lsp_completion = Some(session);
}

// ── Menu appears / typing narrows ─────────────────────────────────────────────

#[test]
fn menu_appears_with_top_items_after_begin() {
    let mut ed = Editor::open(None).unwrap();
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
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("foobar", None), ("grape", None)]);

    // "g" narrows to just "grape" — must be reflected in the session's own
    // filtered ranking (the render path is exercised separately by the
    // snapshot test above).
    ed.feed_key(key('g'));

    let session = ed.state.lsp_completion.as_ref().unwrap();
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
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    ed.feed_key(key_enter());

    assert!(
        ed.state.lsp_completion.is_none(),
        "session must close after accept"
    );
    assert!(ed.state.lsp_completion_view.read().unwrap().is_none());
    let text = ed.doc().text().to_string();
    assert_eq!(text, "foo\n", "insert_text must be applied at the anchor");
}

#[test]
fn enter_with_no_session_inserts_a_newline_regression() {
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    assert!(ed.state.lsp_completion.is_none(), "sanity: no session open");

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
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);
    ed.feed_key(key('f'));

    ed.feed_key(key_esc());

    assert!(ed.state.lsp_completion.is_none());
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Insert,
        "Esc dismisses the session, not Insert mode itself"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(text, "f\n", "the typed filter char must not be reverted");
}

// ── Backspace: within token refilters, past anchor dismisses ────────────────

#[test]
fn backspace_within_the_token_refilters_and_keeps_the_session_open() {
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None), ("grape", None)]);
    ed.feed_key(key('g'));
    ed.feed_key(key('g')); // filter now "gg" — matches neither by prefix

    ed.feed_key(key_backspace());
    assert!(
        ed.state.lsp_completion.is_some(),
        "backspace within the token must not dismiss the session"
    );
    let session = ed.state.lsp_completion.as_ref().unwrap();
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
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    ed.feed_key(key('x')); // one char BEFORE the session's anchor
    begin_session(&mut ed, &[("foo", None)]);

    // No filter chars typed yet — cursor sits exactly at the anchor.
    // Backspace now would delete "x", which lies before the anchor.
    ed.feed_key(key_backspace());

    assert!(
        ed.state.lsp_completion.is_none(),
        "backspace at the anchor must dismiss the session"
    );
    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "\n",
        "the backspace itself must still delete \"x\" normally"
    );
}

// ── Mode change dismisses ──────────────────────────────────────────────────────

#[test]
fn ctrl_c_exits_insert_and_dismisses_the_session() {
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("foo", None)]);

    ed.feed_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert!(ed.state.lsp_completion.is_none());
    assert!(ed.state.lsp_completion_view.read().unwrap().is_none());
}

// ── Regression: minibuffer `:e <Tab>` completion untouched ───────────────────

#[test]
fn minibuffer_e_tab_completion_is_unaffected_by_the_lsp_completion_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("hello.rs");
    std::fs::write(&file_path, "").unwrap();

    let mut ed = editor_from("-[x]>\n");
    type_cmd(&mut ed, &format!(":e {}", tmp.path().display()));
    ed.feed_key(key_tab());

    // Whatever the minibuffer's own completion produces, dispatch must not
    // have been intercepted or altered by the LSP completion guard (no
    // session exists in Command mode at all).
    assert!(ed.state.lsp_completion.is_none());
}
