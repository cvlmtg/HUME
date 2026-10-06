// The shipped `core:lsp` plugin over two servers on one buffer: features
// that merge every server's answer (completion, code actions, locations,
// inlay hints, diagnostics) and features that pick one (formatting), with
// per-server trigger characters. Loads the real plugin in place
// (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::editor::tests::lsp_rig::RA_LINT;
use hume_engine::pipeline::RenderContext;
use hume_lsp::test_util::RecordingLspBackend;

const RA: ServerId = ServerId(0);
const LINT: ServerId = ServerId(1);

fn init_result(capabilities: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "capabilities": capabilities })
}

/// `rust-analyzer` (`RA`) and `ra-lint` (`LINT`) on `content`, cursor at its
/// start, answering `initialize` with `ra` and `lint` capabilities; `extra`
/// is Scheme evaluated after both register.
fn two_servers(
    tmp: &Path,
    content: &str,
    ra: serde_json::Value,
    lint: serde_json::Value,
    extra: &str,
    script: impl FnOnce(&mut RecordingLspBackend),
) -> (LspRig, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to_server(RA, "initialize", init_result(ra));
    backend.respond_to_server(LINT, "initialize", init_result(lint));
    script(&mut backend);
    let init = format!("{}\n{RA_LINT}\n{extra}", core_lsp_init());
    let mut rig = LspRig::drained(
        tmp,
        RigSpec::rust(&marked_at_start(content)).with_init(&init),
        backend,
    );
    rig.ed.settle();
    (rig, guard)
}

fn settle(ed: &mut Editor) {
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn run(ed: &mut Editor, command: &str) {
    ed.execute_keymap_command(command.to_string().into(), Some(1), false);
    settle(ed);
}

/// The labels the open completion menu ranks, sorted.
fn completion_labels(ed: &Editor) -> Vec<String> {
    let mut labels: Vec<String> = ed
        .state
        .input
        .buffer_completion()
        .map(|s| {
            s.top(20, &ed.state.config.completion_sources)
                .iter()
                .map(|v| v["label"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    labels.sort();
    labels
}

fn menu_rows(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .menu()
        .map(|m| m.rows.iter().map(|r| r.main.to_string()).collect())
        .unwrap_or_default()
}

fn completion_caps() -> serde_json::Value {
    serde_json::json!({ "completionProvider": { "triggerCharacters": [] } })
}

fn open_completion(ed: &mut Editor) {
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key_ctrl(' '));
    settle(ed);
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

#[test]
fn completion_merges_two_servers_items() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        FOO,
        completion_caps(),
        completion_caps(),
        "",
        |b| {
            b.respond_to_server(
                RA,
                "textDocument/completion",
                serde_json::json!([{"label": "alpha"}]),
            );
            b.respond_to_server(
                LINT,
                "textDocument/completion",
                serde_json::json!({"isIncomplete": false, "items": [{"label": "beta"}]}),
            );
        },
    );

    open_completion(&mut rig.ed);

    assert_eq!(completion_labels(&rig.ed), vec!["alpha", "beta"]);
}

#[test]
fn completion_incomplete_if_any() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        FOO,
        completion_caps(),
        completion_caps(),
        "",
        |b| {
            for _ in 0..2 {
                b.respond_to_server(
                    RA,
                    "textDocument/completion",
                    serde_json::json!([{"label": "alpha"}]),
                );
                b.respond_to_server(
                    LINT,
                    "textDocument/completion",
                    serde_json::json!({"isIncomplete": true, "items": [{"label": "abc"}]}),
                );
            }
        },
    );
    open_completion(&mut rig.ed);

    rig.ed.feed_key(key('a'));
    settle(&mut rig.ed);

    for sid in [RA, LINT] {
        assert_eq!(
            rig.requests_to(sid, "textDocument/completion").len(),
            2,
            "typing re-asks every server: {sid:?}"
        );
    }
}

#[test]
fn except_features_completion_excludes_server() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        FOO,
        completion_caps(),
        completion_caps(),
        r#"(set-language-servers! "rust" (list "rust-analyzer" (hash 'name "ra-lint" 'except-features '(completion))))"#,
        |b| {
            b.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "alpha"}]),
            )
        },
    );

    open_completion(&mut rig.ed);

    assert_eq!(rig.requests_to(RA, "textDocument/completion").len(), 1);
    assert!(rig.requests_to(LINT, "textDocument/completion").is_empty());
}

#[test]
fn trigger_chars_union_per_server_and_die_with_the_instance() {
    let tmp = safe_tempdir();
    let trigger =
        |c: &str| serde_json::json!({ "completionProvider": { "triggerCharacters": [c] } });
    let (mut rig, _guard) = two_servers(tmp.path(), FOO, trigger("."), trigger(":"), "", |b| {
        for _ in 0..3 {
            b.respond_to("textDocument/completion", serde_json::json!([]));
        }
    });
    let completions = |rig: &LspRig| {
        rig.requests
            .borrow()
            .iter()
            .filter(|(_, m, _)| m == "textDocument/completion")
            .count()
    };

    rig.ed.feed_key(key('i'));
    rig.ed.settle();
    rig.ed.feed_key(key('.'));
    settle(&mut rig.ed);
    rig.ed.feed_key(key(':'));
    settle(&mut rig.ed);
    let both = completions(&rig);
    rig.ed.feed_key(key_esc());
    rig.stop("ra-lint");
    rig.ed.settle();
    rig.ed.feed_key(key('i'));
    rig.ed.settle();
    rig.ed.feed_key(key(':'));
    settle(&mut rig.ed);

    assert!(both >= 2, "each server's own trigger char fires: {both}");
    assert_eq!(
        completions(&rig),
        both,
        "the stopped server's ':' no longer fires"
    );
}

#[test]
fn format_picks_first_with_range_provider() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "line1\nline2\n",
        serde_json::json!({ "documentFormattingProvider": true }),
        serde_json::json!({ "documentFormattingProvider": true, "documentRangeFormattingProvider": true }),
        "",
        |b| b.respond_to("textDocument/rangeFormatting", serde_json::json!([])),
    );
    select(&mut rig.ed, &[(0, 5)], 0);

    run(&mut rig.ed, "lsp-fmt");

    assert!(
        rig.requests_to(RA, "textDocument/rangeFormatting")
            .is_empty()
    );
    assert_eq!(
        rig.requests_to(LINT, "textDocument/rangeFormatting").len(),
        1
    );
}

#[test]
fn format_uses_the_first_whole_document_formatter_for_a_partial_selection() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "line1\nline2\n",
        serde_json::json!({ "documentRangeFormattingProvider": true }),
        serde_json::json!({ "documentFormattingProvider": true }),
        "",
        |b| b.respond_to("textDocument/formatting", serde_json::json!([])),
    );

    run(&mut rig.ed, "lsp-fmt");

    assert!(rig.requests_to(RA, "textDocument/formatting").is_empty());
    assert_eq!(rig.requests_to(LINT, "textDocument/formatting").len(), 1);
}

fn action(title: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "command": {"title": title, "command": "smoke.doThing", "arguments": []}
    })
}

#[test]
fn code_actions_menu_merges_with_suffix_only_when_multi() {
    let caps = || serde_json::json!({ "codeActionProvider": true });
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(tmp.path(), FOO, caps(), caps(), "", |b| {
        b.respond_to_server(
            RA,
            "textDocument/codeAction",
            serde_json::json!([action("Fix")]),
        );
        b.respond_to_server(
            LINT,
            "textDocument/codeAction",
            serde_json::json!([action("Lint")]),
        );
        b.respond_to_server(
            RA,
            "textDocument/codeAction",
            serde_json::json!([action("Fix")]),
        );
        b.respond_to_server(LINT, "textDocument/codeAction", serde_json::json!([]));
    });

    run(&mut rig.ed, "lsp-code-actions");
    let merged = menu_rows(&rig.ed);
    rig.ed.feed_key(key_esc());
    rig.ed.settle();
    run(&mut rig.ed, "lsp-code-actions");

    assert_eq!(merged, vec!["Fix (rust-analyzer)", "Lint (ra-lint)"]);
    assert_eq!(menu_rows(&rig.ed), vec!["Fix"]);
}

#[test]
fn code_action_context_only_own_diagnostics() {
    let caps = || serde_json::json!({ "codeActionProvider": true });
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(tmp.path(), FOO, caps(), caps(), "", |b| {
        b.respond_to("textDocument/codeAction", serde_json::json!([]));
    });
    let diag = |message: &str| {
        serde_json::json!([{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "severity": 2,
            "message": message,
        }])
    };
    rig.publish(RA, diag("from ra"));
    rig.publish(LINT, diag("from lint"));
    rig.ed.settle();

    run(&mut rig.ed, "lsp-code-actions");

    let messages = |sid| -> Vec<String> {
        rig.requests_to(sid, "textDocument/codeAction")[0]["context"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["message"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(messages(RA), vec!["from ra"]);
    assert_eq!(messages(LINT), vec!["from lint"]);
}

#[test]
fn execute_command_goes_to_producing_server() {
    let caps = || serde_json::json!({ "codeActionProvider": true, "executeCommandProvider": {"commands": []} });
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(tmp.path(), FOO, caps(), caps(), "", |b| {
        b.respond_to_server(
            RA,
            "textDocument/codeAction",
            serde_json::json!([action("Fix")]),
        );
        b.respond_to_server(
            LINT,
            "textDocument/codeAction",
            serde_json::json!([action("Lint")]),
        );
        b.respond_to("workspace/executeCommand", serde_json::json!(null));
    });

    run(&mut rig.ed, "lsp-code-actions");
    rig.ed.feed_key(key('j'));
    rig.ed.feed_key(key_enter());
    settle(&mut rig.ed);

    assert!(rig.requests_to(RA, "workspace/executeCommand").is_empty());
    assert_eq!(rig.requests_to(LINT, "workspace/executeCommand").len(), 1);
}

#[test]
fn goto_two_servers_same_location_jumps() {
    let caps = || serde_json::json!({ "definitionProvider": true });
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "fn main() {}\nfn foo() {}\n",
        caps(),
        caps(),
        "",
        |b| {
            b.respond_to(
                "textDocument/definition",
                serde_json::json!([loc(&uri, 1, 3)]),
            );
            b.respond_to(
                "textDocument/definition",
                serde_json::json!([loc(&uri, 1, 3)]),
            );
        },
    );

    run(&mut rig.ed, "lsp-goto-definition");

    assert!(
        rig.ed.state.views.drawer.read().is_none(),
        "one place, no list"
    );
    assert_eq!(rig.ed.current_view().primary().head().offset(), co(16));
}

#[test]
fn references_merge_dedupe_drawer() {
    let caps = || serde_json::json!({ "referencesProvider": true });
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "fn main() {\n    foo();\n}\n",
        caps(),
        caps(),
        "",
        |b| {
            b.respond_to_server(
                RA,
                "textDocument/references",
                serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4)]),
            );
            b.respond_to_server(
                LINT,
                "textDocument/references",
                serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            );
        },
    );

    run(&mut rig.ed, "lsp-references");

    assert_eq!(drawer_rows(&rig.ed).len(), 3);
}

#[test]
fn locations_refresh_regroups_servers() {
    let caps = || serde_json::json!({ "referencesProvider": true });
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "fn main() {\n    foo();\n}\n",
        caps(),
        caps(),
        "",
        |b| {
            b.respond_to_server(
                RA,
                "textDocument/references",
                serde_json::json!([loc(&uri, 1, 4)]),
            );
            b.respond_to_server(
                LINT,
                "textDocument/references",
                serde_json::json!([loc(&uri, 2, 0)]),
            );
            b.respond_to_server(
                RA,
                "textDocument/references",
                serde_json::json!([loc(&uri, 2, 4)]),
            );
            b.respond_to_server(
                LINT,
                "textDocument/references",
                serde_json::json!([loc(&uri, 3, 0), loc(&uri, 2, 4)]),
            );
        },
    );
    set_cursor(&mut rig.ed, 16);
    run(&mut rig.ed, "lsp-references");
    assert_eq!(drawer_rows(&rig.ed).len(), 2, "setup: one row per server");

    set_cursor(&mut rig.ed, 0);
    rig.ed.handle_key(key('O'));
    rig.ed.handle_key(key_esc());
    rig.ed.settle();
    std::thread::sleep(Duration::from_millis(400));
    rig.ed.settle();
    settle(&mut rig.ed);

    for sid in [RA, LINT] {
        assert_eq!(
            rig.requests_to(sid, "textDocument/references").len(),
            2,
            "{sid:?}"
        );
    }
    assert_eq!(
        drawer_rows(&rig.ed).len(),
        2,
        "both servers' new rows, the shared one once"
    );
}

fn hints(entries: &[(u32, u32, &str)]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|(line, character, label)| {
                serde_json::json!({"position": {"line": line, "character": character}, "label": label})
            })
            .collect(),
    )
}

fn refresh_hints(ed: &mut Editor) {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focus.id();
    ed.queue_viewport_change(pid);
    ed.settle();
    std::thread::sleep(Duration::from_millis(300));
    ed.settle();
    ed.settle();
}

fn hint_texts(ed: &Editor) -> Vec<String> {
    let bid = ed.focused_buffer_id();
    let mut texts: Vec<String> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .map(|h| h.text.clone())
        .collect();
    texts.sort();
    texts
}

#[test]
fn inlay_merged_from_two_servers() {
    let caps = || serde_json::json!({ "inlayHintProvider": true });
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(tmp.path(), "let x = 1;\n", caps(), caps(), "", |b| {
        b.respond_to_server(RA, "textDocument/inlayHint", hints(&[(0, 5, ": i32")]));
        b.respond_to_server(LINT, "textDocument/inlayHint", hints(&[(0, 9, " // lint")]));
    });
    rig.ed.state.settings.lsp_inlay_hints = true;

    refresh_hints(&mut rig.ed);

    assert_eq!(hint_texts(&rig.ed), vec![" // lint", ": i32"]);
}

#[test]
fn inlay_detach_one_keeps_other_hints() {
    let caps = || serde_json::json!({ "inlayHintProvider": true });
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(tmp.path(), "let x = 1;\n", caps(), caps(), "", |b| {
        b.respond_to_server(RA, "textDocument/inlayHint", hints(&[(0, 5, ": i32")]));
        b.respond_to_server(LINT, "textDocument/inlayHint", hints(&[(0, 9, " // lint")]));
        b.respond_to_server(RA, "textDocument/inlayHint", hints(&[(0, 5, ": i32")]));
    });
    rig.ed.state.settings.lsp_inlay_hints = true;
    refresh_hints(&mut rig.ed);

    rig.stop("ra-lint");
    rig.ed.settle();
    std::thread::sleep(Duration::from_millis(300));
    rig.ed.settle();
    rig.ed.settle();

    assert_eq!(hint_texts(&rig.ed), vec![": i32"]);
}

#[test]
fn diagnostics_detach_one_keeps_other_decorations() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        "a\nb\n",
        serde_json::json!({}),
        serde_json::json!({}),
        "",
        |_| {},
    );
    let diag = |line: u32| {
        serde_json::json!([{
            "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}},
            "severity": 1,
            "message": "boom",
        }])
    };
    rig.publish(RA, diag(0));
    rig.publish(LINT, diag(1));
    rig.ed.settle();
    let eol = |ed: &Editor| {
        ed.state
            .config
            .decorations
            .eol_text_for_buffer(ed.focused_buffer_id())
            .count()
    };
    assert_eq!(eol(&rig.ed), 2, "setup: one line each");

    rig.stop("ra-lint");
    rig.ed.settle();

    assert_eq!(eol(&rig.ed), 1);
}

/// A feature no server of the buffer offers is a fact about the setup, not
/// a failure: it is reported at Info, never at Error.
#[test]
fn unsupported_feature_reports_at_info() {
    let tmp = safe_tempdir();
    let (mut rig, _guard) = two_servers(
        tmp.path(),
        FOO,
        serde_json::json!({}),
        serde_json::json!({}),
        "",
        |_| {},
    );

    run(&mut rig.ed, "lsp-hover");

    let status = rig.ed.state.status_msg.clone().unwrap_or_default();
    assert!(status.contains("hover is not supported"), "{status:?}");
    let errors: Vec<String> = rig
        .ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == crate::editor::Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
