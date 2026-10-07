// The full completion flow through the real shipped `core:lsp` plugin and
// a real LSP round trip: `completion-trigger` (Ctrl-Space) and the server's
// trigger chars invoke the plugin's registered `"lsp"` source, which sends
// textDocument/completion and answers with `completion-emit!`; accept
// applies additionalTextEdits or resolves; an `isIncomplete` answer is
// re-requested as the user types. The orchestration itself is covered
// without a server in `tests/completion/`; this file drives it end to end.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use super::*;
use crate::editor::tests::lsp_rig::stop_server;

fn full_completion_caps() -> serde_json::Value {
    serde_json::json!({
        "completionProvider": {"triggerCharacters": ["."], "resolveProvider": true}
    })
}

fn settle(ed: &mut Editor) {
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

#[test]
fn trigger_char_fires_the_completion_request() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        });

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

/// A trigger char is unsolicited (the user typed `.`, not "please
/// complete"), so a server answering with nothing must not flash a status
/// message on every keystroke that happens not to have anything to offer.
/// Contrast [`ctrl_space_with_no_completions_reports_it`]: an explicit
/// Ctrl-Space with the same empty answer does report.
#[test]
fn trigger_char_with_no_completions_is_silent() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        });

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(
        status(&ed),
        "",
        "a trigger char answered empty must not flash \"no completions\""
    );
}

/// [`trigger_char_with_no_completions_is_silent`]'s contrast: the same
/// empty answer to an explicit Ctrl-Space does report: the user asked.
#[test]
fn ctrl_space_with_no_completions_reports_it() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        });

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(status(&ed), "no completions");
}

#[test]
fn ctrl_space_fires_completion_trigger() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        });

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

#[test]
fn ctrl_space_in_a_buffer_with_no_file_sends_no_request() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        });
    ed.doc_mut().set_path(None);

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert_eq!(status(&ed), "no completions");
}

#[test]
fn capability_gated_no_completion_provider_sends_no_request() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, serde_json::json!({}), |_backend, _sid| {});

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert_eq!(
        status(&ed),
        "no completions",
        "the source declines without a request; the framework reports the empty answer"
    );
}

#[test]
fn null_response_opens_no_session() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::Value::Null);
        });

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        !status(&ed).to_lowercase().contains("error"),
        "a null response must be a clean no-op, not fall through to a type error \
         (json null decodes to Steel void, never #f), got status {:?}",
        status(&ed)
    );
    ed.feed_key(key_esc());

    // No session means accept! must error.
    let source = r#"(define-typed-command! "try-accept" "" (lambda () (completion-accept! 0)))"#;
    let mut host = ed.scripting.take().unwrap();
    eval_with_real_host(&mut ed, &mut host, source, tmp.path());
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":try-accept");

    assert!(
        status(&ed)
            .to_lowercase()
            .contains("no active completion session"),
        "a null response must open no session, got status {:?}",
        status(&ed)
    );
}

#[test]
fn accept_applies_main_edit_and_additional_text_edits_as_one_undo_step() {
    let tmp = safe_tempdir();
    // Blank line 0 (the auto-import destination) + "foo" on line 1 (the
    // completion site), non-overlapping, matching the real-world shape:
    // an import lands above the cursor's line, not at the exact same spot.
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "\nfoo\n",
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
    set_cursor(&mut ed, 1);

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    // Enter is the real acceptance key (insert.rs's completion-menu
    // intercept) and accepts the currently-selected (default: index 0) item.
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "use std::bar;\n\nbarfoo\n",
        "the main edit (insertText at the anchor, char 1) and additionalTextEdits \
         (line 0, above it) must both land"
    );

    ed.feed_key(key_esc()); // no menu left open: a plain Insert-mode exit
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
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "\nfoo\n",
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
    set_cursor(&mut ed, 1);

    ed.feed_key(key('i'));
    ed.settle();
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
    // "foo.b XXX\n": the main edit replaces ".b" (chars 3..5) with ".bar",
    // shifting everything after it on the line by +2 UTF-16 units. The
    // additionalTextEdits entry (chars 6..9, "XXX") is on the same line,
    // entirely after the main edit's end, so its position is stale unless
    // shifted by that same delta.
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "foo.b XXX\n",
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "filterText": ".bar",
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
    // Char 5 is right after "foo.b", matching the server's textEdit end
    // exactly, so accept() never extends the range past what's specified.
    set_cursor(&mut ed, 5);

    ed.feed_key(key('i'));
    ed.settle();
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
/// but with an astral-plane character (🎉, a UTF-16 surrogate pair: 2 wire
/// units, 1 char) before both edits on the line. The atomic-batch path
/// (`build_edit_changeset`) converts each edit's own wire position to a char
/// offset independently via `wire_to_char` (no UTF-16-delta arithmetic
/// between edits at all), so this proves that conversion is correct with an
/// astral prefix on the line, not just plain ASCII.
#[test]
fn additional_edit_on_the_same_line_with_an_astral_prefix_lands_correctly() {
    let tmp = safe_tempdir();
    // "🎉foo.b XXX\n": 🎉 is char 0 (wire columns 0..2); "foo.b XXX" follows
    // at char 1 (wire column 2).
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "🎉foo.b XXX\n",
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "filterText": ".bar",
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
    // Char 6: right after "🎉foo.b" (1 + 5 = 6), matching the server's
    // textEdit end exactly.
    set_cursor(&mut ed, 6);

    ed.feed_key(key('i'));
    ed.settle();
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
/// `additional_edit_on_the_same_line_as_a_text_edit_main_edit_shifts_with_it`:
/// same fixture and expected result, but the additionalTextEdits arrive
/// via `completionItem/resolve` instead of inline on the completion
/// response, exercising `edits::build_edits_from_earlier_document`'s
/// `ChangeSet::map_ranges` position tracking instead of the inline atomic
/// batch. Both land at the identical final text, proving the resolve path
/// is exact, not an approximation.
#[test]
fn resolved_additional_edits_land_through_the_accept_edit_on_the_same_line() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "foo.b XXX\n",
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "filterText": ".bar",
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
    set_cursor(&mut ed, 5);

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    // `key_enter()` runs accept synchronously (main edit lands, resolve
    // request sent); the single `settle()` below drains the scripted
    // backend's already-queued response and runs the (plain Rust, not
    // Steel-queued) resolve callback inline, so no second drain round needed,
    // unlike a Steel `lsp-request!` callback which only queues on response.
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "foo.bar YYY\n",
        "a resolved additionalTextEdit on the same line as the main edit must land at \
         the exact position, mapped through the accept ChangeSet, not approximated \
         by a UTF-16 delta"
    );
}

/// The staleness half of the resolve contract: a resolve response arriving
/// after the user has typed more text must be dropped, not applied against
/// stale positions (same discipline `ResponseAnchor` already gives every
/// other `lsp-request!`).
#[test]
fn resolved_additional_edits_are_dropped_after_a_post_accept_edit() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        tmp.path(),
        "\nfoo\n",
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
    set_cursor(&mut ed, 1);

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    // `key_enter()`'s keybinding dispatch runs `accept_completion_selection`
    // synchronously: the main edit lands and the resolve request is *sent*
    // in this call, but its scripted response isn't drained until the next
    // `drain_lsp`. Typing `X` right here, before any drain, changes the text version
    // past what the resolve request's `ResponseAnchor` was armed with.
    ed.feed_key(key_enter());
    ed.feed_key(key('X'));

    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "\nbarXfoo\n",
        "a resolve response landing after further typing must be dropped: no \
         \"use std::bar;\" must appear, and the typed X must survive untouched"
    );
}

/// `:lsp-stop` sweeps every pending request via `LspClient::drain_pending`
/// (the same generic teardown every in-flight `lsp-request!` gets): a
/// resolve request in flight at stop time must not apply anything once its
/// swept `Outcome::TimedOut` reaches the callback, and must not panic.
#[test]
fn resolve_does_not_apply_anything_after_lsp_stop() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) = setup_trigger_char_feature(
        tmp.path(),
        "\nfoo\n",
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            // No scripted reply for completionItem/resolve:
            // :lsp-stop must sweep it before any reply would matter.
        },
    );
    set_cursor(&mut ed, 1);

    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    ed.feed_key(key_enter()); // main edit lands, resolve request sent

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "sanity: resolve must have been sent before the stop"
    );

    stop_server(&mut ed, "rust-analyzer"); // sweeps the in-flight resolve as TimedOut

    assert_eq!(
        ed.doc().text().to_string(),
        "\nbarfoo\n",
        "the main edit must stand, and the swept resolve must not apply anything \
         (no panic, no phantom edit)"
    );
}

/// A completion item resolves with the server that sent it. `rust-analyzer`
/// is attached first but its list entry excludes completion, so the items
/// come from `ra-lint`, and so must the resolve.
#[test]
fn completion_resolve_goes_to_the_items_origin_server() {
    use hume_lsp::backend::ServerId;
    let tmp = safe_tempdir();
    let _guard = RealRuntimeGuard::new();
    let (mut backend, _, requests) = RecordingLspBackend::new();
    let caps = serde_json::json!({ "capabilities": full_completion_caps() });
    backend.respond_to("initialize", caps.clone());
    backend.respond_to("initialize", caps);
    backend.respond_to_server(
        ServerId(1),
        "textDocument/completion",
        serde_json::json!([{"label": "bar", "insertText": "bar"}]),
    );
    backend.respond_to(
        "completionItem/resolve",
        serde_json::json!({"label": "bar"}),
    );
    let init = format!(
        r#"{}
(register-lsp-server! "ra-lint" #:command "ra-lint")
(set-language-servers! "rust" (list (hash 'name "rust-analyzer" 'except-features '(completion)) "ra-lint"))"#,
        core_lsp_init()
    );
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust(&marked_at_start(FOO)).with_init(&init),
        backend,
    );
    rig.ed.settle();

    rig.ed.feed_key(key('i'));
    rig.ed.settle();
    rig.ed.feed_key(key_ctrl(' '));
    settle(&mut rig.ed);
    rig.ed.feed_key(key_enter());
    settle(&mut rig.ed);

    let resolves: Vec<ServerId> = requests
        .borrow()
        .iter()
        .filter(|(_, m, _)| m == "completionItem/resolve")
        .map(|(sid, _, _)| *sid)
        .collect();
    assert_eq!(resolves, vec![ServerId(1)]);
}

#[test]
fn resolve_sent_only_when_item_lacks_additional_text_edits_and_resolve_provider_present() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({"label": "bar"}),
            );
        });
    ed.feed_key(key('i'));
    ed.settle();
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
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to("completionItem/resolve", serde_json::Value::Null);
        });
    ed.feed_key(key('i'));
    ed.settle();
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
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
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
        });
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "barfoo\n",
        "sanity: accept must actually have run"
    );
    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        0,
        "an item that already carries additionalTextEdits (even empty) must not resolve"
    );
}

/// The `"lsp"` source answered `isIncomplete`: the framework calls it again
/// on the next keystroke, and it re-requests.
#[test]
fn an_incomplete_answer_is_re_requested_on_typing() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) = setup_trigger_char_feature(
        tmp.path(),
        FOO,
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
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        2,
        "typing after an isIncomplete answer must re-request"
    );
}

#[test]
fn a_complete_answer_is_not_re_requested_on_typing() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "foo", "insertText": "foo"}]),
            );
        });
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        1,
        "a complete (non-isIncomplete) answer must not re-request on further typing"
    );
}

/// Detach must be a true no-op, not a per-keystroke request or log: the
/// `"lsp"` source's trigger-char registration is set once at attach, so
/// `on-lsp-detach` must clear it. A trigger char left registered past
/// `:lsp-stop` would still invoke the source on every matching keystroke.
#[test]
fn detach_clears_completion_trigger_chars_so_a_stale_trigger_is_a_true_no_op() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |_backend, _sid| {});

    stop_server(&mut ed, "rust-analyzer");
    ed.settle(); // on-lsp-detach clears the "lsp" trigger chars

    ed.feed_key(key('i'));
    ed.settle();
    let before = ed.state.status_msg.clone();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert_eq!(
        ed.state.status_msg, before,
        "a trigger char left registered past detach must be a true no-op, not a \
         status message every keystroke"
    );
}

/// An open completion session's `items` are a snapshot already fetched from
/// the server, not a live subscription, but leaving it open after the
/// server stops would keep showing (and let the user accept) suggestions
/// from a server that's no longer running for this buffer.
#[test]
fn detach_dismisses_an_open_completion_session_for_that_buffer() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
        });
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "sanity: a session must be open"
    );

    stop_server(&mut ed, "rust-analyzer");

    assert!(
        ed.state.input.buffer_completion().is_none(),
        "an open completion session for the detached buffer must be dismissed, \
         not left showing stale items from a server that's no longer running"
    );
}

#[test]
fn snippet_item_lands_as_stripped_plain_text() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _requests) =
        setup_trigger_char_feature(tmp.path(), FOO, full_completion_caps(), |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "for",
                    "insertText": "for ${1:x} in ${2:iter} {\n    $0\n}",
                    "insertTextFormat": 2
                }]),
            );
        });
    ed.feed_key(key('i'));
    ed.settle();
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
