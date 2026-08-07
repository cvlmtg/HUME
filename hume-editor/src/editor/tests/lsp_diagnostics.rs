// Diagnostics store: ingest, drain-batch
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
    publish_diagnostics_notification_versioned(uri, ranges_and_severity, None)
}

fn publish_diagnostics_notification_versioned(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
    version: Option<i32>,
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
    let mut params = serde_json::json!({ "uri": uri, "diagnostics": diagnostics });
    if let Some(v) = version {
        params["version"] = serde_json::json!(v);
    }
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params,
    }
}

#[test]
fn ingest_converts_utf16_positions_across_an_emoji() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    // "😀 error here\n" — the emoji is 1 Rust char but 2 UTF-16 code units,
    // so a naive char-count read of the wire position would land one
    // character early.
    std::fs::write(&file, "😀 error here\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
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
    let text = ed
        .state
        .buffers
        .get(bid)
        .text()
        .rope()
        .slice(start..end)
        .to_string();
    assert_eq!(
        text, "error",
        "UTF-16 position must land after the emoji, not one char early"
    );
}

#[test]
fn two_publishes_in_one_drain_batch_coalesce_to_the_last() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    // First publish: two errors. Second (same uri, same batch): one warning.
    // Only the second must survive — servers burst-publish and only the
    // newest matters.
    backend.push_from_server(
        sid,
        publish_diagnostics_notification(uri.as_str(), &[((0, 0), (0, 3), 1), ((0, 4), (0, 7), 1)]),
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
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path())
        .unwrap()
        .join("never_opened.rs");
    // Never written to disk / never opened — no buffer will ever match it.

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
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
    assert_eq!(
        entries.len(),
        1,
        "exactly one Trace line, never per-diagnostic spam"
    );
}

#[test]
fn malformed_publish_diagnostics_reaches_the_unhandled_notification_path() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    // `uri` and `diagnostics` both wrong-shaped — fails to parse as
    // `PublishDiagnosticsParams`, so `hume-lsp` classifies it as a
    // `ServerNotification` fallthrough instead of `Diagnostics`.
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({"uri": 42, "diagnostics": "nope"}),
        },
    );
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    open_with_client(&mut ed, &file, sid);

    ed.drain_lsp(); // must not panic

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("unhandled notification textDocument/publishDiagnostics"),
        "expected an unhandled-notification trace line, got: {log}"
    );
}

// ── Minor B — stale-versioned publishes are dropped ────────────────────────

/// Extracts and parses the `params` payload back out of a scripted
/// `publishDiagnostics` notification `Message`, for tests that call
/// `ingest_publish_diagnostics` directly (needed to exercise two separate
/// ingest calls in sequence — batch coalescing would otherwise collapse two
/// same-drain publishes into one before ingest ever saw the first).
fn params_of(msg: hume_lsp::codec::Message) -> lsp_types::PublishDiagnosticsParams {
    match msg {
        hume_lsp::codec::Message::Notification { params, .. } => {
            serde_json::from_value(params).unwrap()
        }
        other => panic!("expected a Notification, got {other:?}"),
    }
}

#[test]
fn publish_with_matching_version_is_ingested() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;

    let params = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_publish_diagnostics(sid, params);

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "a publish whose version matches the buffer's current text_gen must be ingested"
    );
}

#[test]
fn publish_with_a_stale_version_is_dropped_and_does_not_disturb_stored_diagnostics() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;

    // Seed one real (current-version) diagnostic first.
    let seed = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_publish_diagnostics(sid, seed);
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed publish must land"
    );

    // A later publish computed against a version we've already moved past
    // (the server hasn't caught up with our own edits yet) must be dropped
    // — not applied on top of, and not clearing, what's already stored.
    let stale = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 4), (0, 7), 2), ((0, 8), (0, 13), 2)],
        Some(current_gen - 1),
    ));
    ed.ingest_publish_diagnostics(sid, stale);

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "a stale-versioned publish must be dropped, leaving the prior stored diagnostics untouched"
    );
    let entries: Vec<_> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.text.contains("stale version"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "exactly one Trace line for the dropped publish"
    );
}

// ── Minor — stores pruned on buffer close ───────────────────────────────────

/// A `BufferId` is a versioned slotmap key, so a future reused slot can
/// never alias with a closed buffer's stale entries — this is a memory-leak
/// fix, not a correctness one, but nothing else ever freed these.
#[test]
fn close_buffer_prunes_stored_diagnostics_and_decorations() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);

    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;
    let params = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_publish_diagnostics(sid, params);
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![crate::editor::decorations::InlayHintEntry {
            pos: 0,
            text: "x".to_string(),
            before: true,
        }],
    );
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed diagnostic must land"
    );
    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_some(),
        "seed hint must land"
    );

    ed.close_buffer(bid);

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "diagnostics for a closed buffer must not linger forever"
    );
    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_none(),
        "decorations for a closed buffer must not linger forever"
    );
}

/// The Steel `(close-buffer! bid)` entry point must apply the exact same
/// cleanup as `Editor::close_buffer` above — both go through the shared
/// `buffer::lifecycle::close_buffer_and_notify` chokepoint — plus fire
/// `OnBufferClose`, which the direct-Rust-call test above never exercises.
///
/// Fail oracle: revert `EditorHostImpl::close_buffer` to call the bare
/// `lifecycle::close_buffer` (skipping the didClose/diagnostics/decorations/
/// hook-enqueue orchestration) — the diagnostics/decoration assertions below
/// fail, and the log line the hook writes never appears.
#[test]
fn steel_close_buffer_prunes_diagnostics_decorations_and_fires_hook() {
    use hume_scripting::ScriptingHost;

    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);

    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;
    let params = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_publish_diagnostics(sid, params);
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![crate::editor::decorations::InlayHintEntry {
            pos: 0,
            text: "x".to_string(),
            before: true,
        }],
    );
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed diagnostic must land"
    );
    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_some(),
        "seed hint must land"
    );

    // `define-command!` must register into the editor's real `CommandRegistry`
    // for `:go` to dispatch below — a `MockHost` eval (fine for `register-hook!`,
    // which only touches `ScriptingHost`'s own state) leaves it unregistered.
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-close (lambda (bid) (log! 'warn "close-hook-fired")))
           (define-command! "go" "" (lambda () (close-buffer! (current-buffer))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // `bid` is the focused buffer here (opened via `:e` above) — `(current-buffer)`
    // resolves to it, so `:go` needs no path/id embedded in the Steel source.
    type_cmd(&mut ed, ":go");
    // Hooks queued during dispatch fire on an explicit drain, not automatically
    // (`Editor::step`, which `type_cmd` rides, deliberately doesn't drain).
    ed.settle();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "diagnostics for a closed buffer must not linger forever"
    );
    assert!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .next()
            .is_none(),
        "decorations for a closed buffer must not linger forever"
    );
    assert!(
        ed.state
            .message_log
            .format_for_display()
            .contains("close-hook-fired"),
        "OnBufferClose handler must have run"
    );
}

// ── Diagnostics cleared on `:lsp-stop` ─────────────────────────────────

/// Without `DiagnosticsStore::remove_server`, a stopped server's diagnostics
/// stayed rendered forever (squiggles/signs keep showing) and stopped
/// remapping (the buffer is no longer attached, so `flush_lsp_pending_changes`
/// never touches it), drifting silently out of sync with further edits.
#[test]
fn lsp_stop_clears_stored_diagnostics_for_the_detached_buffer() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "one two three four\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);
    ed.lsp.insert_server_key_for_test(
        "rust".to_string(),
        file.parent().unwrap().to_path_buf(),
        sid,
    );
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;
    let params = params_of(publish_diagnostics_notification_versioned(
        uri.as_str(),
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_publish_diagnostics(sid, params);
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed publish must land"
    );

    ed.lsp_stop(Some("rust"));

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "diagnostics from the stopped server must not survive the stop"
    );
}

/// `lsp_stop_one` used to null `buf.lsp_server` and clear `buf.lsp_pending`
/// without first draining it through the decoration remap chokepoint
/// (`flush_lsp_pending_changes` — `lsp_pending` is its only carrier). Any
/// edit queued since the last frame's flush was discarded unremapped,
/// leaving a plugin's sign anchored at its pre-edit position permanently —
/// a detached buffer no longer gets queued for the remap at all, so it
/// never resyncs later either.
#[test]
fn lsp_stop_remaps_a_pending_edit_before_detaching_not_after() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "aa\nbb\ncc\n").unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);
    ed.lsp.insert_server_key_for_test(
        "rust".to_string(),
        file.parent().unwrap().to_path_buf(),
        sid,
    );
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    // "cc"'s line-start char offset in "aa\nbb\ncc\n" is 6.
    ed.state.config.decorations.set_signs(
        "test".to_string(),
        bid,
        vec![crate::editor::decorations::SignEntry {
            pos: 6,
            text: "!".to_string(),
            scope: "x".to_string(),
            priority: 0,
        }],
    );

    // Insert a new first line — shifts "cc" one line down, to a line-start
    // char offset of 8. Deliberately no `ed.settle()`/`drain_lsp()` here:
    // the edit's ChangeSet sits unflushed in `buf.lsp_pending` until
    // `lsp_stop` runs, exactly the race this regression covers.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());

    ed.lsp_stop(Some("rust"));

    assert_eq!(
        ed.state.buffers.get(bid).text().rope().to_string(),
        "X\naa\nbb\ncc\n",
        "sanity: the edit landed"
    );
    let signs = ed.state.config.decorations.signs_for("test", bid);
    assert_eq!(signs.len(), 1);
    assert_eq!(
        signs[0].pos, 8,
        "the sign must follow the edit through the stop, not stay anchored \
         at its pre-edit position"
    );
}

/// Regression: on the minimal 1-char "\n" buffer, `widen_zero_length` has no
/// char to widen a zero-width diagnostic onto in either direction under the
/// general forward/backward rule — it must widen onto the structural
/// newline itself (matching how a selection can cover that same cell) and
/// be stored and counted, not silently dropped from `:lsp-status`.
#[test]
fn zero_width_diagnostic_on_minimal_buffer_is_widened_onto_the_newline() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path()).unwrap().join("main.rs");
    std::fs::write(&file, "\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let bid = open_with_client(&mut ed, &file, sid);
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();

    let params = params_of(publish_diagnostics_notification(
        uri.as_str(),
        &[((0, 0), (0, 0), 1)],
    ));
    ed.ingest_publish_diagnostics(sid, params);

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "an unwidenable zero-width diagnostic must widen onto the newline and be counted"
    );
    assert_eq!(
        ed.lsp.diagnostics_for_test(bid).collect::<Vec<_>>(),
        vec![(0, 1)],
        "the widened diagnostic must also be visible to for_range, not just counts"
    );
}
