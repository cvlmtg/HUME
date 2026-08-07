// Introspection builtins: lsp-capabilities,
// lsp-server-status, lsp-server-for-buffer, buffer-generation,
// lsp-position-params, lsp-range-params, lsp-position->offset,
// lsp-range->offsets.

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Wires a scripted backend, drives its handshake to completion (so
/// `capabilities_json` gets cached the same way production does), and
/// attaches the focused buffer to it under language `"rust"`.
fn attach_running_server(ed: &mut Editor, initialize_result: serde_json::Value) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    sid
}

#[test]
fn lsp_capabilities_decodes_after_handshake() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_server(
        &mut ed,
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
    );

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (hash-ref (lsp-capabilities #f) "hoverProvider") #t)"#,
    );
    assert!(
        fired,
        "lsp-capabilities must decode the cached ServerCapabilities"
    );
}

#[test]
fn lsp_capabilities_is_false_before_running() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    // Client wired but handshake never driven — stays Starting.
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-capabilities #f) #f)"#,
    );
    assert!(
        fired,
        "capabilities must be #f before the handshake completes"
    );
}

#[test]
fn lsp_server_status_lists_the_running_server() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((entry (car (lsp-server-status))))
             (and (equal? (hash-ref entry "language") "rust")
                  (equal? (hash-ref entry "state") "Running")
                  (equal? (hash-ref entry "pending") 0)))"#,
    );
    assert!(
        fired,
        "lsp-server-status must list the running server correctly"
    );
}

#[test]
fn lsp_server_for_buffer_reflects_attachment() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-server-for-buffer (current-buffer)) "rust")"#,
    );
    assert!(
        fired,
        "lsp-server-for-buffer must return the attached language"
    );
}

#[test]
fn lsp_registered_for_language_reflects_registration() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '())"#,
        tmp.path(),
    );

    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(lsp-registered-for-language? "rust")"#,
    );
    assert!(
        fired,
        "lsp-registered-for-language? must be true once the language is registered"
    );
}

#[test]
fn lsp_registered_for_language_is_false_when_unregistered() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(not (lsp-registered-for-language? "rust"))"#,
    );
    assert!(
        fired,
        "lsp-registered-for-language? must be false when nothing is registered"
    );
}

#[test]
fn buffer_generation_changes_after_an_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "snap" "" (lambda () (log! 'info (to-string (buffer-generation (current-buffer))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":snap");
    let before_gen = ed
        .state
        .status_msg
        .clone()
        .expect("log! set the status message");

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    type_cmd(&mut ed, ":snap");
    let after_gen = ed
        .state
        .status_msg
        .clone()
        .expect("log! set the status message");

    assert_ne!(
        before_gen, after_gen,
        "buffer-generation must change after a mutation"
    );
}

#[test]
fn lsp_position_params_uses_the_negotiated_utf16_encoding_for_multibyte_chars() {
    let tmp = safe_tempdir();
    // Buffer: "🎉" (char 0, one grapheme, 2 UTF-16 code units) then cursor on 'x' (char 1).
    let mut ed = editor_from("🎉-[x]>rest\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-introspect.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}})); // UTF-16 default

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((p (lsp-position-params (current-buffer))))
             (and p
                  (equal? (hash-ref (hash-ref p "position") "line") 0)
                  (equal? (hash-ref (hash-ref p "position") "character") 2)))"#,
    );
    assert!(
        fired,
        "UTF-16 negotiated: 🎉 is a surrogate pair, so char index 1 must be wire character 2"
    );
}

#[test]
fn lsp_position_params_uses_the_negotiated_utf8_encoding_for_multibyte_chars() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("🎉-[x]>rest\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-introspect-utf8.rs")));
    attach_running_server(
        &mut ed,
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
    );

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((p (lsp-position-params (current-buffer))))
             (and p
                  (equal? (hash-ref (hash-ref p "position") "character") 4)))"#,
    );
    assert!(
        fired,
        "UTF-8 negotiated: 🎉 is 4 bytes, so char index 1 must be wire character 4"
    );
}

#[test]
fn lsp_range_params_reflects_the_primary_selection() {
    let tmp = safe_tempdir();
    // Selection covers "bcd" (chars 1..=3, inclusive head at 3): half-open
    // wire range must be [1, 4).
    let mut ed = editor_from("a<[bcd]-ef\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-introspect-range.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let* ((p (lsp-range-params (current-buffer)))
                  (r (hash-ref p "range")))
             (and (equal? (hash-ref (hash-ref r "start") "character") 1)
                  (equal? (hash-ref (hash-ref r "end") "character") 4)))"#,
    );
    assert!(
        fired,
        "range params must span the primary selection, half-open"
    );
}

/// L3 regression: the wire range's `end` must land after a full grapheme
/// cluster, never mid-cluster. `char_to_wire(rope, end_c + 1, ..)` (a raw
/// `+ 1`) would split `é` (`e` + U+0301, two chars, one cluster) if the
/// selection's inclusive `head` sits on the cluster's first char.
#[test]
fn lsp_range_params_end_lands_on_a_grapheme_boundary_not_mid_cluster() {
    use hume_editing::selection::Selection;

    let tmp = safe_tempdir();
    // "caf" + é (U+0065 U+0301, two chars) + "\n". Grapheme boundaries:
    // 0,1,2,3,5,6 — é occupies chars 3..5. Selection anchor=0, head=3
    // (inclusive) covers "caf" plus é's first char only.
    let content = "caf\u{0065}\u{0301}\n";
    let mut ed = Editor::for_testing(Buffer::new(Text::from(content), SelectionSet::default()));
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-range-grapheme.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let bid = ed.focused_buffer_id();
    let focused = ed.state.focused_pane_id;
    ed.state.panes.state[focused][bid].selections = SelectionSet::single(Selection::new(0, 3));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let* ((p (lsp-range-params (current-buffer)))
                  (r (hash-ref p "range")))
             (equal? (hash-ref (hash-ref r "end") "character") 5))"#,
    );
    assert!(
        fired,
        "end must land after the full é cluster (char 5), not mid-cluster (char 4)"
    );
}

#[test]
fn viewport_range_matches_the_on_viewport_change_hooks_own_computation() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    // Captures the hook's own `(first . last)` payload so the assertion
    // compares two independently-reached values, not the builtin against
    // itself — both paths share `introspect::pane_visible_range`, so this
    // pins that they stay in sync, not just that the builtin returns
    // *something*.
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define *captured* #f)
           (register-hook! 'on-viewport-change
             (lambda (bid first last) (set! *captured* (cons first last))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let pid = ed.state.focused_pane_id;
    ed.queue_viewport_change(pid);
    ed.settle();

    let host = ed.scripting.take().unwrap();
    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(equal? *captured* (viewport-range (current-buffer)))"#,
    );
    assert!(
        fired,
        "viewport-range must agree with the on-viewport-change hook's own \
         computation for the same pane"
    );
}

#[test]
fn viewport_range_is_false_for_a_buffer_not_shown_in_any_pane() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");

    // `open_extra_files` opens a second buffer into the buffer list without
    // switching any pane to show it — it stays paneless.
    let extra = tmp.path().join("hidden.rs");
    std::fs::write(&extra, "fn hidden() {}\n").unwrap();
    ed.open_extra_files(std::slice::from_ref(&extra));
    let hidden_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    assert_ne!(
        hidden_bid,
        ed.focused_buffer_id(),
        "test setup: the extra buffer must not be focused"
    );

    // Only two buffers exist, so "the one that isn't the focused buffer"
    // unambiguously picks out the hidden one — relies on R4's equal?/hash
    // fix (`equality_hint`) for buffer-id comparison across independently
    // decoded `(buffers)` entries.
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? b (current-buffer)))) (buffers)))))
             (equal? (viewport-range hidden) #f))"#,
    );
    assert!(
        fired,
        "a buffer not shown in any pane must yield #f from viewport-range"
    );
}

#[test]
fn lsp_position_params_is_false_for_an_unattached_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    // No server attached at all.
    let host = ScriptingHost::new();
    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(equal? (lsp-position-params (current-buffer)) #f)"#,
    );
    assert!(fired, "no attached server must yield #f, not an error");
}

#[test]
fn lsp_position_to_offset_uses_the_negotiated_utf16_encoding() {
    let tmp = safe_tempdir();
    // "🎉" is 1 char, 2 UTF-16 code units — wire character 2 (the emoji's
    // full UTF-16 width) must land on char index 1, the char right after it.
    let mut ed = editor_from("-[x]>🎉rest\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-position-to-offset.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}})); // UTF-16 default

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-position->offset (current-buffer)
                     (hash "line" 0 "character" 2))
                   1)"#,
    );
    assert!(
        fired,
        "UTF-16 negotiated: wire character 2 must land right after the emoji, at char index 1"
    );
}

#[test]
fn lsp_position_to_offset_uses_the_negotiated_utf8_encoding() {
    let tmp = safe_tempdir();
    // "🎉" is 4 UTF-8 bytes — wire character 4 must land on char index 1.
    let mut ed = editor_from("-[x]>🎉rest\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-position-to-offset-utf8.rs")));
    attach_running_server(
        &mut ed,
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
    );

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-position->offset (current-buffer)
                     (hash "line" 0 "character" 4))
                   1)"#,
    );
    assert!(
        fired,
        "UTF-8 negotiated: wire character 4 must land right after the 4-byte emoji, at char index 1"
    );
}

#[test]
fn lsp_position_to_offset_is_false_for_an_unattached_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    // No server attached at all — no negotiated encoding to convert with.
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-position->offset (current-buffer) (hash "line" 0 "character" 0)) #f)"#,
    );
    assert!(fired, "no attached server must yield #f, not a guess");
}

#[test]
fn lsp_position_to_offset_is_false_when_it_would_land_on_the_trailing_phantom_line() {
    let tmp = safe_tempdir();
    // "-[x]>abc\n" is "xabc\n" — one content line; a wire `line` past it
    // clamps (inside `wire_to_char`) onto the buffer's trailing phantom line
    // rather than erroring — servers send past-end positions routinely. Every
    // point-anchored decoration setter (`set-inlay-hints!`) rejects that
    // offset outright, so `lsp-position->offset` must refuse here too,
    // rather than handing back a value only useful for failing one step
    // later (and, inside a hint batch, failing every *other* hint with it).
    let mut ed = editor_from("-[x]>abc\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-position-phantom-line.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-position->offset (current-buffer)
                     (hash "line" 5 "character" 0))
                   #f)"#,
    );
    assert!(
        fired,
        "a wire line past the buffer's content must yield #f, not the phantom line's offset"
    );
}

#[test]
fn lsp_range_to_offsets_converts_both_endpoints_half_open() {
    let tmp = safe_tempdir();
    // "🎉" occupies char 0 (2 UTF-16 code units); 'b' is char 1, wire
    // character 2. A wire range [0, 2) must convert to char offsets (0 . 1)
    // — covering just the emoji, half-open.
    let mut ed = editor_from("-[x]>🎉bcdef\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-range-to-offsets.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}})); // UTF-16 default

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-range->offsets (current-buffer)
                     (hash "start" (hash "line" 0 "character" 0)
                           "end" (hash "line" 0 "character" 2)))
                   (cons 0 1))"#,
    );
    assert!(
        fired,
        "wire range [0, 2) (UTF-16) must convert to half-open char offsets (0 . 1)"
    );
}

#[test]
fn lsp_range_to_offsets_end_may_land_at_the_buffers_char_length() {
    // Deliberately the opposite of `lsp_position_to_offset_is_false_when_it_
    // would_land_on_the_trailing_phantom_line`: a range's `end` legitimately
    // sits at the buffer's char length (`set-extra-highlights!`'s
    // `validate_range` accepts that boundary), so `lsp-range->offsets` must
    // keep the clamping behavior `lsp-position->offset` deliberately
    // refuses — a past-end wire `line` for `end` is not an error here.
    // "-[x]>abc\n" is "xabc\n" (the marked 'x' is real buffer content) — 5
    // chars.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abc\n");
    ed.doc_mut()
        .set_path(Some(tmp.path().join("fake-lsp-range-end-at-length.rs")));
    attach_running_server(&mut ed, serde_json::json!({"capabilities": {}}));

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-range->offsets (current-buffer)
                     (hash "start" (hash "line" 0 "character" 0)
                           "end" (hash "line" 5 "character" 0)))
                   (cons 0 5))"#,
    );
    assert!(
        fired,
        "a past-end wire `line` for `end` must clamp to the buffer's char length, not #f"
    );
}

#[test]
fn lsp_range_to_offsets_is_false_for_an_unattached_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-range->offsets (current-buffer)
                     (hash "start" (hash "line" 0 "character" 0)
                           "end" (hash "line" 0 "character" 1)))
                   #f)"#,
    );
    assert!(fired, "no attached server must yield #f, not a guess");
}
