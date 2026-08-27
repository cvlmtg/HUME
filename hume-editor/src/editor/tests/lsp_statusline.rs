// Statusline diagnostics element: `StatusElement::Diagnostics` reads the
// diagnostics store directly (never through Steel) and its loading state
// from the attached LSP server. These tests cover the *data* flow — counts
// and activity state landing correctly on the editor — not the rendered
// glyphs/spacing, which are pinned as inline snapshots in
// `ui::statusline::tests`.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::lsp::introspect::LspActivity;
use crate::ui::statusline::StatusElement;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::{ClientAction, LspClient, ServerState};
use hume_lsp::inline::InlineLspBackend;

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn publish_diagnostics_notification(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = ranges_and_severity
        .iter()
        .map(|&((sl, sc), (el, ec), severity)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": severity,
                "message": "boom",
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

struct DiagCtx {
    _file_dir: tempfile::TempDir,
    ed: Editor,
}

/// `publishes`: one or more diagnostic batches for the same file, pushed to
/// the scripted backend in order *before* the single `drain_lsp()` call —
/// multiple publishes in one drain batch coalesce to the last one (same
/// semantics `lsp_diagnostics.rs`'s
/// `two_publishes_in_one_drain_batch_coalesce_to_the_last` locks in), which
/// is exactly the "server republishes with the error fixed" scenario.
fn setup(content: &str, publishes: &[&[DiagFixture]]) -> DiagCtx {
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    for diags in publishes {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    // A server publishing diagnostics is, in reality, always past its
    // handshake — `Running` here so the `Diagnostics` element renders
    // counts rather than the `Starting` loading spinner. `insert_client_for_test`
    // otherwise leaves it at `LspClient::new`'s default `Starting`.
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    DiagCtx {
        _file_dir: file_dir,
        ed,
    }
}

#[test]
fn diagnostics_element_empty_with_no_diagnostics() {
    let c = setup("abcdefgh\n", &[]);
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(&StatusElement::Diagnostics, &c.ed, &colors, "");
    assert!(
        text.is_empty(),
        "expected empty with no diagnostics, got {text:?}"
    );
}

#[test]
fn diagnostics_element_displays_published_error_and_warning_counts() {
    let c = setup(
        "abcdefgh\n",
        &[&[
            ((0, 0), (0, 1), 1),
            ((0, 2), (0, 3), 2),
            ((0, 4), (0, 5), 2),
        ]],
    );
    let bid = c.ed.focused_buffer_id();
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (1, 2),
        "one severity-1 (error) and two severity-2 (warning) diagnostics were published"
    );

    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(&StatusElement::Diagnostics, &c.ed, &colors, "");
    assert!(
        !text.is_empty(),
        "known diagnostic counts must be displayed"
    );
}

#[test]
fn severity_mapping_produces_error_only_and_warning_only_counts() {
    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 1)]]);
    let bid = c.ed.focused_buffer_id();
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (1, 0),
        "severity 1 must map to the error count"
    );

    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 2)]]);
    let bid = c.ed.focused_buffer_id();
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (0, 1),
        "severity 2 must map to the warning count"
    );
}

#[test]
fn configure_statusline_round_trips_diagnostics_element_name() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    crate::editor::commands::typed_set(&mut ed, Some("global statusline=Diagnostics||"), false)
        .unwrap();
    assert_eq!(
        ed.state.settings.statusline.left,
        vec![StatusElement::Diagnostics]
    );
}

/// Tier 2: counts must track a second, corrected publish for the same file —
/// not just the first snapshot. Both publishes are queued before the single
/// `drain_lsp()` call (see `setup`'s doc comment).
#[test]
fn diagnostic_counts_update_across_a_corrected_publish() {
    let c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 1)], &[]]);
    let bid = c.ed.focused_buffer_id();
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (0, 0),
        "the corrected (empty) publish must replace the stale error count"
    );
}

// ── Loading spinner (Starting / $/progress) ───────────────────────────────

/// A `$/progress` notification action for `dispatch_lsp_action`, bypassing
/// the transport — this exercises `handle_progress`'s handling directly, the
/// same way the other `lsp_*` test files drive typed `ClientAction` variants
/// without a live backend round-trip.
fn progress_action(token: &str, value: serde_json::Value) -> ClientAction {
    ClientAction::Progress(
        serde_json::from_value(serde_json::json!({"token": token, "value": value})).unwrap(),
    )
}

#[test]
fn starting_server_displays_a_loading_indicator() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    // `LspClient::new` defaults to `Starting` — exactly like a real server
    // between spawn and `initialize` completing. No `drain_lsp()` call here,
    // so the spinner frame stays at its initial 0.
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    assert!(matches!(ed.lsp_activity(bid), LspActivity::Starting));

    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) =
        crate::ui::statusline::render_element(&StatusElement::Diagnostics, &ed, &colors, "");
    assert!(
        !text.is_empty(),
        "a starting server must display a loading indicator"
    );
}

#[test]
fn progress_begin_report_end_tracks_the_active_task() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    assert!(
        matches!(ed.lsp_activity(bid), LspActivity::Idle),
        "a Running server with no progress task is idle"
    );

    ed.dispatch_lsp_action(
        sid,
        progress_action(
            "t1",
            serde_json::json!({"kind": "begin", "title": "Indexing"}),
        ),
    );
    match ed.lsp_activity(bid) {
        LspActivity::Progress { percentage } => {
            assert_eq!(percentage, None, "begin carried no percentage");
        }
        _ => panic!("expected Progress after begin"),
    }
    assert_eq!(ed.lsp.progress_title_for_test(sid), Some("Indexing"));

    // report: percentage arrives; title must persist (merged, not replaced —
    // an absent field means "unchanged" per the LSP spec).
    ed.dispatch_lsp_action(
        sid,
        progress_action(
            "t1",
            serde_json::json!({"kind": "report", "percentage": 45}),
        ),
    );
    match ed.lsp_activity(bid) {
        LspActivity::Progress { percentage } => {
            assert_eq!(percentage, Some(45));
        }
        _ => panic!("expected Progress after report"),
    }
    assert_eq!(
        ed.lsp.progress_title_for_test(sid),
        Some("Indexing"),
        "title must survive an unrelated report"
    );

    ed.dispatch_lsp_action(
        sid,
        progress_action("t1", serde_json::json!({"kind": "end"})),
    );
    assert!(
        matches!(ed.lsp_activity(bid), LspActivity::Idle),
        "the task must be dropped once its `end` arrives"
    );
}

/// A `$/progress` begin missing the (lsp_types-required) `title` — real
/// servers treat it as optional in practice. Drives the *real* transport
/// path (`push_from_server` + `drain_lsp`, not the `progress_action` helper
/// above, which builds a `ClientAction::Progress` via a strict deserialize
/// that would itself panic on this input) so `classify_notification`'s
/// lenient recovery is what's under test, not a hand-built action.
#[test]
fn progress_begin_missing_title_still_animates_the_spinner() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "$/progress".to_string(),
            params: serde_json::json!({"token": "t1", "value": {"kind": "begin"}}),
        },
    );

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    ed.drain_lsp();

    assert!(
        ed.lsp.has_animating_server(),
        "a recovered progress task must still animate the spinner"
    );
    assert!(
        matches!(ed.lsp_activity(bid), LspActivity::Progress { .. }),
        "must classify as Progress, not fall through to ServerNotification"
    );
}

/// A server that crashes mid-index must not leave the spinner animating for
/// a task it will never finish — `ClientAction::Crashed` clears
/// `ServerEntry.progress`, so `activity()` falls through to `Idle` even
/// though the entry (unlike `:lsp-stop`'s teardown) stays in `lsp.servers`.
#[test]
fn crash_clears_progress_so_the_spinner_stops() {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    ed.dispatch_lsp_action(
        sid,
        progress_action(
            "t1",
            serde_json::json!({"kind": "begin", "title": "Indexing"}),
        ),
    );
    assert!(matches!(ed.lsp_activity(bid), LspActivity::Progress { .. }));

    ed.dispatch_lsp_action(
        sid,
        ClientAction::Crashed {
            error: Some("boom".to_string()),
        },
    );
    assert!(
        matches!(ed.lsp_activity(bid), LspActivity::Idle),
        "a crashed server's leftover progress must not keep the spinner going"
    );
}

#[test]
fn loading_state_keeps_diagnostic_counts_available() {
    let mut c = setup("abcdefgh\n", &[&[((0, 0), (0, 1), 1)]]);
    let bid = c.ed.focused_buffer_id();
    let sid =
        c.ed.state
            .buffers
            .get(bid)
            .lsp_server
            .expect("setup attaches a server");
    assert_ne!(
        c.ed.diagnostic_counts(bid),
        (0, 0),
        "fixture must actually carry a count for this to be a meaningful check"
    );

    // `setup` already leaves the client `Running` (see its doc comment) —
    // the server is now (re)loading, e.g. mid `:lsp-restart` reindex.
    c.ed.dispatch_lsp_action(
        sid,
        progress_action(
            "t1",
            serde_json::json!({"kind": "begin", "title": "Indexing"}),
        ),
    );

    assert!(
        matches!(c.ed.lsp_activity(bid), LspActivity::Progress { .. }),
        "a begun progress task must be reflected in the activity state"
    );
    assert_eq!(
        c.ed.diagnostic_counts(bid),
        (1, 0),
        "a background progress task must not clear already-known diagnostic counts"
    );
}
