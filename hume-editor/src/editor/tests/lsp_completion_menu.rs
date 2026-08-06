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
use crate::editor::buffer::Buffer;
use crate::editor::lsp::completion::{CompletionSession, StoredCompletionItem};
use crate::editor::{commands, cursor};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_engine::format::FormatScratch;
use hume_engine::pane::WrapMode;
use hume_engine::pipeline::RenderContext;
use ratatui::layout::Rect;

fn begin_session(ed: &mut Editor, items: &[(&str, Option<&str>)]) {
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
    begin_session_items(ed, &items_json);
}

/// Generalized form of [`begin_session`] for items carrying `textEdit` /
/// `additionalTextEdits` — arbitrary JSON, not just label/detail.
fn begin_session_items(ed: &mut Editor, items: &[serde_json::Value]) {
    let bid = ed.focused_buffer_id();
    let items: Vec<StoredCompletionItem> = items
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
    ed.sync_viewport_dims(40, 6);
    ed.settle();
    ed.prepare_frame(&mut ctx);

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
    ed.sync_viewport_dims(20, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

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
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);
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
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

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
fn left_arrow_dismisses_the_session_immediately() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "abc".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("abc", None)]);

    // Left isn't intercepted by `handle_completion_key` (only Tab/BackTab/
    // Up/Down/Enter/Esc/Backspace are) — it resolves through the insert
    // trie's `WalkResult::Leaf` arm instead, which now dismisses any open
    // completion session unconditionally before running the motion. Before
    // this fix the session instead survived with a now-stale anchor, so a
    // later `Enter` would accept using a `(back, forward)` span still
    // derived from the pre-move anchor — silently swallowing whatever the
    // cursor had moved across.
    ed.feed_key(KeyEvent::new(KeyCode::Left, Modifiers::NONE));
    assert!(
        ed.lsp.completion.is_none(),
        "a motion key must dismiss the session immediately, not leave a stale anchor"
    );

    ed.feed_key(key('x'));
    let text = ed.doc().text().to_string();
    assert_eq!(
        text, "abxc\n",
        "sanity: typing after dismissal is ordinary insertion, at the moved cursor"
    );
}

#[test]
fn right_arrow_then_enter_dismisses_instead_of_swallowing_the_passed_over_char() {
    // The finding this fixes, reproduced with default keybindings: cursor
    // sits right before an existing 'X' (typed "pri" then triggered
    // completion mid-line), press Right once — stepping over 'X' without
    // dismissing the session, pre-fix — then Enter. Pre-fix, `accept`
    // re-anchored its `(back, forward)` span to the live head, so accepting
    // would have overwritten the token *plus* 'X'. Post-fix, Right dismisses
    // the session outright, so Enter is an ordinary newline and 'X' survives.
    let mut ed = editor_from("pri-[X]>\n");
    ed.feed_key(key('i'));
    begin_session(&mut ed, &[("print", None)]);
    assert!(ed.lsp.completion.is_some(), "sanity: session is open");

    ed.feed_key(KeyEvent::new(KeyCode::Right, Modifiers::NONE));
    assert!(
        ed.lsp.completion.is_none(),
        "Right must dismiss the session immediately"
    );

    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "priX\n\n",
        "Enter with no session open must insert a plain newline, not accept — \
         and 'X' (the char the cursor stepped over) must survive intact"
    );
}

#[test]
fn ctrl_w_dismisses_the_session_instead_of_leaving_a_stale_anchor() {
    // `Ctrl+W` (delete-word-backward) is a `MappableCommand::Edit` bound in
    // the insert trie — it runs through `run_native_body`, not
    // `apply_insert_edit`, so it can never call `observe_edit` to keep the
    // session's anchor in sync (the lint `single native-dispatch funnel
    // discipline` forbids routing it any other way). Before this fix the
    // session survived with a now-meaningless anchor; post-fix, any
    // trie-matched key (this one included) dismisses the session outright.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "pri".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("print", None)]);
    assert!(ed.lsp.completion.is_some(), "sanity: session is open");

    ed.feed_key(key_ctrl('w'));
    assert!(
        ed.lsp.completion.is_none(),
        "Ctrl+W must dismiss the session immediately"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "sanity: delete-word-backward still runs normally once the session is out of the way"
    );
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

// ── Regression: stale anchor after an out-of-band buffer change ─────────────
//
// `session.anchor()` is derived by mapping the `completion-begin!`-time
// position through every edit `CompletionSession::observe_edit` has been
// told about — but only `apply_insert_edit` (the chokepoint every ordinary
// Insert-mode keystroke goes through) ever calls `observe_edit`. An edit from
// any other source (a `:e!` reload, a pane switching to a different buffer)
// bypasses it entirely and can leave `anchor` pointing past the
// currently-focused buffer's end, and `sync_completion_menu_view` must not
// panic walking `RowMap::locate` with it.

#[test]
fn stale_anchor_after_a_buffer_reload_skips_render_instead_of_panicking() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for line in ["line0", "line1", "line2", "line3", "line4"] {
        for ch in line.chars() {
            ed.feed_key(key(ch));
        }
        ed.feed_key(key_enter());
    }
    // Cursor is now on the blank line past "line4\n" — the session anchor.
    begin_session(&mut ed, &[("candidate", None)]);
    let bid = ed.focused_buffer_id();
    let anchor = ed.lsp.completion.as_ref().unwrap().anchor();
    assert!(anchor > 3, "sanity: anchor is deep in the buffer");

    // `reload_buffer_in_place` (`:e!`) clamps every pane's cursor to the new,
    // much shorter content, but has no notion of an open completion session
    // — so `anchor` is left pointing past the reloaded buffer's end.
    let replacement = Buffer::new(Text::from("hi\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx); // must not panic

    assert!(
        ed.state.completion_menu_view.read().unwrap().is_none(),
        "popup must not render against a stale out-of-range anchor"
    );
    assert!(
        ed.lsp.completion.is_some(),
        "the guard skips only this frame's render — dismissal stays with \
         the existing keypress-driven paths"
    );
}

#[test]
fn stale_anchor_after_switching_focus_to_another_buffer_skips_render() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for ch in "hello".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("candidate", None)]);
    assert!(ed.lsp.completion.is_some(), "sanity: session open");

    // Switch focus to a different buffer without dismissing the session —
    // `sync_completion_menu_view` builds its `RowMap` over whichever buffer
    // is focused *now*, not the one the session was opened against.
    let other = ed.open_buffer(Buffer::new(Text::from("other\n"), SelectionSet::default()));
    ed.switch_to_buffer_with_jump(other);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx); // must not panic

    assert!(
        ed.state.completion_menu_view.read().unwrap().is_none(),
        "popup must not render a session anchored to a buffer that isn't focused"
    );
}

// ── Regression: overlay anchor reuses the scroll pass's cached cursor cell ──
//
// `popup_anchor_and_bounds` takes a fast path when its `anchor_char` is the
// focused cursor: it reuses `ctx.cursor_screen`, resolved by `scroll_into_view`
// earlier in `prepare_frame`, instead of re-walking the row list. Pins that
// the reused cell agrees with a full, independent walk — in wrap mode, where
// that walk is a per-line format, so a wrong cache would show up as a
// silently-misplaced popup, not a panic.

#[test]
fn completion_popup_anchor_matches_an_independent_screen_pos_walk_when_wrapped() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let pid = ed.state.focused_pane_id;
    // Explicit non-zero width, independent of the terminal size passed to
    // `prepare_frame` below, so the cursor lands several wrap rows into the
    // line regardless of pane width.
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(WrapMode::Soft { width: 6 }),
        saved: None,
    });

    ed.feed_key(key('i'));
    for ch in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("candidate", None)]);

    let mut ctx = RenderContext::new();
    // Ample room on every side: `resolve_popup_geometry` neither flips above
    // the cursor nor clamps the position, so the popup's (x, y) is exactly
    // (anchor_x, anchor_y + 1) — letting this test check the anchor cell
    // itself without reimplementing that geometry logic.
    ed.sync_viewport_dims(80, 24);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let (x, y) = {
        let view = ed.state.completion_menu_view.read().unwrap();
        let state = view.as_ref().expect("popup must be showing");
        (state.x, state.y)
    };

    // Independent oracle: re-derive the same cell via a fresh `RowMap` and
    // `cursor::screen_pos` — the exact primitives the fast path's slow
    // fallback uses — entirely bypassing `ctx.cursor_screen`.
    let bid = ed.focused_buffer_id();
    let cursor_char = ed.current_selections().primary().head();
    let pane_rect = ed.view.pane_rect(pid).expect("focused pane has a rect");
    let buf = ed.state.buffers.get(bid);
    let gutter_w = cursor::gutter_width(
        ed.view.panes[pid].providers.gutter_columns(),
        buf.text().len_lines(),
    );
    let mut scratch = FormatScratch::new();
    let mut rm = commands::pane_row_map(buf, &ed.state.settings, &ed.view.panes[pid], &mut scratch);
    let vp = &ed.view.panes[pid].viewport;
    let (col, row) = cursor::screen_pos(vp, &mut rm, cursor_char).expect("cursor is visible");
    let expected_x = col + gutter_w + pane_rect.x;
    let expected_y = row + pane_rect.y + 1; // resolve_popup_geometry: room below → anchor_y + 1

    assert_eq!((x, y), (expected_x, expected_y));
}

// ── Multi-cursor accept ─────────────────────────────────────────────────────
//
// `c` on two selections leaves two collapsed cursors in one Insert session
// (`hume-ops/src/edit/tests/delete.rs`'s `change_span`/`delete_selection`
// tests cover the deletion itself) — these pin that accepting a completion
// lands the edit at every one of them, not just the primary, and that the
// session's own bookkeeping (anchor, undo grouping) stays correct regardless
// of which cursor is primary.

#[test]
fn accepting_a_completion_lands_at_every_cursor_not_just_the_primary() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    for ch in "st".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("std", None)]);
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "std std\n");
}

#[test]
fn accepting_a_server_text_edit_also_lands_at_every_cursor() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    for ch in "st".chars() {
        ed.feed_key(key(ch));
    }
    let head = ed.current_selections().primary().head();
    // `newText` distinct from `label`/`insertText` — proves the server's
    // range drove the replacement, not the `insertText` fallback.
    begin_session_items(
        &mut ed,
        &[serde_json::json!({
            "label": "std",
            "insertText": "ignored-fallback",
            "textEdit": {
                "range": {
                    "start": {"line": 0, "character": (head - 2) as u32},
                    "end": {"line": 0, "character": head as u32}
                },
                "newText": "STD"
            }
        })],
    );
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "STD STD\n");
}

#[test]
fn additional_text_edits_land_once_not_once_per_cursor() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    for ch in "st".chars() {
        ed.feed_key(key(ch));
    }
    begin_session_items(
        &mut ed,
        &[serde_json::json!({
            "label": "std",
            "insertText": "std",
            "additionalTextEdits": [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                 "newText": "// header\n"}
            ]
        })],
    );
    ed.feed_key(key_enter());
    let text = ed.doc().text().to_string();
    assert_eq!(text, "// header\nstd std\n");
    assert_eq!(
        text.matches("// header\n").count(),
        1,
        "additionalTextEdits has no cursor of its own — must land once, not per cursor"
    );
}

#[test]
fn additional_text_edits_track_a_real_edit_observed_since_begin_not_the_live_rope_directly() {
    // Request-time document: "fo, extra\n" — additionalTextEdits' wire range
    // (chars 4..9, "extra") is computed against exactly this document.
    // Cursor sits right after "fo" (before the comma).
    let mut ed = editor_from("fo-[,]> extra\n");
    ed.feed_key(key('i'));
    begin_session_items(
        &mut ed,
        &[serde_json::json!({
            "label": "foo",
            "insertText": "func",
            "additionalTextEdits": [
                {"range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 9}},
                 "newText": "EXTRA"}
            ]
        })],
    );
    // A real keystroke since `begin` shifts everything after it by one char
    // — decoding additionalTextEdits' wire range against the *live* rope
    // directly (the pre-fix bug) would land one char off ("extr" preceded
    // by the space, not "extra"); decoding against `rope_at_begin` and
    // mapping forward through the observed edit (the fix) still finds
    // "extra" exactly, regardless of what happened elsewhere on the line.
    ed.feed_key(key('o'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foo, extra\n",
        "sanity: real edit landed"
    );

    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "func, EXTRA\n",
        "additionalTextEdits must track the real edit typed since begin, landing on \
         \"extra\" — not on whatever the same wire offsets now point at in the live rope"
    );
}

#[test]
fn multi_cursor_accept_is_one_undo_step_in_insert_mode() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    for ch in "st".chars() {
        ed.feed_key(key(ch));
    }
    begin_session(&mut ed, &[("std", None)]);
    ed.feed_key(key_enter());
    ed.feed_key(key('!'));
    assert_eq!(ed.doc().text().to_string(), "std! std!\n");
    ed.feed_key(key_esc());
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foo bar\n",
        "one undo reverts the whole session: both cursors' completion \
         accept plus the char typed after it"
    );
}

#[test]
fn multi_cursor_accept_is_one_undo_step_from_steel_outside_insert_mode() {
    // Two cursors placed directly, entirely in Normal mode — no `c`, no
    // Insert-mode key ever pressed, so no edit group is open going in.
    // Mirrors `lsp_completion.rs`'s `accept_is_one_undo_step` (single
    // cursor, same Steel-only setup) but with two.
    let mut ed = editor_from("-[a]>bcdef gh-[i]>jkl\n");
    begin_session_items(
        &mut ed,
        &[serde_json::json!({
            "label": "std",
            "insertText": "X",
            "additionalTextEdits": [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                 "newText": "// header\n"}
            ]
        })],
    );
    let session = ed.lsp.completion.take().expect("session open");
    session
        .accept(&mut ed.state, &mut ed.lsp, 0)
        .expect("accept must succeed");
    let text = ed.doc().text().to_string();
    assert_eq!(
        text.matches('X').count(),
        2,
        "both cursors must receive the completion"
    );
    assert!(
        text.starts_with("// header\n"),
        "additionalTextEdits must land"
    );

    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef ghijkl\n",
        "one undo must revert both cursors' completion and additionalTextEdits \
         together, even though accept opened its own edit group"
    );
}

#[test]
fn anchor_remap_keeps_the_filter_correct_when_primary_is_not_the_first_cursor() {
    // Two cursors from one `c`, but the SECOND is primary — every keystroke
    // inserts at cursor 1 *before* cursor 2 (primary) shifts cursor 2's head
    // by more than one char. Without remapping the session anchor through
    // each keystroke, the filter would pick up drifted text instead of what
    // was actually typed at the primary.
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    // Force the second (higher-offset) cursor to be primary.
    {
        let sels = &mut ed.state.panes.state[pid][bid].selections;
        let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head()).collect();
        *sels = SelectionSet::from_vec(heads.iter().map(|&h| Selection::collapsed(h)).collect(), 1);
    }
    begin_session(&mut ed, &[("candidate", None)]);
    for ch in "st".chars() {
        ed.feed_key(key(ch));
    }
    let session = ed.lsp.completion.as_ref().expect("session stays open");
    assert_eq!(
        ed.doc()
            .text()
            .slice(session.anchor()..ed.current_selections().primary().head())
            .to_string(),
        "st",
        "filter span (anchor..primary head) must be exactly what was typed \
         at the primary cursor, not text drifted in from the other cursor's \
         own insert"
    );
}
