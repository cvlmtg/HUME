// Decoration stores (set-inlay-hints!,
// set-signs!, set-virtual-lines!, set-extra-highlights!, set-eol-text!) and
// the diagnostics pull (diagnostics-for-buffer, diagnostic-counts).

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Attaches the focused buffer to a `Running` scripted server (UTF-16
/// encoding, the negotiated default) and gives it a path: several tests
/// below compose `lsp-position->offset`, which needs a resolvable server to
/// convert a wire position, and `inlay_hints_remap_through_an_edit` needs
/// the path for the remap chokepoint to have somewhere to (not) send a
/// `didChange`.
fn attach_running_server(ed: &mut Editor) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    // `flush_lsp_pending_changes` (and therefore the remap chokepoint)
    // bails out for a pathless buffer; a real attach always has one.
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from(
            "/tmp/hume-decorations-test.rs",
        )));
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    sid
}

#[test]
fn set_inlay_hints_composes_with_lsp_position_to_offset() {
    let tmp = safe_tempdir();
    // "🎉" is 1 char, 2 UTF-16 code units, 4 UTF-8 bytes: a wire character
    // offset of 2 (the emoji's UTF-16 width) must land on char index 1, the
    // char right after it, not byte/char index 2 or 4. `set-inlay-hints!`
    // does not decode wire positions itself: a plugin composes
    // `lsp-position->offset` before calling the setter.
    //
    // The position must arrive as a real tagged handle (`lsp-position->
    // offset` rejects a hand-built (untagged) one outright), so this echoes
    // it through a scripted `test/echo` request/response round trip rather
    // than reusing `attach_running_server`, which sets up no such response.
    let mut ed = editor_from("-[x]>🎉bcdef\n");
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    backend.respond_to("test/echo", serde_json::json!({"line": 0, "character": 2}));
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from(
            "/tmp/hume-decorations-test.rs",
        )));
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-hints-a" "" (lambda (bid)
             (lsp-request! bid "test/echo" (hash) (lambda (err res)
               (set-inlay-hints! "linter" bid
                 (list (hash 'offset (lsp-position->offset bid res) 'text "hint" 'side 'after)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");
    ed.drain_lsp();
    ed.settle();

    let hints: Vec<_> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .collect();
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].pos,
        co(1),
        "wire char 2 (UTF-16) must land right after the emoji, at char index 1"
    );
    assert_eq!(hints[0].text, "hint");
    assert!(!hints[0].before);
}

#[test]
fn set_inlay_hints_replaces_wholesale_not_appends() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-hints-a" "" (lambda (bid)
             (set-inlay-hints! "linter" bid
               (list (hash 'offset 0 'text "first" 'side 'before)))))
           (define-typed-command! "arm-hints-b" "" (lambda (bid)
             (set-inlay-hints! "linter" bid
               (list (hash 'offset 1 'text "second" 'side 'before)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");
    type_cmd(&mut ed, ":arm-hints-b");

    let hints: Vec<_> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .collect();
    assert_eq!(
        hints.len(),
        1,
        "the second set-inlay-hints! must replace, not append"
    );
    assert_eq!(hints[0].text, "second");
}

/// A malformed offset (non-integer, or negative) must error loudly at the
/// `set-inlay-hints!` boundary rather than being silently dropped. Silent
/// extraction would leave a plugin author's typo producing fewer hints with
/// no explanation.
#[test]
fn set_inlay_hints_errors_loudly_on_a_malformed_offset() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-bad" "" (lambda (bid)
             (set-inlay-hints! "linter" bid
               (list (hash 'offset "not-a-number" 'text "oops" 'side 'before)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-bad");

    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_none(),
        "a malformed entry must not land in the store at all"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("offset"),
        "must report which field is invalid: {log:?}"
    );
}

/// An `'after` hint anchored on the buffer's trailing structural `\n` must
/// error loudly at `set-inlay-hints!`, not store an entry the render bridge
/// (`update_inlay_hint_providers`) can only resolve onto the unrendered
/// trailing phantom line: the end-to-end companion to
/// `host_impl::tests::validate_offset_rejects_after_on_the_trailing_newline`,
/// exercised here through the actual `set-inlay-hints!` builtin rather than
/// the bare validation function.
#[test]
fn set_inlay_hints_errors_loudly_on_an_after_hint_at_the_trailing_newline() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n"); // "xabcdef\n", 8 chars; offset 7 is the trailing '\n'
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-bad" "" (lambda (bid)
             (set-inlay-hints! "linter" bid
               (list (hash 'offset 7 'text "hint" 'side 'after)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-bad");

    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_none(),
        "an 'after hint at the trailing newline must not land in the store"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("trailing"),
        "must explain why this offset is rejected: {log:?}"
    );
}

/// An out-of-range char offset must error loudly at `set-extra-highlights!`
/// rather than storing a span that never renders: the fail-fast contract
/// every kind's host-boundary conversion holds to.
#[test]
fn set_extra_highlights_errors_loudly_on_an_out_of_range_end() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n"); // 8 chars total
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-bad" "" (lambda (bid)
             (set-extra-highlights! "linter" bid (list (hash 'start 0 'end 100 'scope "unused")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-bad");

    assert!(
        ed.state
            .config
            .decorations
            .extra_highlights_for_buffer(bid)
            .next()
            .is_none(),
        "a malformed entry must not land in the store at all"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("100") && log.contains("set-extra-highlights!"),
        "must name the builtin and the offending value: {log:?}"
    );
}

/// An out-of-range `line` must error loudly at the boundary shared by
/// signs/virtual-lines/EOL-text/line-backgrounds, rather than leaving the
/// entry to never render.
#[test]
fn set_signs_set_virtual_lines_set_eol_text_and_set_line_backgrounds_error_loudly_on_an_out_of_range_line()
 {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n"); // one real line
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-signs" "" (lambda (bid)
             (register-sign-source! "linter" bid 10)
             (set-signs! "linter" bid (list (hash 'line 99 'text "!" 'scope "error")))))
           (define-typed-command! "arm-vlines" "" (lambda (bid)
             (set-virtual-lines! "git-diff" bid (list (hash 'line 99 'text "note")))))
           (define-typed-command! "arm-eol" "" (lambda (bid)
             (set-eol-text! "diagnostics" bid (list (hash 'line 99 'text "msg" 'scope "diagnostic.error")))))
           (define-typed-command! "arm-linebg" "" (lambda (bid)
             (set-line-backgrounds! "git-diff" bid (list (hash 'line 99 'scope "diff.plus")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":arm-signs");
    assert!(
        ed.state
            .config
            .decorations
            .signs_for("linter", bid)
            .is_empty(),
        "set-signs! must not store an entry for an out-of-range line"
    );

    type_cmd(&mut ed, ":arm-vlines");
    assert!(
        ed.state
            .config
            .decorations
            .virtual_lines_for("git-diff", bid)
            .is_empty(),
        "set-virtual-lines! must not store an entry for an out-of-range line"
    );

    type_cmd(&mut ed, ":arm-eol");
    assert!(
        ed.state
            .config
            .decorations
            .eol_text_for_buffer(bid)
            .next()
            .is_none(),
        "set-eol-text! must not store an entry for an out-of-range line"
    );

    type_cmd(&mut ed, ":arm-linebg");
    assert!(
        ed.state
            .config
            .decorations
            .line_backgrounds_for("git-diff", bid)
            .is_empty(),
        "set-line-backgrounds! must not store an entry for an out-of-range line"
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("set-signs!")
            && log.contains("set-virtual-lines!")
            && log.contains("set-eol-text!")
            && log.contains("set-line-backgrounds!"),
        "each builtin must report its own out-of-range line loudly: {log:?}"
    );
}

#[test]
fn inlay_hints_remap_through_an_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    attach_running_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-hints-a" "" (lambda (bid)
             (set-inlay-hints! "linter" bid
               (list (hash 'offset 3 'text "hint" 'side 'after)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");
    let hint_pos = |ed: &Editor| {
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .unwrap()
            .pos
    };
    assert_eq!(hint_pos(&ed), co(3));

    // Insert two chars before the hint's position; the hint must move with
    // the text it annotates, not stay pinned to the old char index.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    ed.drain_lsp();

    assert_eq!(
        hint_pos(&ed),
        co(5),
        "the hint must remap forward by the 2 inserted chars"
    );
}

/// Regression: decorations are not LSP-owned. LSP is just their first
/// client (any plugin can call `set-extra-highlights!`/`set-inlay-hints!`
/// on any buffer). `record_lsp_edits` must queue a buffer's edits for the
/// remap chokepoint whether or not an LSP server is attached, or a buffer
/// with decorations but no server drifts out of position on every edit.
#[test]
fn extra_highlights_remap_through_an_edit_on_a_buffer_with_no_lsp_server() {
    let tmp = safe_tempdir();
    // No attach_running_server call: this buffer has no LSP
    // server and no path, nothing but the decoration itself.
    let mut ed = editor_from("-[x]>abcdef\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-extra-highlights! "linter" bid (list (hash 'start 3 'end 5 'scope "unused")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let before: Vec<_> = ed
        .state
        .config
        .decorations
        .extra_highlights_for_buffer(bid)
        .map(|e| (e.start, e.end))
        .collect();
    assert_eq!(
        before,
        vec![(co(3), co(5))],
        "seed highlight must land before the edit"
    );

    // Insert two chars before the highlight's start.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    ed.drain_lsp();

    let after: Vec<_> = ed
        .state
        .config
        .decorations
        .extra_highlights_for_buffer(bid)
        .map(|e| (e.start, e.end))
        .collect();
    assert_eq!(
        after,
        vec![(co(5), co(7))],
        "the highlight must remap forward by the 2 inserted chars, even with no attached LSP server"
    );
}

/// `signs`/`virtual_lines`/`eol_text` remap through edits like every other
/// kind. A line-indexed sign that never remapped would silently drift onto
/// the wrong line the moment a line was inserted or deleted above it.
/// No `attach_running_server` call: `has_any`
/// covers every kind, so a signs-only buffer with no LSP server still
/// gets its edits queued for the remap chokepoint.
#[test]
fn sign_remaps_through_a_line_inserted_above_it() {
    let tmp = safe_tempdir();
    // "xaaaa\nbbbb\ncccc\n": the sign below sits on "bbbb\n", line 1.
    let mut ed = editor_from("-[x]>aaaa\nbbbb\ncccc\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (register-sign-source! "linter" bid 10)
             (set-signs! "linter" bid (list (hash 'line 1 'text "!" 'scope "error")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let sign_line = |ed: &Editor| {
        let pos = ed.state.config.decorations.signs_for("linter", bid)[0].pos;
        ed.state.buffers.get(bid).text().char_to_line(pos)
    };
    assert_eq!(
        sign_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "sanity: sign starts on line 1"
    );

    // Insert a whole new blank line above line 0: "bbbb" (and the sign on
    // it) must shift from line 1 to line 2.
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());
    ed.drain_lsp();

    assert_eq!(
        sign_line(&ed),
        hume_rope::line::ContentLine::new(2),
        "the sign must remap forward with the line it annotates"
    );
}

/// Same drift regression as the sign test above, for the other two
/// line-anchored kinds together.
#[test]
fn virtual_line_and_eol_text_remap_through_a_line_inserted_above_them() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>aaaa\nbbbb\ncccc\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-virtual-lines! "git-diff" bid (list (hash 'line 1 'text "note")))
             (set-eol-text! "diagnostics" bid (list (hash 'line 1 'text "msg" 'scope "diagnostic.error")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let vline_line = |ed: &Editor| {
        let pos = ed
            .state
            .config
            .decorations
            .virtual_lines_for("git-diff", bid)[0]
            .pos;
        ed.state.buffers.get(bid).text().char_to_line(pos)
    };
    let eol_line = |ed: &Editor| {
        let pos = ed
            .state
            .config
            .decorations
            .eol_text_for_buffer(bid)
            .next()
            .unwrap()
            .1
            .pos;
        ed.state.buffers.get(bid).text().char_to_line(pos)
    };
    assert_eq!(
        vline_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "sanity: virtual line starts on line 1"
    );
    assert_eq!(
        eol_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "sanity: EOL text starts on line 1"
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());
    ed.drain_lsp();

    assert_eq!(
        vline_line(&ed),
        hume_rope::line::ContentLine::new(2),
        "the virtual line must remap forward with the line it annotates"
    );
    assert_eq!(
        eol_line(&ed),
        hume_rope::line::ContentLine::new(2),
        "the EOL text must remap forward with the line it annotates"
    );
}

/// Line-anchored kinds remap with `Assoc::After`, not
/// `Assoc::Before`. An "open line above" edit (a newline inserted exactly
/// at the decorated line's line-start offset) must leave the decoration on
/// the original line's content, now one line further down, not stranded on
/// the newly inserted blank line.
#[test]
fn line_anchored_decoration_follows_its_content_past_an_open_line_above_it() {
    let tmp = safe_tempdir();
    // "xaaaa\n" is line 0 (chars 0..6); "bbbb\n" is line 1, starting
    // exactly at char 6, and the insertion below lands exactly there.
    let mut ed = editor_from("-[x]>aaaa\nbbbb\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (register-sign-source! "linter" bid 10)
             (set-signs! "linter" bid (list (hash 'line 1 'text "!" 'scope "error")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");
    assert_eq!(
        ed.state.config.decorations.signs_for("linter", bid)[0].pos,
        co(6),
        "sanity: line 1's line-start char offset is 6"
    );

    // Move the cursor onto "bbbb"'s first char (char 6) and open a blank
    // line exactly there: `i` + Enter inserts "\n" at position 6 without
    // touching anything before or after it.
    select(&mut ed, &[(6, 6)], 0);
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());
    ed.drain_lsp();

    let pos = ed.state.config.decorations.signs_for("linter", bid)[0].pos;
    let text = ed.state.buffers.get(bid).text();
    assert_eq!(
        pos,
        co(7),
        "Assoc::After must land the sign just past the inserted newline, at \
         \"bbbb\"'s new position; Assoc::Before would leave it at 6, on the \
         new blank line instead"
    );
    assert_eq!(
        text.char_to_line(pos),
        hume_rope::line::ContentLine::new(2),
        "the sign must render on line 2, where \"bbbb\" now is, not line 1's new blank line"
    );
}

#[test]
fn set_signs_virtual_lines_and_extra_highlights_round_trip_and_replace_per_source() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-hints-a" "" (lambda (bid)
             (register-sign-source! "linter" bid 10)
             (register-sign-source! "vcs" bid 5)
             (set-signs! "linter" bid (list (hash 'line 0 'text "!" 'scope "error")))
             (set-signs! "vcs" bid (list (hash 'line 0 'text "+" 'scope "added")))
             (set-virtual-lines! "linter" bid (list (hash 'line 0 'text "note: …")))
             (set-extra-highlights! "linter" bid (list (hash 'start 0 'end 3 'scope "unused")))))
           (define-typed-command! "clear-linter-signs" "" (lambda (bid)
             (set-signs! "linter" bid '())))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-hints-a");

    let linter_signs = ed.state.config.decorations.signs_for("linter", bid);
    assert_eq!(linter_signs.len(), 1);
    assert_eq!(
        (
            linter_signs[0].pos,
            &*linter_signs[0].text,
            scope_name(&ed, linter_signs[0].scope),
        ),
        (co(0), "!", "error"),
        "line 0's line-start char offset is 0 on this fixture"
    );

    let vcs_signs = ed.state.config.decorations.signs_for("vcs", bid);
    assert_eq!(
        vcs_signs.len(),
        1,
        "different sources' signs for the same buffer must coexist"
    );
    assert_eq!(&*vcs_signs[0].text, "+");

    let vlines = ed.state.config.decorations.virtual_lines_for("linter", bid);
    assert_eq!(vlines.len(), 1);
    assert_eq!(vlines[0].text, "note: …");

    let highlights = ed
        .state
        .config
        .decorations
        .extra_highlights_for("linter", bid);
    assert_eq!(highlights.len(), 1);
    assert_eq!(
        (
            highlights[0].start,
            highlights[0].end,
            scope_name(&ed, highlights[0].scope)
        ),
        (co(0), co(3), "unused")
    );

    // Replace semantics: a second set-signs! for the same source clears the first.
    type_cmd(&mut ed, ":clear-linter-signs");
    assert!(
        ed.state
            .config
            .decorations
            .signs_for("linter", bid)
            .is_empty(),
        "an empty set-signs! must clear, not leave the previous entries"
    );
    assert_eq!(
        ed.state.config.decorations.signs_for("vcs", bid).len(),
        1,
        "clearing one source must not affect another source's signs"
    );
}

/// Every `set-*!` decoration setter must intern its scope name at the call
/// itself, not lazily, the first time a render bridge happens to resolve
/// it. Asserted with no `prepare_frame` anywhere in this test: if a scope
/// were still resolved by a render bridge, the registry would not yet know
/// its name at this point.
#[test]
fn every_setter_interns_its_scope_at_the_set_call_not_at_first_render() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (register-sign-source! "src" bid 10)
             (set-signs! "src" bid (list (hash 'line 0 'text "!" 'scope "scope-sign")))
             (set-virtual-lines! "src" bid
               (list (hash 'line 0 'text "x" 'scope "scope-vline")))
             (set-eol-text! "src" bid (list (hash 'line 0 'text "x" 'scope "scope-eol")))
             (set-line-backgrounds! "src" bid (list (hash 'line 0 'scope "scope-linebg")))
             (set-extra-highlights! "src" bid (list (hash 'start 0 'end 3 'scope "scope-extra")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    for name in [
        "scope-sign",
        "scope-vline",
        "scope-eol",
        "scope-linebg",
        "scope-extra",
    ] {
        assert!(
            ed.view.registry.get(name).is_some(),
            "{name} must already be interned by its set-*! call, before any prepare_frame runs"
        );
    }
}

#[test]
fn set_virtual_lines_anchor_scope_and_segments_round_trip_into_the_store() {
    let tmp = safe_tempdir();
    // `-[x]>` puts the 1-char cursor marker "x" at the very start, so line 0
    // is "xaaaa\n" (6 chars) and lines 1-3 are 5 chars each ("bbbb\n" etc.):
    // line 3 (0-indexed) is in range, its line-start char offset is
    // 6 + 5 + 5 = 16.
    let mut ed = editor_from("-[x]>aaaa\nbbbb\ncccc\ndddd\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-virtual-lines! "git-diff" bid
               (list (hash 'line 3 'anchor 'before 'text "- let x = 5" 'scope "diff.minus"
                           'segments (list (hash 'start 2 'end 5 'scope "keyword")))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let vlines = ed
        .state
        .config
        .decorations
        .virtual_lines_for("git-diff", bid);
    assert_eq!(vlines.len(), 1);
    assert_eq!(
        vlines[0].pos,
        co(16),
        "'line 3's line-start char offset on this fixture"
    );
    assert_eq!(vlines[0].text, "- let x = 5");
    assert!(vlines[0].before, "'anchor 'before must set before: true");
    assert_eq!(scope_name(&ed, vlines[0].scope), "diff.minus");
    assert_eq!(
        vlines[0].segments,
        vec![(bc(2), bc(5), scope(&ed, "keyword"))],
        "'segments' are char offsets at the Steel surface; on this ASCII fixture the host \
         boundary's char\u{2192}byte conversion (validated there, not at the Steel boundary) \
         is a no-op, so they reach the store unchanged"
    );
}

#[test]
fn set_eol_text_round_trips_and_replaces_per_source() {
    let tmp = safe_tempdir();
    // `-[x]>` puts the 1-char cursor marker "x" at the very start, so line 0
    // is "xabcdef\n" (8 chars) and line 1 ("ghijkl\n") starts at char 8.
    let mut ed = editor_from("-[x]>abcdef\nghijkl\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-a" "" (lambda (bid)
             (set-eol-text! "diagnostics" bid
               (list (hash 'line 0 'text "[2] first problem" 'scope "diagnostic.error")))))
           (define-typed-command! "arm-b" "" (lambda (bid)
             (set-eol-text! "diagnostics" bid
               (list (hash 'line 1 'text "second problem" 'scope "diagnostic.warning")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-a");

    let entries: Vec<_> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].1.pos,
        co(0),
        "line 0's line-start char offset is 0"
    );
    assert_eq!(entries[0].1.text, "[2] first problem");
    assert_eq!(scope_name(&ed, entries[0].1.scope), "diagnostic.error");

    // A second call for the same source must replace wholesale, not append.
    type_cmd(&mut ed, ":arm-b");
    let entries: Vec<_> = ed
        .state
        .config
        .decorations
        .eol_text_for_buffer(bid)
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "the second set-eol-text! must replace, not append"
    );
    assert_eq!(
        entries[0].1.pos,
        co(8),
        "line 1's line-start char offset on this fixture (\"xabcdef\\n\" is 8 chars)"
    );
    assert_eq!(entries[0].1.text, "second problem");
    assert_eq!(scope_name(&ed, entries[0].1.scope), "diagnostic.warning");
}

#[test]
fn set_eol_text_hide_on_insert_line_defaults_off_and_round_trips() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdef\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-plain" "" (lambda (bid)
             (set-eol-text! "s" bid
               (list (hash 'line 0 'text "t" 'scope "diagnostic.error")))))
           (define-typed-command! "arm-hidden" "" (lambda (bid)
             (set-eol-text! "s" bid
               (list (hash 'line 0 'text "t" 'scope "diagnostic.error"))
               #:hide-on-insert-line #t)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    let hide_flags = |ed: &Editor| -> Vec<bool> {
        ed.state
            .config
            .decorations
            .eol_text_for_buffer(bid)
            .map(|(_, e)| e.hide_on_insert_line)
            .collect()
    };
    type_cmd(&mut ed, ":arm-plain");
    assert_eq!(hide_flags(&ed), vec![false]);
    type_cmd(&mut ed, ":arm-hidden");
    assert_eq!(hide_flags(&ed), vec![true]);
}

#[test]
fn set_line_backgrounds_round_trips_and_replaces_per_source() {
    let tmp = safe_tempdir();
    // "xabcdef\n" is line 0 (8 chars); "ghijkl\n" (line 1) starts at char 8.
    let mut ed = editor_from("-[x]>abcdef\nghijkl\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm-a" "" (lambda (bid)
             (set-line-backgrounds! "git-diff" bid (list (hash 'line 0 'scope "diff.plus")))))
           (define-typed-command! "arm-b" "" (lambda (bid)
             (set-line-backgrounds! "git-diff" bid (list (hash 'line 1 'scope "diff.minus")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-a");

    let entries = ed
        .state
        .config
        .decorations
        .line_backgrounds_for("git-diff", bid);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].pos,
        co(0),
        "line 0's line-start char offset is 0"
    );
    assert_eq!(scope_name(&ed, entries[0].scope), "diff.plus");

    // A second call for the same source must replace wholesale, not append.
    type_cmd(&mut ed, ":arm-b");
    let entries = ed
        .state
        .config
        .decorations
        .line_backgrounds_for("git-diff", bid);
    assert_eq!(
        entries.len(),
        1,
        "the second set-line-backgrounds! must replace, not append"
    );
    assert_eq!(
        entries[0].pos,
        co(8),
        "line 1's line-start char offset on this fixture (\"xabcdef\\n\" is 8 chars)"
    );
    assert_eq!(scope_name(&ed, entries[0].scope), "diff.minus");
}

/// Same drift regression as `sign_remaps_through_a_line_inserted_above_it`,
/// for line backgrounds: remap coverage applies to every
/// line-anchored kind (all four implement `PointAnchored`), not just signs.
#[test]
fn line_background_remaps_through_a_line_inserted_above_it() {
    let tmp = safe_tempdir();
    // The tint below sits on "bbbb\n", line 1.
    let mut ed = editor_from("-[x]>aaaa\nbbbb\ncccc\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-line-backgrounds! "git-diff" bid (list (hash 'line 1 'scope "diff.plus")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let tint_line = |ed: &Editor| {
        let pos = ed
            .state
            .config
            .decorations
            .line_backgrounds_for("git-diff", bid)[0]
            .pos;
        ed.state.buffers.get(bid).text().char_to_line(pos)
    };
    assert_eq!(
        tint_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "sanity: tint starts on line 1"
    );

    // Insert a whole new blank line above line 0: "bbbb" (and its tint)
    // must shift from line 1 to line 2.
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());
    ed.drain_lsp();

    assert_eq!(
        tint_line(&ed),
        hume_rope::line::ContentLine::new(2),
        "the tint must remap forward with the line it annotates"
    );
}

#[test]
fn diagnostics_for_buffer_and_diagnostic_counts_reflect_the_published_batch() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdefghij\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
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
        ed.diagnostic_counts(bid),
        (1, 1),
        "counts must tally exactly the one error and one warning"
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "counts" "" (lambda (bid)
             (let ((c (diagnostic-counts bid)))
               (log! 'info (to-string (hash-ref c 'errors) "." (hash-ref c 'warnings))))))
           (define-typed-command! "all" "" (lambda (bid)
             (log! 'info (to-string (length (diagnostics-for-buffer bid))))))
           (define-typed-command! "floored" "" (lambda (bid)
             (log! 'info (to-string (length (diagnostics-for-buffer bid #:severity 'warning))))))
           (define-typed-command! "ranged" "" (lambda (bid)
             (log! 'info (to-string (length (diagnostics-for-buffer bid #:range (hash 'start 5 'end 10)))))))
           (define-typed-command! "first" "" (lambda (bid)
             (let ((d (car (diagnostics-for-buffer bid))))
               (log! 'info (to-string (hash-ref d 'message) "@" (hash-ref d 'start)
                                      (equal? (hash-ref d 'severity) 'error))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":counts");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "1 . 1",
        "(errors . warnings) must be a real dotted pair"
    );

    type_cmd(&mut ed, ":all");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "3",
        "no floor/range must return all three"
    );

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

    type_cmd(&mut ed, ":first");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "an error @ 0 #true",
        "an entry's fields are read by symbol key, its severity a symbol"
    );
}

/// An unknown `#:severity` name (e.g. a typo like `'warn` for `'warning`)
/// must error loudly rather than silently returning nothing that qualifies.
/// A silent empty result is indistinguishable from "no diagnostics at
/// that floor".
#[test]
fn diagnostics_for_buffer_errors_loudly_on_an_unknown_severity_name() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "typo" "" (lambda (bid)
             (diagnostics-for-buffer bid #:severity 'warn)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":typo");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("warn") && log.to_lowercase().contains("severity"),
        "an unknown severity name must be reported loudly, naming the bad value: {log:?}"
    );
}
