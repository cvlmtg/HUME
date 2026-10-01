//! The Insert-mode completion menu's key handling (`completion_input_buffer`,
//! `input_stack/completion.rs`) and the session's lifetime under keys,
//! mode changes, and out-of-band buffer changes.

use super::*;
use crate::editor::buffer::Buffer;
use crate::editor::host_impl::EditorHostImpl;
use hume_editing::text::BufferText;
use hume_scripting::host::{PopupKind, UiHost};
use termina::event::{KeyCode, Modifiers};

// ── Interaction with the two popup homes ─────────────────────────────────────
//
// `show_popup` is called directly through `EditorHostImpl` rather than a
// typed `:` command: `:` while already in Insert inserts a literal colon
// instead of entering Command mode, and both real callers (hover,
// signature help) reach `show_popup` the same way, via an async LSP
// response landing outside the keymap dispatcher.

/// A completion session opening while a `Scrollable` popup (hover, or the
/// `gn`/`gp` diagnostic overlay) is up must retire the popup first:
/// a completion layer's `LayerOnly` eviction clears the pushed-layer popup
/// home before landing, keeping `PopupLayer`'s "never buried" invariant.
#[test]
fn completion_over_a_live_scrollable_popup_clears_it() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_popup(
        pane,
        "hover".to_string(),
        PopupKind::Scrollable,
        false,
        None,
    )
    .expect("show-popup! must succeed");
    assert!(ed.state.input.popup().is_some(), "sanity: popup open");

    open_completion_session(&mut ed, &["foo"]);

    assert!(
        ed.state.input.buffer_completion().is_some(),
        "the session opened"
    );
    assert!(
        ed.state.input.popup().is_none(),
        "the popup must be cleared, not buried, when completion lands above it"
    );
}

/// A `Sticky` popup (LSP signature help) sits in the mode layer's own slot
/// and must survive a completion session opening alongside it. A naive
/// `clear_popups` (both homes) would kill it.
#[test]
fn completion_over_a_sticky_popup_leaves_it_open() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_popup(pane, "sig-help".to_string(), PopupKind::Sticky, false, None)
        .expect("show-popup! must succeed");
    assert!(ed.state.input.popup().is_some(), "sanity: sighelp open");

    open_completion_session(&mut ed, &["foo"]);

    assert!(
        ed.state.input.buffer_completion().is_some(),
        "the session opened"
    );
    assert!(
        ed.state.input.popup().is_some(),
        "signature help must survive a completion session opening alongside it"
    );
}

/// The reverse order: a `Popup` (hover, arriving async after a trigger char
/// opened the menu, say) lands *above* an already-open completion session.
/// `dismiss_completion`/`take_buffer_completion` reach the session by its
/// own `LayerRef` (`ref_of`) rather than a pop-if-top rule. This pins that
/// a popup landing above it doesn't strand it: `EditorState::dismiss_
/// completion`'s own doc (`mod.rs`) is explicit that "a `Completion` layer
/// can sit under a `Popup`, and a pop-if-top rule would leave a stale
/// session behind one."
#[test]
fn a_popup_landing_above_a_live_session_does_not_strand_it() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "sanity: session open"
    );

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_popup(
        pane,
        "hover".to_string(),
        PopupKind::Scrollable,
        false,
        None,
    )
    .expect("show-popup! must succeed");
    assert!(
        ed.state.input.popup().is_some(),
        "sanity: popup landed above the session"
    );
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "the session must still be reachable, not buried under the popup"
    );

    assert!(
        ed.state.take_buffer_completion(&ed.view).is_some(),
        "take_buffer_completion must find the layer wherever it sits on the stack, \
         not only when it's top-of-stack"
    );
}

/// A background dismiss (`dismiss_completion`, e.g. the session going spent)
/// must not take a popup stacked above it down too: a completion layer's
/// `removal_scope` is `SelfOnly` for exactly the same coexistence
/// `popup_eviction`'s `LayerOnly` override already declares. The default
/// `Stack` scope would otherwise have `retire::<BufferCompletionLayer>` reach for
/// `truncate_layers`, destroying the popup as collateral.
#[test]
fn dismissing_the_session_leaves_a_popup_above_it_open() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_popup(
        pane,
        "hover".to_string(),
        PopupKind::Scrollable,
        false,
        None,
    )
    .expect("show-popup! must succeed");
    assert!(
        ed.state.input.popup().is_some(),
        "sanity: popup landed above the session"
    );

    ed.state.dismiss_completion(&ed.view);

    assert!(
        ed.state.input.buffer_completion().is_none(),
        "the session itself is gone"
    );
    assert!(
        ed.state.input.popup().is_some(),
        "the popup above it must survive: it's stacked over the session by \
         coincidence, not by dependency on it staying open"
    );
}

// ── Typing narrows / Enter / Esc ──────────────────────────────────────────────

#[test]
fn typing_narrows_the_ranked_items() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo", "foobar", "grape"]);
    ed.feed_key(key('g'));
    assert_eq!(labels(&ed), vec!["grape"]);
}

/// Typing resets the selection to row 0: the previous index has no
/// guaranteed meaning against the new order.
#[test]
fn typing_resets_the_selection_to_row_zero() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["a", "b", "c"]);
    ed.feed_key(key_tab());
    ed.feed_key(key_tab());
    assert_eq!(selected_row(&ed), 2, "sanity: two Tabs move off row 0");
    ed.feed_key(key('a'));
    assert_eq!(
        selected_row(&ed),
        0,
        "a re-rank clears the selection back to row 0"
    );
}

#[test]
fn enter_applies_the_selected_item_and_closes_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);

    ed.feed_key(key_enter());

    assert!(
        ed.state.input.buffer_completion().is_none(),
        "session must close after accept"
    );
    assert!(ed.state.views.completion_menu.read().is_none());
    assert_eq!(ed.doc().text().to_string(), "foo\n");
}

#[test]
fn enter_with_no_session_inserts_a_newline() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    assert!(ed.state.input.buffer_completion().is_none(), "sanity");
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "a\n\n");
}

#[test]
fn esc_dismisses_the_session_but_keeps_typed_text_and_stays_in_insert() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    ed.feed_key(key('f'));

    ed.feed_key(key_esc());

    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "Esc dismisses the session, not Insert mode itself"
    );
    assert_eq!(ed.doc().text().to_string(), "f\n");
}

// ── Narrowed to zero matches: doesn't trap Esc/Enter/Tab ────────────────────

/// An open-but-empty session (narrowed to zero matches by continued
/// typing) must not intercept Enter into an "index out of range" accept.
#[test]
fn enter_at_zero_matches_inserts_a_newline_instead_of_erroring() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    ed.feed_key(key('z'));
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "sanity: survives"
    );

    ed.feed_key(key_enter());

    assert_eq!(ed.state.status_msg, None);
    assert_eq!(ed.doc().text().to_string(), "z\n\n");
}

/// Same for Tab: it falls through to normal Insert dispatch (an indent).
#[test]
fn tab_at_zero_matches_falls_through_to_normal_insert() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    ed.feed_key(key('z'));
    let before = ed.doc().text().to_string();

    ed.feed_key(key_tab());

    assert_eq!(
        selected_row(&ed),
        0,
        "Tab must not move the selection for a menu that isn't shown"
    );
    assert_ne!(ed.doc().text().to_string(), before);
}

/// A single Esc reaching Normal is the regression this guards: an
/// invisible empty session must not trap the first Esc.
#[test]
fn typing_to_zero_matches_keeps_the_session_but_a_single_esc_still_exits_insert() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    ed.feed_key(key('z'));
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "a transient zero-match must not kill the session outright"
    );

    ed.feed_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.input.buffer_completion().is_none());
}

// ── Backspace: within the token narrows, past it dismisses ──────────────────

#[test]
fn backspace_within_the_token_refilters_and_keeps_the_session_open() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo", "grape"]);
    ed.feed_key(key('g'));
    ed.feed_key(key('g')); // "gg" matches neither

    ed.feed_key(key_backspace());
    assert!(ed.state.input.buffer_completion().is_some());
    assert_eq!(labels(&ed), vec!["grape"], "back down to \"g\"");
}

/// With the cursor exactly at the token's start, Backspace deletes the char
/// *before* the token, crossing it, not narrowing it.
#[test]
fn backspace_past_the_token_start_dismisses_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    // A non-word char right before triggering: the word token is empty,
    // starting exactly at the cursor.
    ed.feed_key(key(';'));
    open_completion_session(&mut ed, &["foo"]);

    ed.feed_key(key_backspace());

    assert!(
        ed.state.input.buffer_completion().is_none(),
        "backspace at the token start must dismiss the session"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "the backspace itself must still delete \";\" normally"
    );
}

/// A word already typed before the trigger is part of the seeded token, so
/// a Backspace that only removes part of it narrows the session exactly
/// like removing a char typed after the trigger would.
#[test]
fn backspace_within_a_seeded_prefix_narrows_instead_of_dismissing() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "fo");
    open_completion_session(&mut ed, &["foo", "bar"]);
    assert_eq!(labels(&ed), vec!["foo"], "sanity: seeded \"fo\" narrows");

    ed.feed_key(key_backspace());

    assert!(ed.state.input.buffer_completion().is_some());
    assert_eq!(labels(&ed), vec!["foo"], "back down to \"f\"");
    assert_eq!(ed.doc().text().to_string(), "f\n");
}

/// Deleting the token's *first* char (cursor mid-token) stays inside it.
#[test]
fn backspace_on_the_tokens_first_char_keeps_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "fo");
    open_completion_session(&mut ed, &["foo", "bar"]);
    ed.feed_key(key_backspace());
    ed.feed_key(key_backspace());
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "the token is now empty but not crossed"
    );
    assert_eq!(
        labels(&ed),
        vec!["bar", "foo"],
        "an empty token shows everything"
    );
    ed.feed_key(key_backspace());
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "at the buffer's start there is nothing before the token to cross; a \
         Backspace that deletes nothing leaves the session as it was"
    );
}

// ── Tab/Shift-Tab wrap ───────────────────────────────────────────────────────

#[test]
fn tab_at_last_item_wraps_to_first() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo", "bar", "baz"]);

    ed.feed_key(key_tab());
    ed.feed_key(key_tab());
    assert_eq!(selected_row(&ed), 2);

    ed.feed_key(key_tab());
    assert_eq!(selected_row(&ed), 0, "Tab past the last item must wrap");
}

#[test]
fn shift_tab_at_first_item_wraps_to_last() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo", "bar", "baz"]);
    ed.feed_key(key_shift_tab());
    assert_eq!(selected_row(&ed), 2);
}

// ── Mode change dismisses ──────────────────────────────────────────────────────

#[test]
fn ctrl_c_exits_insert_and_dismisses_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);

    ed.feed_key(key_ctrl('c'));

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.input.buffer_completion().is_none());
    assert!(ed.state.views.completion_menu.read().is_none());
}

/// The `Completion` layer sits above `Insert`, so a mode change from outside
/// key dispatch (simulated by truncating the mode layer directly) removes it
/// in the same top-first `truncate_layers` call, synchronously.
#[test]
fn mode_change_outside_key_dispatch_dismisses_the_session_synchronously() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    let r = ed.state.input.mode_layer();
    ed.state.truncate_layers(&ed.view, r);

    assert!(ed.state.input.buffer_completion().is_none());
    assert!(ed.state.views.completion_menu.read().is_none());
}

// ── Bound keys dismiss (motions, edit commands) ─────────────────────────────

/// A bound command dismisses; the session isn't reconciled against a
/// motion, so it never leaves a stale token a later Enter would accept
/// against.
#[test]
fn left_arrow_dismisses_the_session_immediately() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "abc");
    open_completion_session(&mut ed, &["abc"]);

    ed.feed_key(KeyEvent::new(KeyCode::Left, Modifiers::NONE));
    assert!(ed.state.input.buffer_completion().is_none());

    ed.feed_key(key('x'));
    assert_eq!(ed.doc().text().to_string(), "abxc\n");
}

#[test]
fn right_arrow_then_enter_dismisses_instead_of_swallowing_the_passed_over_char() {
    let mut ed = editor_from("pri-[X]>\n");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["print"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(KeyEvent::new(KeyCode::Right, Modifiers::NONE));
    assert!(ed.state.input.buffer_completion().is_none());

    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "priX\n\n",
        "a plain newline; 'X' (the char the cursor stepped over) survives"
    );
}

/// `Ctrl-w` (delete-word-backward) is an edit command bound in the insert
/// trie, and a bound command dismisses.
#[test]
fn ctrl_w_dismisses_the_session_instead_of_leaving_a_stale_token() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "pri");
    open_completion_session(&mut ed, &["print"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(key_ctrl('w'));
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "\n");
}

/// Auto-pair skip-close (typing `)`/`"` when the cursor already sits on the
/// closer) moves the cursor with a motion, not an edit. Same as the arrow
/// keys above, it can't keep a live session's token tracked, so it
/// dismisses rather than leave a stale menu an Enter would fail to accept
/// against (`completion-accept!: insertText token does not contain the
/// cursor`).
#[test]
fn skip_close_dismisses_the_session_instead_of_leaving_a_stale_token() {
    let mut ed = editor_from("-[)]>\n");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(key(')')); // skip-close: moves past the pre-existing `)`
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), ")\n");
}

// ── Typing at the end of the token ──────────────────────────────────────────

/// With no word typed yet the token is empty and follows the cursor, so each
/// typed character narrows the list.
#[test]
fn typing_into_an_empty_token_narrows_the_list() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo", "grape", "grid"]);

    type_chars(&mut ed, "gr");

    assert_eq!(labels(&ed), vec!["grape", "grid"]);
}

/// A character outside the token's class typed at its end leaves the token.
#[test]
fn typing_a_non_word_char_after_a_word_dismisses_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "fo");
    open_completion_session(&mut ed, &["foo"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(key(' '));

    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "fo \n");
}

/// With no word typed yet, a character outside the token's class is still
/// outside it: the answer no longer applies.
#[test]
fn typing_a_non_word_char_into_an_empty_token_dismisses_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["foo"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(key(';'));

    assert!(ed.state.input.buffer_completion().is_none());
}

/// An auto-paired closer inserted at the token's end is not part of the
/// token: the cursor now sits inside the pair, past the token.
#[test]
fn an_auto_paired_closer_at_the_token_end_dismisses_the_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "foo");
    open_completion_session(&mut ed, &["foobar"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.feed_key(key('('));

    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "foo()\n");
}

// ── Regression: typing after accept must not desync the edit group ──────────

#[test]
fn typing_after_accept_composes_into_the_open_edit_group_without_panicking() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "DEFAULT_");
    open_completion_session(&mut ed, &["DEFAULT_WIDTH"]);

    ed.feed_key(key_enter());
    assert!(ed.state.input.buffer_completion().is_none());

    ed.feed_key(key(','));
    assert_eq!(ed.doc().text().to_string(), "DEFAULT_WIDTH,\n");

    ed.feed_key(key_esc());
    ed.feed_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "one undo reverts the whole insert session, accept included"
    );
}

// ── Out-of-band buffer changes are caught at settle ─────────────────────────

/// A `:e!` reload replaces the text with no change to carry the session's
/// tokens through. The next settle dismisses the session, and the render
/// fail-safe covers a frame drawn before it.
#[test]
fn a_stale_session_after_a_buffer_reload_is_dismissed_at_settle() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    for line in ["line0", "line1", "line2", "line3", "line4"] {
        type_chars(&mut ed, line);
        ed.feed_key(key_enter());
    }
    open_completion_session(&mut ed, &["candidate"]);
    assert!(
        ed.state
            .input
            .buffer_completion()
            .unwrap()
            .menu_anchor(ed.doc().text())
            .unwrap()
            .offset()
            > co(3),
        "sanity: the token is deep in the buffer"
    );

    let replacement = Buffer::at_start(BufferText::from("hi\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.state),
        ReplaceSource::Disk(Box::new(replacement)),
    );

    ed.settle();
    assert!(ed.state.input.buffer_completion().is_none());

    frame(&mut ed, 40, 8);
    assert!(ed.state.views.completion_menu.read().is_none());
}

/// A Steel edit above the token (a raw `apply-text-edits!`) leaves the
/// session open and shifts its token, so accepting still lands at the cursor.
#[test]
fn an_edit_above_the_token_from_a_script_keeps_the_session_and_shifts_its_token() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("a -[c]>def\n");
    super::super::lsp_bridge::setup_with(&mut ed, |backend, _sid| {
        // `apply-text-edits!` now only accepts a server-tagged wire edit
        // (via a real response); this canned response is what `:stash`
        // (dispatched below, before Insert mode) turns into one.
        backend.respond_to(
            "test/textEdits",
            serde_json::json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "Q"}]),
        );
    });
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test", &completion_labels(&["x"]), ""),
            r#"(define stashed-edits (box #f))
               (define-typed-command! "stash" "" (lambda (bid)
                 (lsp-request! bid "test/textEdits" (hash) (lambda (err res) (set-box! stashed-edits res)))))
               (define-command! "corrupt" "" (lambda (bid)
                 (apply-text-edits! bid (json-list (unbox stashed-edits)))))
               (define-command! "finish" "" (lambda () (completion-accept! 0)))"#
        ),
    );
    type_cmd(&mut ed, ":stash");
    ed.drain_lsp();
    ed.settle();

    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    ed.execute_keymap_command("corrupt".into(), None, false);
    ed.settle();
    assert!(ed.state.input.buffer_completion().is_some());
    ed.execute_keymap_command("finish".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "Qa xcdef\n");
}
