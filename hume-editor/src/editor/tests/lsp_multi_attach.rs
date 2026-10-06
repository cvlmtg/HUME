// Several servers attached to one buffer: per-server document sync, the
// attachment lifecycle a server list drives, merged diagnostics, and the
// per-(buffer, server) hooks.

use super::lsp_rig::{LspRig, RigSpec, TWO_RUST_SERVERS};
use super::*;
use crate::editor::lsp::introspect::LspActivity;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;
use hume_rope::position_encoding::PositionEncoding;
use hume_scripting::{FeatureFilter, LspFeature};
use test_fixtures::unicode::{ASTRAL, CJK, COMBINING};

const INCREMENTAL: i64 = 2;
const FULL: i64 = 1;

fn initialize_result(sync: i64, encoding: &str) -> serde_json::Value {
    serde_json::json!({
        "capabilities": { "textDocumentSync": sync, "positionEncoding": encoding }
    })
}

/// `rust-analyzer` (spawned first, `ServerId(0)`) and `ra-lint`
/// (`ServerId(1)`) on `src/main.rs`, answering `initialize` with `first`
/// and `second`.
fn two_server_backend(first: serde_json::Value, second: serde_json::Value) -> RecordingLspBackend {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to_server(ServerId(0), "initialize", first);
    backend.respond_to_server(ServerId(1), "initialize", second);
    backend
}

fn two_servers(tmp: &tempfile::TempDir, marked: &str) -> LspRig {
    let caps = || initialize_result(INCREMENTAL, "utf-8");
    LspRig::drained(
        tmp.path(),
        RigSpec::rust(marked).with_init(TWO_RUST_SERVERS),
        two_server_backend(caps(), caps()),
    )
}

fn type_insert(ed: &mut Editor, text: &str) {
    ed.feed_key(key('i'));
    type_chars(ed, text);
    ed.feed_key(key_esc());
}

/// Replays what `sid` was told about the rig's document (`didOpen`, then
/// each `didChange`) against a plain `String`, decoding ranged events in
/// `encoding`, and returns the text and the last version it was told.
fn replay(rig: &LspRig, sid: ServerId, encoding: PositionEncoding) -> (String, i64) {
    let mut text = String::new();
    let mut version = -1;
    for (s, method, params) in rig.notifications.borrow().iter() {
        if *s != sid {
            continue;
        }
        match method.as_str() {
            "textDocument/didOpen" => {
                text = params["textDocument"]["text"].as_str().unwrap().to_string();
                version = params["textDocument"]["version"].as_i64().unwrap();
            }
            "textDocument/didChange" => {
                version = params["textDocument"]["version"].as_i64().unwrap();
                let events: Vec<lsp_types::TextDocumentContentChangeEvent> =
                    serde_json::from_value(params["contentChanges"].clone()).unwrap();
                for event in events {
                    if event.range.is_some() {
                        text = hume_lsp::sync::apply_events_to_string_mirror(
                            text,
                            std::slice::from_ref(&event),
                            encoding,
                        );
                    } else {
                        text = event.text;
                    }
                }
            }
            _ => {}
        }
    }
    (text, version)
}

fn diagnostic(line: u32, severity: i64) -> serde_json::Value {
    serde_json::json!({
        "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}},
        "severity": severity,
        "message": "boom",
    })
}

#[test]
fn two_servers_both_receive_did_open_change_save_close() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[f]>n main() {}\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));
    assert_eq!(rig.attached(), vec![ra, lint]);

    type_insert(&mut rig.ed, "x");
    rig.ed.drain_lsp();
    rig.ed.execute_typed("w", None).unwrap();
    rig.ed.close_buffer(rig.bid);

    for sid in [ra, lint] {
        assert_eq!(rig.sent(sid, "textDocument/didOpen").len(), 1, "{sid:?}");
        assert_eq!(rig.sent(sid, "textDocument/didChange").len(), 1, "{sid:?}");
        assert_eq!(rig.sent(sid, "textDocument/didSave").len(), 1, "{sid:?}");
        assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1, "{sid:?}");
    }
}

/// One INCREMENTAL server negotiating UTF-8 and one FULL server left on
/// UTF-16, both on text whose positions differ between the two encodings:
/// each one's own replay must reproduce the buffer.
#[test]
fn flush_uses_each_servers_encoding_and_sync_kind() {
    let tmp = safe_tempdir();
    let marked = format!("{ASTRAL}-[{CJK}]>{COMBINING}\n");
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust(&marked).with_init(TWO_RUST_SERVERS),
        two_server_backend(
            initialize_result(INCREMENTAL, "utf-8"),
            initialize_result(FULL, "utf-16"),
        ),
    );
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    type_insert(&mut rig.ed, "Z");
    type_insert(&mut rig.ed, "Y");
    rig.ed.drain_lsp();

    let buffer = rig.ed.state.buffers.get(rig.bid).text();
    let (expected, generation) = (buffer.to_string(), buffer.generation() as i64);
    assert_eq!(
        replay(&rig, ra, PositionEncoding::Utf8),
        (expected.clone(), generation)
    );
    assert_eq!(
        replay(&rig, lint, PositionEncoding::Utf16),
        (expected, generation)
    );
    let ranged = |sid| {
        rig.sent(sid, "textDocument/didChange")
            .iter()
            .flat_map(|p| p["contentChanges"].as_array().unwrap().clone())
            .all(|event| event.get("range").is_some())
    };
    assert!(ranged(ra), "an INCREMENTAL server gets ranged events");
    assert!(
        rig.sent(lint, "textDocument/didChange")
            .iter()
            .flat_map(|p| p["contentChanges"].as_array().unwrap().clone())
            .all(|event| event.get("range").is_none()),
        "a FULL server gets whole-document events only"
    );
}

#[test]
fn second_server_attach_after_unflushed_edit_does_not_replay_old_changes() {
    let tmp = safe_tempdir();
    let caps = || initialize_result(INCREMENTAL, "utf-8");
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n"),
        two_server_backend(caps(), caps()),
    );
    let ra = rig.sid("rust-analyzer");
    type_insert(&mut rig.ed, "x");

    rig.eval(super::lsp_rig::RA_LINT);
    rig.ed.drain_lsp();

    let lint = rig.sid("ra-lint");
    assert_eq!(rig.attached(), vec![ra, lint]);
    assert_eq!(
        rig.sent(ra, "textDocument/didChange").len(),
        1,
        "the attached server gets the edit before the new one opens the document"
    );
    let opened = rig.sent(lint, "textDocument/didOpen");
    assert_eq!(opened[0]["textDocument"]["text"], "xfn main() {}\n");
    assert!(
        rig.sent(lint, "textDocument/didChange").is_empty(),
        "an edit the didOpen text already contains must not reach the new server"
    );
}

#[test]
fn stop_one_server_keeps_other_attached_and_pending_queue() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[f]>n main() {}\n");
    let ra = rig.sid("rust-analyzer");
    type_insert(&mut rig.ed, "x");

    rig.stop("ra-lint");
    type_insert(&mut rig.ed, "y");
    rig.ed.drain_lsp();

    assert_eq!(rig.attached(), vec![ra]);
    let buffer = rig.ed.state.buffers.get(rig.bid).text();
    assert_eq!(
        replay(&rig, ra, PositionEncoding::Utf8),
        (buffer.to_string(), buffer.generation() as i64)
    );
}

#[test]
fn last_detach_clears_pending() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    for _ in 0..2 {
        backend.respond_to("initialize", initialize_result(INCREMENTAL, "utf-8"));
    }
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[f]>n main() {}\n"), backend);
    rig.stop("rust-analyzer");
    type_insert(&mut rig.ed, "x");
    assert!(!rig.ed.state.buffer_positions.lsp.has_doc(rig.bid));

    rig.ed
        .apply_lsp_server_op(hume_scripting::PendingLspServerOp::Restart {
            target: hume_scripting::LspServerTarget::Name(
                hume_scripting::ServerName::parse("rust-analyzer").unwrap(),
            ),
        });
    let fresh = rig.sid("rust-analyzer");
    rig.ed.drain_lsp();

    assert!(rig.sent(fresh, "textDocument/didChange").is_empty());
    assert_eq!(
        rig.sent(fresh, "textDocument/didOpen")[0]["textDocument"]["text"],
        "xfn main() {}\n"
    );
}

#[test]
fn diagnostics_merge_from_two_servers_counts() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[a]>\nb\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.publish(ra, serde_json::json!([diagnostic(0, 1)]));
    rig.publish(
        lint,
        serde_json::json!([diagnostic(1, 2), diagnostic(0, 2)]),
    );

    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (1, 2));
}

#[test]
fn diagnostics_entries_carry_server() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[a]>\nb\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));
    rig.publish(ra, serde_json::json!([diagnostic(0, 1)]));
    rig.publish(lint, serde_json::json!([diagnostic(1, 1)]));

    rig.probe(
        r#"(log! 'warn (to-string (map (lambda (d) (lsp-server-name (hash-ref d 'server)))
                                       (diagnostics-for-buffer pane))))"#,
    );

    assert_eq!(
        rig.warnings(),
        vec![r#"("rust-analyzer" "ra-lint")"#.to_string()]
    );
}

#[test]
fn except_diagnostics_filter_drops_publish() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[a]>\nb\n");
    rig.eval(
        r#"(set-language-servers! "rust"
             (list "rust-analyzer" (hash 'name "ra-lint" 'except-features '(diagnostics))))"#,
    );
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.publish(ra, serde_json::json!([diagnostic(0, 1)]));
    rig.publish(lint, serde_json::json!([diagnostic(1, 1)]));

    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (1, 0));
}

#[test]
fn detach_removes_only_that_servers_diagnostics() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[a]>\nb\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));
    rig.publish(ra, serde_json::json!([diagnostic(0, 1)]));
    rig.publish(lint, serde_json::json!([diagnostic(1, 2)]));

    rig.stop("ra-lint");

    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (1, 0));
}

#[test]
fn set_language_servers_reconcile_detaches_with_did_close() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[f]>n main() {}\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);

    assert_eq!(rig.attached(), vec![ra]);
    assert_eq!(rig.sent(lint, "textDocument/didClose").len(), 1);
    assert!(
        rig.ed
            .state
            .lsp
            .instances_named_for_test("ra-lint")
            .is_empty()
    );
}

#[test]
fn reorder_reconcile_reorders_attachments() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[f]>n main() {}\n");
    let (ra, lint) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"));

    rig.eval(r#"(set-language-servers! "rust" '("ra-lint" "rust-analyzer"))"#);

    assert_eq!(rig.attached(), vec![lint, ra]);
    for sid in [ra, lint] {
        assert_eq!(rig.sent(sid, "textDocument/didOpen").len(), 1, "{sid:?}");
        assert!(rig.sent(sid, "textDocument/didClose").is_empty(), "{sid:?}");
    }
}

#[test]
fn changing_a_listed_servers_filter_keeps_the_instance_and_updates_the_filter() {
    let tmp = safe_tempdir();
    let mut rig = two_servers(&tmp, "-[a]>\nb\n");
    let lint = rig.sid("ra-lint");
    rig.publish(lint, serde_json::json!([diagnostic(0, 1)]));
    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (1, 0));

    rig.eval(
        r#"(set-language-servers! "rust"
             (list "rust-analyzer" (hash 'name "ra-lint" 'only-features '(format))))"#,
    );

    assert_eq!(rig.sid("ra-lint"), lint, "the instance is kept");
    assert!(rig.sent(lint, "textDocument/didClose").is_empty());
    assert_eq!(
        rig.ed.state.buffer_positions.lsp.filter_of(rig.bid, lint),
        Some(FeatureFilter::Only(
            [LspFeature::Format].into_iter().collect()
        ))
    );
    assert_eq!(
        rig.ed.diagnostic_counts(rig.bid),
        (0, 0),
        "a filter that no longer admits diagnostics drops what the server published"
    );
}

#[test]
fn activity_aggregates_starting_first() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to_server(
        ServerId(0),
        "initialize",
        initialize_result(INCREMENTAL, "utf-8"),
    );
    let rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(TWO_RUST_SERVERS),
        backend,
    );

    assert!(matches!(
        rig.ed.lsp_activity(rig.bid),
        LspActivity::Starting
    ));
}

#[test]
fn status_lists_names_per_buffer() {
    let tmp = safe_tempdir();
    let rig = two_servers(&tmp, "-[f]>n main() {}\n");

    let text = rig.ed.lsp_status_text();
    assert!(
        text.contains("main.rs [rust-analyzer, ra-lint]: 0 error(s), 0 warning(s)"),
        "{text}"
    );
    assert!(text.contains("ra-lint [rust] @"), "{text}");
    assert!(text.contains("rust-analyzer [rust] @"), "{text}");
}

#[test]
fn attach_hook_fires_once_per_buffer_and_server() {
    let tmp = safe_tempdir();
    let caps = || initialize_result(INCREMENTAL, "utf-8");
    let init = format!(
        r#"{TWO_RUST_SERVERS}
(register-hook! 'on-lsp-attach (lambda (pane server)
  (set-buffer-option! pane "tab-width" (+ 1 (get-buffer-option pane "tab-width")))))"#
    );
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(&init),
        two_server_backend(caps(), caps()),
    );
    rig.ed.settle();

    assert_eq!(
        rig.ed.state.buffers.get(rig.bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 2)
    );
}
