// The full completion flow: trigger (Ctrl+Space + server trigger chars) ->
// textDocument/completion -> completion-begin!; on-completion-accept applies
// additionalTextEdits or resolves; on-completion-refilter re-requests while
// isIncomplete. Named lsp_completion_feature.rs (not lsp_completion.rs — that
// file already covers the completion-begin!/update-filter!/top/
// accept!/dismiss! orchestration directly; this file drives the same
// primitives through the real shipped plugin and a real LSP round trip).
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};
use hume_scripting::ScriptingHost;

fn write_fixture_file(file_dir: &Path) -> PathBuf {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "foo\n").unwrap();
    file
}

/// Same plugin-before-handshake ordering as the signature-help setup: `on-lsp-attach`'s
/// handler (registers trigger chars) must already be installed when the
/// `Running` transition fires it, once, at attach time.
fn setup(
    file: &Path,
    tmp: &Path,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, RequestLog) {
    let guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": capabilities}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    // This harness's `eval_init` never loads `languages.scm` (unlike the
    // real `Editor::init_scripting` startup sequence), so `.rs` extension
    // detection never ran — set the language explicitly to match the
    // "rust" server key below, which on-lsp-attach's `server-name` arg
    // (the language) must equal for register-trigger-chars! to route here.
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);

    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    ed.drain_events(); // on-lsp-attach registers trigger chars

    (ed, guard, requests)
}

fn full_completion_caps() -> serde_json::Value {
    serde_json::json!({
        "completionProvider": {"triggerCharacters": ["."], "resolveProvider": true}
    })
}

fn settle(ed: &mut Editor) {
    ed.drain_events();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
}

fn request_count(requests: &RequestLog, method: &str) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == method)
        .count()
}

fn status(ed: &Editor) -> String {
    ed.state.status_msg.clone().unwrap_or_default()
}

#[test]
fn trigger_char_fires_the_completion_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        },
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

#[test]
fn ctrl_space_fires_completion_trigger() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        },
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

#[test]
fn capability_gated_no_completion_provider_sends_no_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        serde_json::json!({}),
        |_backend, _sid| {},
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert!(status(&ed).to_lowercase().contains("not supported"));
}

#[test]
fn null_response_opens_no_session() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::Value::Null);
        },
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        !status(&ed).to_lowercase().contains("error"),
        "a null response must be a clean no-op, not fall through to a type error \
         (json null decodes to Steel void, never #f), got status {:?}",
        status(&ed)
    );
    ed.feed_key(key_esc());

    // Directly exercise the session state the completion orchestration tests
    // already cover: no
    // active session means accept! must error.
    let source = r#"(define-command! "try-accept" "" (lambda () (completion-accept! 0)))"#;
    let mut host = ed.scripting.take().unwrap();
    eval_with_real_host(&mut ed, &mut host, source, tmp.path());
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":try-accept");

    assert!(
        status(&ed)
            .to_lowercase()
            .contains("no active completion session"),
        "a null response must never call completion-begin!, got status {:?}",
        status(&ed)
    );
}

#[test]
fn accept_applies_main_edit_and_additional_text_edits_as_one_undo_step() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Blank line 0 (the auto-import destination) + "foo" on line 1 (the
    // completion site) — non-overlapping, matching the real-world shape:
    // an import lands above the cursor's line, not at the exact same spot.
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "\nfoo\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!([{
                "label": "bar",
                "insertText": "bar",
                "additionalTextEdits": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                     "newText": "use std::bar;\n"}
                ]
            }]),
        );
        },
    );
    // Char 1 is the start of "foo" on line 1 (char 0 is line 0's newline).
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(1),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    // Enter is the real acceptance key (insert.rs's completion-menu
    // intercept) — accepts the currently-selected (default: index 0) item.
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "use std::bar;\n\nbarfoo\n",
        "the main edit (insertText at the anchor, char 1) and additionalTextEdits \
         (line 0, above it) must both land"
    );

    ed.feed_key(key_esc()); // no menu left open — a plain Insert-mode exit
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nfoo\n",
        "the main edit and additionalTextEdits both compose into the still-open \
         insert-session edit group, so one undo reverts the whole session"
    );
}

#[test]
fn typing_after_an_accept_with_additional_text_edits_composes_into_the_same_group() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "\nfoo\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "insertText": "bar",
                    "additionalTextEdits": [
                        {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                         "newText": "use std::bar;\n"}
                    ]
                }]),
            );
        },
    );
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(1),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    ed.feed_key(key_enter());
    settle(&mut ed);

    // The main edit and the additionalTextEdits both go through
    // `apply-text-edits!` (the same chokepoint) while the insert session's
    // edit group is open. The next keystroke must compose against their
    // combined result, not panic on a stale `ChangeSet::compose` length.
    ed.feed_key(key('X'));
    assert_eq!(
        ed.doc().text().to_string(),
        "use std::bar;\n\nbarXfoo\n",
        "typing right after the accept must land after the inserted completion text"
    );

    ed.feed_key(key_esc());
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nfoo\n",
        "one undo reverts the whole session: main edit + additionalTextEdits + typed char"
    );
}

#[test]
fn additional_edit_on_the_same_line_as_a_text_edit_main_edit_shifts_with_it() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // "foo.b XXX\n" — main edit replaces ".b" (chars 3..5) with ".bar",
    // shifting everything after it on the line by +2 UTF-16 units. The
    // additionalTextEdits entry (chars 6..9, "XXX") is on the same line,
    // entirely after the main edit's end — its position is stale unless
    // shifted by that same delta.
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "foo.b XXX\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "textEdit": {
                        "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 5}},
                        "newText": ".bar"
                    },
                    "additionalTextEdits": [
                        {"range": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 9}},
                         "newText": "YYY"}
                    ]
                }]),
            );
        },
    );
    // Char 5 is right after "foo.b" — matches the server's textEdit end
    // exactly, so accept() never extends the range past what's specified.
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(5),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "foo.bar YYY\n",
        "the additionalTextEdits range must shift by the main edit's UTF-16 \
         length delta since it lands on the same line, after the main edit"
    );
}

/// Same shape as `additional_edit_on_the_same_line_as_a_text_edit_main_edit_shifts_with_it`,
/// but with an astral-plane character (🎉, a UTF-16 surrogate pair — 2 wire
/// units, 1 char) before both edits on the line. The atomic-batch path
/// (`build_edit_changeset`) converts each edit's own wire position to a char
/// offset independently via `wire_to_char` — no UTF-16-delta arithmetic
/// between edits at all — so this proves that conversion is correct with an
/// astral prefix on the line, not just plain ASCII.
#[test]
fn additional_edit_on_the_same_line_with_an_astral_prefix_lands_correctly() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // "🎉foo.b XXX\n": 🎉 is char 0 (wire columns 0..2); "foo.b XXX" follows
    // at char 1 (wire column 2).
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "🎉foo.b XXX\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "textEdit": {
                        "range": {"start": {"line": 0, "character": 5}, "end": {"line": 0, "character": 7}},
                        "newText": ".bar"
                    },
                    "additionalTextEdits": [
                        {"range": {"start": {"line": 0, "character": 8}, "end": {"line": 0, "character": 11}},
                         "newText": "YYY"}
                    ]
                }]),
            );
        },
    );
    // Char 6: right after "🎉foo.b" (1 + 5 = 6) — matches the server's
    // textEdit end exactly.
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(6),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "🎉foo.bar YYY\n",
        "both edits must land at their exact wire-converted positions despite the \
         astral-plane prefix on the line"
    );
}

/// The resolve-path counterpart to
/// `additional_edit_on_the_same_line_as_a_text_edit_main_edit_shifts_with_it`
/// — same fixture and expected result, but the additionalTextEdits arrive
/// via `completionItem/resolve` instead of inline on the completion
/// response, exercising `edits::build_edits_from_earlier_document`'s
/// `ChangeSet::map_ranges` position tracking instead of the inline atomic
/// batch. Both land at the identical final text, proving the resolve path
/// is exact, not an approximation.
#[test]
fn resolved_additional_edits_land_through_the_accept_edit_on_the_same_line() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "foo.b XXX\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "textEdit": {
                        "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 5}},
                        "newText": ".bar"
                    }
                }]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({
                    "label": "bar",
                    "additionalTextEdits": [
                        {"range": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 9}},
                         "newText": "YYY"}
                    ]
                }),
            );
        },
    );
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(5),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    // `key_enter()` runs accept synchronously (main edit lands, resolve
    // request sent); the single `settle()` below drains the scripted
    // backend's already-queued response and runs the (plain Rust, not
    // Steel-queued) resolve callback inline — no second drain round needed,
    // unlike a Steel `lsp-request` callback which only queues on response.
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "foo.bar YYY\n",
        "a resolved additionalTextEdit on the same line as the main edit must land at \
         the exact position, mapped through the accept ChangeSet — not approximated \
         by a UTF-16 delta"
    );
}

/// The staleness half of the resolve contract: a resolve response arriving
/// after the user has typed more text must be dropped, not applied against
/// stale positions (same discipline `stale_check` already gives every other
/// `lsp-request`).
#[test]
fn resolved_additional_edits_are_dropped_after_a_post_accept_edit() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "\nfoo\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({
                    "label": "bar",
                    "additionalTextEdits": [
                        {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                         "newText": "use std::bar;\n"}
                    ]
                }),
            );
        },
    );
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(1),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    // `key_enter()`'s keybinding dispatch runs `accept_completion_selection`
    // synchronously — the main edit lands and the resolve request is *sent*
    // in this call, but its scripted response isn't drained until the next
    // `drain_lsp`. Typing `X` right here, before any drain, bumps text_gen
    // past what the resolve request's `stale_check` was armed with.
    ed.feed_key(key_enter());
    ed.feed_key(key('X'));

    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "\nbarXfoo\n",
        "a resolve response landing after further typing must be dropped — no \
         \"use std::bar;\" must appear, and the typed X must survive untouched"
    );
}

/// `:lsp-stop` sweeps every pending request via `LspClient::drain_pending`
/// (the same generic teardown every in-flight `lsp-request` gets) — a
/// resolve request in flight at stop time must not apply anything once its
/// swept `Outcome::TimedOut` reaches the callback, and must not panic.
#[test]
fn resolve_does_not_apply_anything_after_lsp_stop() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "\nfoo\n").unwrap();
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            // Deliberately no scripted reply for completionItem/resolve —
            // :lsp-stop must sweep it before any reply would matter.
        },
    );
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(1),
    );

    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    ed.feed_key(key_enter()); // main edit lands, resolve request sent

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "sanity: resolve must have been sent before the stop"
    );

    ed.lsp_stop(Some("rust")); // sweeps the in-flight resolve as TimedOut

    assert_eq!(
        ed.doc().text().to_string(),
        "\nbarfoo\n",
        "the main edit must stand, and the swept resolve must not apply anything \
         (no panic, no phantom edit)"
    );
}

#[test]
fn resolve_sent_only_when_item_lacks_additional_text_edits_and_resolve_provider_present() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({"label": "bar"}),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "an item with no additionalTextEdits, with resolveProvider present, must resolve"
    );
    let resolve_params = requests
        .borrow()
        .iter()
        .find(|(_, m, _)| m == "completionItem/resolve")
        .map(|(_, _, params)| params.clone())
        .expect("resolve request must be present");
    assert_eq!(
        resolve_params,
        serde_json::json!({"label": "bar", "insertText": "bar"}),
        "completionItem/resolve params must be the raw (pristine) accepted item, not a \
         Rust-projected subset"
    );
}

#[test]
fn null_resolve_response_is_a_clean_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to("completionItem/resolve", serde_json::Value::Null);
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "sanity: resolve must have been sent"
    );
    assert!(
        !status(&ed).to_lowercase().contains("error"),
        "a null resolve response must be a clean no-op, got status {:?}",
        status(&ed)
    );
}

#[test]
fn resolve_not_sent_when_the_item_already_has_additional_text_edits() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "insertText": "bar",
                    "additionalTextEdits": []
                }]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({"label": "bar"}),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "barfoo\n",
        "sanity: accept must actually have run (not a zero-effect pass)"
    );
    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        0,
        "an item that already carries additionalTextEdits (even empty) must not resolve"
    );
}

#[test]
fn refilter_on_incomplete_session_re_requests() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!({"isIncomplete": true, "items": [{"label": "foo", "insertText": "foo"}]}),
        );
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!({"isIncomplete": true, "items": [{"label": "foobar", "insertText": "foobar"}]}),
        );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        2,
        "typing while the session is isIncomplete must re-request"
    );
}

#[test]
fn refilter_on_complete_session_does_not_re_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "foo", "insertText": "foo"}]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        1,
        "a complete (non-isIncomplete) session must not re-request on further typing"
    );
}

/// Detach must be a true no-op, not a per-keystroke log:
/// `*completion-chars*`/`"lsp-completion"`'s trigger-char registration is
/// global, set once at attach, so `on-lsp-detach` must clear it — a trigger
/// char left registered past `:lsp-stop` would still reach
/// `lsp/guard-capability`, which resolves the focused buffer's own
/// (now-detached) server and logs "not supported by server" on every
/// matching keystroke.
#[test]
fn detach_clears_completion_trigger_chars_so_a_stale_trigger_is_a_true_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |_backend, _sid| {},
    );

    ed.lsp_stop(Some("rust"));
    ed.drain_events(); // on-lsp-detach clears *completion-chars*

    ed.feed_key(key('i'));
    ed.drain_events();
    let before = ed.state.status_msg.clone();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert_eq!(
        ed.state.status_msg, before,
        "a trigger char left registered past detach must be a true no-op, not a \
         guard-capability 'not supported' status message every keystroke"
    );
}

/// An open completion session's `items` are a snapshot already fetched from
/// the server, not a live subscription — but leaving it open after the
/// server stops would keep showing (and let the user accept) suggestions
/// from a server that's no longer running for this buffer.
#[test]
fn detach_dismisses_an_open_completion_session_for_that_buffer() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        ed.lsp.completion.is_some(),
        "sanity: a session must be open"
    );

    ed.lsp_stop(Some("rust"));

    assert!(
        ed.lsp.completion.is_none(),
        "an open completion session for the detached buffer must be dismissed, \
         not left showing stale items from a server that's no longer running"
    );
}

#[test]
fn snippet_item_lands_as_stripped_plain_text() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "for",
                    "insertText": "for ${1:x} in ${2:iter} {\n    $0\n}",
                    "insertTextFormat": 2
                }]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_events();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "for x in iter {\n    \n}foo\n",
        "snippet placeholders must be stripped to their default text before insertion"
    );
}
