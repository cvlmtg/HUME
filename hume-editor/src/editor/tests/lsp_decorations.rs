// Decoration stores (set-inlay-hints!,
// set-signs!, set-virtual-lines!, set-extra-highlights!) and the
// diagnostics pull (diagnostics-for-buffer, diagnostic-counts).

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

/// Attaches the focused buffer to a `Running` scripted server (UTF-16
/// encoding, the negotiated default) — inlay hints need a resolvable
/// server to convert their wire positions.
fn attach_running_server(ed: &mut Editor) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    // `flush_lsp_pending_changes` (and therefore the remap chokepoint)
    // bails out for a pathless buffer — a real attach always has one.
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from("/tmp/hume-decorations-test.rs")));
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    sid
}

#[test]
fn set_inlay_hints_converts_wire_position_using_utf16_encoding() {
    let tmp = tempfile::tempdir().unwrap();
    // "🎉" is 1 char, 2 UTF-16 code units, 4 UTF-8 bytes — a wire character
    // offset of 2 (the emoji's UTF-16 width) must land on char index 1, the
    // char right after it, not byte/char index 2 or 4.
    let mut ed = editor_from("-[x]>🎉bcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-hints-a" "" (lambda ()
             (set-inlay-hints! (current-buffer)
               (list (list (hash "line" 0 "character" 2) "hint" 'after)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");

    let hints = ed.state.decorations.inlay_hints_for(bid);
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].pos, 1, "wire char 2 (UTF-16) must land right after the emoji, at char index 1");
    assert_eq!(hints[0].text, "hint");
    assert!(!hints[0].before);
}

#[test]
fn set_inlay_hints_replaces_wholesale_not_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-hints-a" "" (lambda ()
             (set-inlay-hints! (current-buffer)
               (list (list (hash "line" 0 "character" 0) "first" 'before)))))
           (define-command! "arm-hints-b" "" (lambda ()
             (set-inlay-hints! (current-buffer)
               (list (list (hash "line" 0 "character" 1) "second" 'before)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");
    type_cmd(&mut ed, ":arm-hints-b");

    let hints = ed.state.decorations.inlay_hints_for(bid);
    assert_eq!(hints.len(), 1, "the second set-inlay-hints! must replace, not append");
    assert_eq!(hints[0].text, "second");
}

#[test]
fn inlay_hints_remap_through_an_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-hints-a" "" (lambda ()
             (set-inlay-hints! (current-buffer)
               (list (list (hash "line" 0 "character" 3) "hint" 'after)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");
    assert_eq!(ed.state.decorations.inlay_hints_for(bid)[0].pos, 3);

    // Insert two chars before the hint's position — the hint must move with
    // the text it annotates, not stay pinned to the old char index.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    ed.drain_lsp();

    assert_eq!(
        ed.state.decorations.inlay_hints_for(bid)[0].pos,
        5,
        "the hint must remap forward by the 2 inserted chars"
    );
}

#[test]
fn set_signs_virtual_lines_and_extra_highlights_round_trip_and_replace_per_source() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdef\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-hints-a" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "error" 10)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "added" 5)))
             (set-virtual-lines! "linter" (current-buffer) (list (list 0 "note: …")))
             (set-extra-highlights! "linter" (current-buffer) (list (list 0 3 "unused")))))
           (define-command! "clear-linter-signs" "" (lambda ()
             (set-signs! "linter" (current-buffer) '())))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");

    let linter_signs = ed.state.decorations.signs_for("linter", bid);
    assert_eq!(linter_signs.len(), 1);
    assert_eq!((linter_signs[0].line, linter_signs[0].text.as_str(), linter_signs[0].scope.as_str(), linter_signs[0].priority), (0, "!", "error", 10));

    let vcs_signs = ed.state.decorations.signs_for("vcs", bid);
    assert_eq!(vcs_signs.len(), 1, "different sources' signs for the same buffer must coexist");
    assert_eq!(vcs_signs[0].text, "+");

    let vlines = ed.state.decorations.virtual_lines_for("linter", bid);
    assert_eq!(vlines.len(), 1);
    assert_eq!(vlines[0].text, "note: …");

    let highlights = ed.state.decorations.extra_highlights_for("linter", bid);
    assert_eq!(highlights.len(), 1);
    assert_eq!((highlights[0].start, highlights[0].end, highlights[0].scope.as_str()), (0, 3, "unused"));

    // Replace semantics: a second set-signs! for the same source clears the first.
    type_cmd(&mut ed, ":clear-linter-signs");
    assert!(
        ed.state.decorations.signs_for("linter", bid).is_empty(),
        "an empty set-signs! must clear, not leave the previous entries"
    );
    assert_eq!(
        ed.state.decorations.signs_for("vcs", bid).len(),
        1,
        "clearing one source must not affect another source's signs"
    );
}

#[test]
fn diagnostics_for_buffer_and_diagnostic_counts_reflect_the_published_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdefghij\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({
                "uri": uri.as_str(),
                "diagnostics": [
                    {
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                        "severity": 1,
                        "message": "an error",
                    },
                    {
                        "range": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 3}},
                        "severity": 2,
                        "message": "a warning",
                    },
                    {
                        "range": {"start": {"line": 0, "character": 8}, "end": {"line": 0, "character": 9}},
                        "severity": 4,
                        "message": "a hint",
                    },
                ],
            }),
        },
    );

    let mut ed = editor_from("-[x]>\n");
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 1),
        "counts must tally exactly the one error and one warning"
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "counts" "" (lambda ()
             (let ((c (diagnostic-counts (current-buffer))))
               (log! 'info (to-string (car c) "." (cdr c))))))
           (define-command! "all" "" (lambda ()
             (log! 'info (to-string (length (diagnostics-for-buffer (current-buffer)))))))
           (define-command! "floored" "" (lambda ()
             (log! 'info (to-string (length (diagnostics-for-buffer (current-buffer) #:severity 'warning))))))
           (define-command! "ranged" "" (lambda ()
             (log! 'info (to-string (length (diagnostics-for-buffer (current-buffer) #:range (list 5 10)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":counts");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1 . 1", "(errors . warnings) must be a real dotted pair");

    type_cmd(&mut ed, ":all");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "3", "no floor/range must return all three");

    type_cmd(&mut ed, ":floored");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "2",
        "a warning floor keeps error+warning, drops the hint"
    );

    type_cmd(&mut ed, ":ranged");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "1",
        "range 5..10 must keep only the hint at char 8"
    );
}
