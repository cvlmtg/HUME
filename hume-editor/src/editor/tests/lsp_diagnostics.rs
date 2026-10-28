// Diagnostics store: ingest, drain-batch
// coalescing, and the unknown-URI drop path. Remap/counts/for_range are
// unit-tested directly in `editor::lsp::diagnostics` (no Editor needed
// there); this file covers the parts that need a real buffer + backend.

use super::lsp_rig::{LspRig, RigSpec, TWO_RUST_SERVERS, answering, stop_server};
use super::*;
use hume_core::{BufferId, DiagSeverity};
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

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

/// `text` with its first char selected, as `RigSpec` takes it.
fn marked(text: &str) -> String {
    let first = text.chars().next().expect("non-empty text");
    format!("-[{first}]>{}", &text[first.len_utf8()..])
}

/// The URI of `name` under the rig's root, before the rig exists.
fn uri_under(tmp: &tempfile::TempDir, name: &str) -> String {
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    hume_lsp::uri::path_to_uri(&root.join(name))
        .unwrap()
        .as_str()
        .to_string()
}

/// A `rust-analyzer` server (UTF-16, no capabilities) on `src/main.rs`
/// holding `text`, drained once. `script` queues server traffic before the
/// file opens, so it arrives in the same drain batch as the handshake;
/// `ServerId(0)` is the one server's id.
fn rig_with(
    tmp: &tempfile::TempDir,
    text: &str,
    script: impl FnOnce(&mut RecordingLspBackend),
) -> LspRig {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    script(&mut backend);
    LspRig::drained(tmp.path(), RigSpec::rust(&marked(text)), backend)
}

/// [`rig_with`] with no extra server traffic, plus its server, buffer and
/// URI.
fn plain_rig(tmp: &tempfile::TempDir, text: &str) -> (LspRig, ServerId, BufferId, String) {
    let rig = rig_with(tmp, text, |_| {});
    let (sid, bid, uri) = (rig.sid("rust-analyzer"), rig.bid, rig.uri());
    (rig, sid, bid, uri)
}

#[test]
fn ingest_converts_utf16_positions_across_an_emoji() {
    let tmp = tempfile::tempdir().unwrap();
    let uri = uri_under(&tmp, "src/main.rs");
    // "😀 error here\n": the emoji is 1 Rust char but 2 UTF-16 code units,
    // so a naive char-count read of the wire position would land one
    // character early. UTF-16 units: 0-1 = emoji, 2 = space, 3..8 = "error".
    let rig = rig_with(&tmp, "😀 error here\n", |b| {
        b.push_from_server(
            ServerId(0),
            publish_diagnostics_notification(&uri, &[((0, 3), (0, 8), 1)]),
        );
    });
    let (ed, bid) = (&rig.ed, rig.bid);

    let stored: Vec<(usize, usize)> = ed
        .state
        .buffer_positions
        .diagnostics
        .spans_for_test(bid)
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
    let tmp = tempfile::tempdir().unwrap();
    let uri = uri_under(&tmp, "src/main.rs");
    // First publish: two errors. Second (same uri, same batch): one warning.
    // Only the second must survive: servers burst-publish and only the
    // newest matters.
    let rig = rig_with(&tmp, "one two three four\n", |b| {
        b.push_from_server(
            ServerId(0),
            publish_diagnostics_notification(&uri, &[((0, 0), (0, 3), 1), ((0, 4), (0, 7), 1)]),
        );
        b.push_from_server(
            ServerId(0),
            publish_diagnostics_notification(&uri, &[((0, 8), (0, 13), 2)]),
        );
    });

    assert_eq!(
        rig.ed.diagnostic_counts(rig.bid),
        (0, 1),
        "only the second (later) publish in the batch must survive"
    );
}

#[test]
fn publish_for_an_unopened_file_is_dropped_without_spam() {
    let tmp = tempfile::tempdir().unwrap();
    // Never written to disk / never opened: no buffer will ever match it.
    let uri = uri_under(&tmp, "never_opened.rs");
    let rig = rig_with(&tmp, "x\n", |b| {
        b.push_from_server(
            ServerId(0),
            publish_diagnostics_notification(&uri, &[((0, 0), (0, 1), 1)]),
        );
    });

    let entries: Vec<_> = rig
        .ed
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
    let tmp = tempfile::tempdir().unwrap();
    // `uri` and `diagnostics` both wrong-shaped, so it fails to parse as
    // `PublishDiagnosticsParams`, so `hume-lsp` classifies it as a
    // `ServerNotification` fallthrough instead of `Diagnostics`.
    let mut rig = rig_with(&tmp, "one two three four\n", |b| {
        b.push_from_server(
            ServerId(0),
            hume_lsp::codec::Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": 42, "diagnostics": "nope"}),
            },
        );
    });
    rig.ed.settle();

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains("unhandled notification textDocument/publishDiagnostics"),
        "expected an unhandled-notification trace line, got: {log}"
    );
}

#[test]
fn a_stored_diagnostic_keeps_the_wire_value_a_server_sent() {
    let tmp = tempfile::tempdir().unwrap();
    let uri = uri_under(&tmp, "src/main.rs");
    let mut rig = rig_with(&tmp, "one two three\n", |b| {
        b.push_from_server(
            ServerId(0),
            hume_lsp::codec::Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({
                    "uri": uri,
                    "diagnostics": [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 3},
                        },
                        "severity": 1,
                        "message": "boom",
                        "x-ext": {"keep": true},
                    }],
                }),
            },
        );
    });

    let entries = crate::editor::lsp::diagnostics::diagnostics_for_buffer(
        &mut rig.ed.state,
        rig.bid,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        entries[0].raw.get("x-ext"),
        Some(&serde_json::json!({"keep": true}))
    );
}

// ── A diagnostic that does not parse ───────────────────────────────────────

fn skipped_warnings(ed: &Editor) -> usize {
    ed.state
        .message_log
        .entries()
        .filter(|entry| {
            entry.severity == hume_core::Severity::Warning && entry.text.contains("do not parse")
        })
        .count()
}

#[test]
fn a_diagnostic_that_does_not_parse_does_not_hide_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let uri = uri_under(&tmp, "src/main.rs");
    let diagnostic = |severity: serde_json::Value| {
        serde_json::json!({
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 0, "character": 3},
            },
            "severity": severity,
            "message": "boom",
        })
    };
    let rig = rig_with(&tmp, "one two three\n", |b| {
        b.push_from_server(
            ServerId(0),
            hume_lsp::codec::Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({
                    "uri": uri,
                    "diagnostics": [
                        diagnostic(serde_json::json!(1)),
                        diagnostic(serde_json::json!("loud")),
                        diagnostic(serde_json::json!(1)),
                    ],
                }),
            },
        );
    });

    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (2, 0));
    assert_eq!(skipped_warnings(&rig.ed), 1);
    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains("'rust-analyzer' sent 1 diagnostic(s)"),
        "the warning names the server and the count: {log}"
    );
}

#[test]
fn skipped_diagnostics_are_warned_about_once_per_server() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, _bid, uri) = plain_rig(&tmp, "one two three\n");
    let publish = || hume_lsp::diagnostic::PublishedDiagnostics {
        uri: uri.parse().unwrap(),
        version: None,
        diagnostics: Vec::new(),
        skipped: vec!["invalid type".to_string()],
    };

    rig.ed
        .dispatch_lsp_action(sid, hume_lsp::client::ClientAction::Diagnostics(publish()));
    rig.ed
        .dispatch_lsp_action(sid, hume_lsp::client::ClientAction::Diagnostics(publish()));

    assert_eq!(skipped_warnings(&rig.ed), 1);
}

// ── Path resolution is bounded per drain ───────────────────────────────────

/// A rig whose first drain holds `misses` publishes for files that do not
/// exist and one for the open buffer.
fn rig_with_unknown_file_publishes(tmp: &tempfile::TempDir, misses: usize) -> LspRig {
    let open_uri = uri_under(tmp, "src/main.rs");
    rig_with(tmp, "one two three\n", |b| {
        for i in 0..misses {
            let uri = uri_under(tmp, &format!("missing/f{i}.rs"));
            b.push_from_server(
                ServerId(0),
                publish_diagnostics_notification(&uri, &[((0, 0), (0, 3), 1)]),
            );
        }
        b.push_from_server(
            ServerId(0),
            publish_diagnostics_notification(&open_uri, &[((0, 0), (0, 3), 1)]),
        );
    })
}

#[test]
fn publishes_for_unknown_files_resolve_in_bounded_chunks() {
    use crate::editor::async_source::AsyncSource;
    use crate::editor::lsp::diagnostics::CANONICALIZE_PER_DRAIN;

    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rig_with_unknown_file_publishes(&tmp, CANONICALIZE_PER_DRAIN + 6);

    assert_eq!(
        rig.ed.diagnostic_counts(rig.bid),
        (1, 0),
        "the open buffer's publish spends none of the budget"
    );
    assert_eq!(rig.ed.state.lsp.deferred_publishes_for_test(), 6);
    assert!(
        rig.ed
            .state
            .lsp
            .next_wake(std::time::Instant::now())
            .is_some(),
        "the loop must come back for what is deferred"
    );

    rig.ed.drain_lsp();

    assert_eq!(rig.ed.state.lsp.deferred_publishes_for_test(), 0);
    assert!(
        rig.ed
            .state
            .lsp
            .next_wake(std::time::Instant::now())
            .is_none()
    );
}

#[test]
fn stopping_a_server_drops_its_deferred_publishes() {
    use crate::editor::lsp::diagnostics::CANONICALIZE_PER_DRAIN;

    let tmp = tempfile::tempdir().unwrap();
    let mut rig = rig_with_unknown_file_publishes(&tmp, CANONICALIZE_PER_DRAIN + 6);
    assert_eq!(rig.ed.state.lsp.deferred_publishes_for_test(), 6);

    stop_server(&mut rig.ed, "rust-analyzer");

    assert_eq!(rig.ed.state.lsp.deferred_publishes_for_test(), 0);
}

#[test]
fn a_publish_for_a_buffer_whose_file_is_not_on_disk_is_ingested() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three\n");
    std::fs::remove_file(rig.root.join("src/main.rs")).unwrap();

    let params = params_of(publish_diagnostics_notification(
        &uri,
        &[((0, 0), (0, 3), 1)],
    ));
    rig.ed.ingest_typed_publish_for_test(sid, params);

    assert_eq!(
        rig.ed.diagnostic_counts(bid),
        (1, 0),
        "a buffer with no file on disk yet still shows its server's diagnostics"
    );
}

// ── Stale-versioned publishes are dropped ──────────────────────────────────

#[test]
fn publish_with_matching_version_is_ingested() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three four\n");
    let ed = &mut rig.ed;
    let current_gen = ed.state.buffers.get(bid).text().generation() as i32;

    let params = params_of(publish_diagnostics_notification_versioned(
        &uri,
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_typed_publish_for_test(sid, params);

    assert_eq!(
        ed.diagnostic_counts(bid),
        (1, 0),
        "a publish whose version matches the buffer's current generation must be ingested"
    );
}

#[test]
fn publish_with_a_stale_version_is_dropped_and_does_not_disturb_stored_diagnostics() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three four\n");
    let ed = &mut rig.ed;
    let current_gen = ed.state.buffers.get(bid).text().generation() as i32;

    // Seed one real (current-version) diagnostic first.
    let seed = params_of(publish_diagnostics_notification_versioned(
        &uri,
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_typed_publish_for_test(sid, seed);
    assert_eq!(ed.diagnostic_counts(bid), (1, 0), "seed publish must land");

    // A later publish computed against a version we've already moved past
    // (the server hasn't caught up with our own edits yet) must be dropped,
    // not applied on top of, and not clearing, what's already stored.
    let stale = params_of(publish_diagnostics_notification_versioned(
        &uri,
        &[((0, 4), (0, 7), 2), ((0, 8), (0, 13), 2)],
        Some(current_gen - 1),
    ));
    ed.ingest_typed_publish_for_test(sid, stale);

    assert_eq!(
        ed.diagnostic_counts(bid),
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

// ── Stores pruned on buffer close ───────────────────────────────────────────

/// Seeds one diagnostic from `sid` and one inlay hint on `bid`, the state
/// both close tests below expect gone afterwards.
fn seed_diagnostic_and_hint(ed: &mut Editor, sid: ServerId, bid: BufferId, uri: &str) {
    let current_gen = ed.state.buffers.get(bid).text().generation() as i32;
    let params = params_of(publish_diagnostics_notification_versioned(
        uri,
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_typed_publish_for_test(sid, params);
    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![hume_decorations::InlayHintEntry {
            pos: co(0),
            text: "x".to_string(),
            side: hume_core::Side::Before,
        }],
    );
    assert_eq!(
        ed.diagnostic_counts(bid),
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
}

/// A `BufferId` is a versioned slotmap key, so a future reused slot can
/// never alias with a closed buffer's stale entries. This is a memory-leak
/// fix, not a correctness one, but nothing else ever freed these.
#[test]
fn close_buffer_prunes_stored_diagnostics_and_decorations() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three\n");
    let ed = &mut rig.ed;
    seed_diagnostic_and_hint(ed, sid, bid, &uri);

    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");

    assert_eq!(
        ed.diagnostic_counts(bid),
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
/// cleanup as `Editor::close_buffer` above (both go through the shared
/// `buffer::lifecycle::close_buffer_and_notify` chokepoint), plus fire
/// `OnBufferClose`, which the direct-Rust-call test above never exercises.
///
/// If `EditorHostImpl::close_buffer` called the bare `lifecycle::close_buffer`,
/// it would skip the didClose, diagnostics, decorations, and hook steps. The
/// assertions below and the hook's log line would all fail.
#[test]
fn steel_close_buffer_prunes_diagnostics_decorations_and_fires_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three\n");
    seed_diagnostic_and_hint(&mut rig.ed, sid, bid, &uri);
    rig.eval(
        r#"(register-hook! 'on-buffer-close (lambda (bid) (log! 'warning "close-hook-fired")))
           (define-typed-command! "go" "" (lambda (bid) (close-buffer! bid)))"#,
    );

    // `bid` is the focused buffer here, and `:go` receives it as its own
    // leading parameter.
    type_cmd(&mut rig.ed, ":go");
    // Hooks queued during dispatch fire on an explicit drain, not automatically
    // (`Editor::step`, which `type_cmd` rides, doesn't drain).
    rig.ed.settle();

    let ed = &rig.ed;
    assert_eq!(
        ed.diagnostic_counts(bid),
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
    assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1);
}

// ── Diagnostics cleared on `:lsp-stop` ─────────────────────────────────

/// A stopped server's diagnostics must not stay rendered (squiggles and
/// signs keep showing) for a buffer no server reports on any more.
#[test]
fn lsp_stop_clears_stored_diagnostics_for_the_detached_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "one two three four\n");
    let ed = &mut rig.ed;
    let current_gen = ed.state.buffers.get(bid).text().generation() as i32;
    let params = params_of(publish_diagnostics_notification_versioned(
        &uri,
        &[((0, 0), (0, 3), 1)],
        Some(current_gen),
    ));
    ed.ingest_typed_publish_for_test(sid, params);
    assert_eq!(ed.diagnostic_counts(bid), (1, 0), "seed publish must land");

    stop_server(ed, "rust-analyzer");

    assert_eq!(
        ed.diagnostic_counts(bid),
        (0, 0),
        "diagnostics from the stopped server must not survive the stop"
    );
}

/// A decoration follows an edit made just before a stop: the stop neither
/// drops it nor freezes it at its pre-edit position.
#[test]
fn lsp_stop_keeps_a_sign_where_a_preceding_edit_moved_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, _sid, bid, _uri) = plain_rig(&tmp, "aa\nbb\ncc\n");
    let ed = &mut rig.ed;

    let scope = ed.view.registry.intern_runtime("x");
    ed.state.config.decorations.set_signs(
        "test".to_string(),
        bid,
        vec![hume_decorations::SignEntry {
            line: hume_rope::line::ContentLine::new(2),
            text: "!".into(),
            scope,
        }],
    );

    // Insert a new first line, shifting "cc" one line down, to line 3. No
    // drain: the change is still queued for the server
    // when the stop runs.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_enter());
    ed.feed_key(key_esc());

    stop_server(ed, "rust-analyzer");

    assert_eq!(
        ed.state.buffers.get(bid).text().rope().to_string(),
        "X\naa\nbb\ncc\n",
        "sanity: the edit landed"
    );
    let signs = ed.state.config.decorations.signs_for("test", bid);
    assert_eq!(signs.len(), 1);
    assert_eq!(
        signs[0].line,
        hume_rope::line::ContentLine::new(3),
        "the sign must follow the edit through the stop, not stay anchored \
         at its pre-edit position"
    );
}

/// On the minimal 1-char "\n" buffer, `widen_zero_length` has no char to
/// widen a zero-width diagnostic onto in either direction under the general
/// forward/backward rule: it must widen onto the structural newline itself
/// (matching how a selection can cover that same cell) and be stored and
/// counted, not dropped from `:lsp-status`.
#[test]
fn zero_width_diagnostic_on_minimal_buffer_is_widened_onto_the_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, bid, uri) = plain_rig(&tmp, "\n");
    let ed = &mut rig.ed;

    let params = params_of(publish_diagnostics_notification(
        &uri,
        &[((0, 0), (0, 0), 1)],
    ));
    ed.ingest_typed_publish_for_test(sid, params);

    assert_eq!(
        ed.diagnostic_counts(bid),
        (1, 0),
        "an unwidenable zero-width diagnostic must widen onto the newline and be counted"
    );
    assert_eq!(
        ed.state
            .buffer_positions
            .diagnostics
            .spans_for_test(bid)
            .collect::<Vec<_>>(),
        vec![(0, 1)],
        "the widened diagnostic must also be visible to for_range, not just counts"
    );
}

// ── The diagnostics-for-buffer read cap ─────────────────────────────────────

/// The `:messages` traces saying `diagnostics-for-buffer` cut a buffer's
/// diagnostics.
fn read_cap_traces(ed: &Editor) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text.contains("returns the first 1000"))
        .map(|e| e.text.clone())
        .collect()
}

/// `n` one-char diagnostics of wire `severity` on line 0.
fn n_diagnostics(n: u32, severity: i64) -> Vec<DiagFixture> {
    (0..n).map(|_| ((0, 0), (0, 1), severity)).collect()
}

/// A publish of `n` one-char errors on line 0 of `uri`.
fn publish_n(uri: &str, n: u32) -> lsp_types::PublishDiagnosticsParams {
    params_of(publish_diagnostics_notification(uri, &n_diagnostics(n, 1)))
}

/// A `diagnostics-for-buffer` read of the rig's buffer at `floor` over the
/// char range `range`; how many diagnostics it returned.
fn read(rig: &mut LspRig, floor: Option<DiagSeverity>, range: Option<(usize, usize)>) -> usize {
    crate::editor::lsp::diagnostics::diagnostics_for_buffer(
        &mut rig.ed.state,
        rig.bid,
        floor,
        range,
    )
    .unwrap()
    .len()
}

/// A read past the cap is traced once per crossing: not on the publish, not
/// again while the buffer stays over, again after a read finds it back under.
#[test]
fn a_read_past_the_cap_is_traced_once_per_crossing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, _bid, uri) = plain_rig(&tmp, "one two three\n");

    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 1001));
    assert_eq!(
        read_cap_traces(&rig.ed),
        Vec::<String>::new(),
        "nothing read yet"
    );

    assert_eq!(read(&mut rig, None, None), 1000);
    let traces = read_cap_traces(&rig.ed);
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert!(
        traces[0]
            .contains("main.rs: diagnostics-for-buffer matched 1001 diagnostics at hint or above"),
        "{traces:?}"
    );

    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 1002));
    read(&mut rig, None, None);
    assert_eq!(read_cap_traces(&rig.ed).len(), 1, "still over the cap");

    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 10));
    read(&mut rig, None, None);
    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 1001));
    read(&mut rig, None, None);
    assert_eq!(read_cap_traces(&rig.ed).len(), 2, "a second crossing");
}

/// A read counts at its own floor: diagnostics below it never reach its
/// cap, and a `#:severity` read below the setting's floor is traced at
/// that floor.
#[test]
fn a_read_is_traced_at_its_own_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, _bid, uri) = plain_rig(&tmp, "one two three\n");
    rig.ed.state.settings.lsp_diagnostics_min_severity = DiagSeverity::Warning;
    let mut fixtures = n_diagnostics(1500, 4);
    fixtures.extend(n_diagnostics(5, 1));
    rig.ed.ingest_typed_publish_for_test(
        sid,
        params_of(publish_diagnostics_notification(&uri, &fixtures)),
    );

    assert_eq!(read(&mut rig, None, None), 5);
    assert_eq!(read_cap_traces(&rig.ed), Vec::<String>::new());

    assert_eq!(read(&mut rig, Some(DiagSeverity::Hint), None), 1000);
    let traces = read_cap_traces(&rig.ed);
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert!(
        traces[0].contains("1505 diagnostics at hint or above"),
        "{traces:?}"
    );
}

/// Lowering `lsp-diagnostics-min-severity` past the cap is traced by the
/// next read at the new floor.
#[test]
fn a_lowered_floor_is_traced_by_the_next_read() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, _bid, uri) = plain_rig(&tmp, "one two three\n");
    rig.ed.state.settings.lsp_diagnostics_min_severity = DiagSeverity::Error;
    rig.ed.ingest_typed_publish_for_test(
        sid,
        params_of(publish_diagnostics_notification(
            &uri,
            &n_diagnostics(1200, 2),
        )),
    );
    read(&mut rig, None, None);
    assert_eq!(read_cap_traces(&rig.ed), Vec::<String>::new());

    rig.ed.state.settings.lsp_diagnostics_min_severity = DiagSeverity::Warning;
    read(&mut rig, None, None);
    let traces = read_cap_traces(&rig.ed);
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert!(
        traces[0].contains("1200 diagnostics at warning or above"),
        "{traces:?}"
    );
}

/// A `#:range` read the cap cuts is traced. One it does not cut says
/// nothing about the rest of the buffer, so only a whole-buffer read under
/// the cap re-arms the trace.
#[test]
fn a_ranged_read_is_traced_but_does_not_re_arm() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut rig, sid, _bid, uri) = plain_rig(&tmp, "one two three\n");
    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 1001));

    assert_eq!(read(&mut rig, None, Some((0, 3))), 1000);
    assert_eq!(read_cap_traces(&rig.ed).len(), 1);

    assert_eq!(read(&mut rig, None, Some((4, 7))), 0);
    read(&mut rig, None, Some((0, 3)));
    read(&mut rig, None, None);
    assert_eq!(read_cap_traces(&rig.ed).len(), 1, "still over the cap");

    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 10));
    read(&mut rig, None, None);
    rig.ed
        .ingest_typed_publish_for_test(sid, publish_n(&uri, 1001));
    assert_eq!(read_cap_traces(&rig.ed).len(), 1, "not read since");
    read(&mut rig, None, Some((0, 3)));
    assert_eq!(read_cap_traces(&rig.ed).len(), 2, "a second crossing");
}

/// One buffer's cap counts every server's diagnostics together.
#[test]
fn every_servers_diagnostics_count_toward_the_read_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = answering(2);
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust(&marked("one two three\n")).with_init(TWO_RUST_SERVERS),
        backend,
    );
    let (ra, lint, uri) = (rig.sid("rust-analyzer"), rig.sid("ra-lint"), rig.uri());

    rig.ed
        .ingest_typed_publish_for_test(ra, publish_n(&uri, 600));
    read(&mut rig, None, None);
    assert_eq!(read_cap_traces(&rig.ed).len(), 0, "600 is under the cap");

    rig.ed
        .ingest_typed_publish_for_test(lint, publish_n(&uri, 401));
    read(&mut rig, None, None);
    let traces = read_cap_traces(&rig.ed);
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert!(traces[0].contains("matched 1001 diagnostics"), "{traces:?}");
}
