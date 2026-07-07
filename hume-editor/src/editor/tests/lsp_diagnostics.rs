// C9 (docs/lsp/step-1.md) — diagnostics store: ingest, drain-batch
// coalescing, and the unknown-URI drop path. Remap/counts/for_range are
// unit-tested directly in `editor::lsp::diagnostics` (no Editor needed
// there); this file covers the parts that need a real buffer + backend.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_engine::pipeline::BufferId;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;

/// Opens `file` (already written to disk) and wires a client for `sid` so
/// `drain_lsp` routes its events instead of dropping them (`on_event` is
/// only reached for servers with a tracked client).
fn open_with_client(ed: &mut Editor, file: &Path, sid: ServerId) -> BufferId {
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, file.parent().unwrap().to_path_buf()));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.focused_buffer_id()
}

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn publish_diagnostics_notification(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = ranges_and_severity
        .iter()
        .map(|((sl, sc), (el, ec), sev)| {
            serde_json::json!({
                "range": {
                    "start": {"line": sl, "character": sc},
                    "end": {"line": el, "character": ec},
                },
                "severity": sev,
                "message": "boom",
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({ "uri": uri, "diagnostics": diagnostics }),
    }
}

#[test]
fn ingest_converts_utf16_positions_across_an_emoji() {
    let tmp = tempfile::tempdir().unwrap();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    // "😀 error here\n" — the emoji is 1 Rust char but 2 UTF-16 code units,
    // so a naive char-count read of the wire position would land one
    // character early.
    std::fs::write(&file, "😀 error here\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    // UTF-16 units: 0-1 = emoji, 2 = space, 3..8 = "error".
    backend.push_from_server(
        sid,
        publish_diagnostics_notification(uri.as_str(), &[((0, 3), (0, 8), 1)]),
    );
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let bid = open_with_client(&mut ed, &file, sid);
    ed.drain_lsp();

    let stored: Vec<(usize, usize)> = ed
        .lsp
        .diagnostics_for_test(bid)
        .map(|d| (d.0, d.1))
        .collect();
    assert_eq!(stored.len(), 1);
    let (start, end) = stored[0];
    let text = ed.state.buffers.get(bid).text().rope().slice(start..end).to_string();
    assert_eq!(text, "error", "UTF-16 position must land after the emoji, not one char early");
}

#[test]
fn two_publishes_in_one_drain_batch_coalesce_to_the_last() {
    let tmp = tempfile::tempdir().unwrap();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    // First publish: two errors. Second (same uri, same batch): one warning.
    // Only the second must survive — servers burst-publish and only the
    // newest matters.
    backend.push_from_server(
        sid,
        publish_diagnostics_notification(
            uri.as_str(),
            &[((0, 0), (0, 3), 1), ((0, 4), (0, 7), 1)],
        ),
    );
    backend.push_from_server(
        sid,
        publish_diagnostics_notification(uri.as_str(), &[((0, 8), (0, 13), 2)]),
    );
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let bid = open_with_client(&mut ed, &file, sid);
    ed.drain_lsp();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 1),
        "only the second (later) publish in the batch must survive"
    );
}

#[test]
fn publish_for_an_unopened_file_is_dropped_without_spam() {
    let tmp = tempfile::tempdir().unwrap();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("never_opened.rs");
    // Never written to disk / never opened — no buffer will ever match it.

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    backend.push_from_server(
        sid,
        publish_diagnostics_notification(uri.as_str(), &[((0, 0), (0, 1), 1)]),
    );
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, tmp.path().to_path_buf()));

    ed.drain_lsp();

    let entries: Vec<_> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.text.contains("publishDiagnostics"))
        .collect();
    assert_eq!(entries.len(), 1, "exactly one Trace line, never per-diagnostic spam");
}
