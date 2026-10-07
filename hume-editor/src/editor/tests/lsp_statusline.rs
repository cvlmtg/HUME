// Statusline diagnostics element: `StatusElement::Diagnostics` reads the
// diagnostics store directly (never through Steel) and its loading state
// from the attached LSP server. These tests cover the *data* flow (counts
// and activity state landing correctly on the editor), not the rendered
// glyphs/spacing, which are pinned as inline snapshots in
// `statusline::tests`.

use super::lsp_rig::{LspRig, RigSpec};
use super::*;
use crate::editor::lsp::introspect::LspActivity;
use crate::statusline::StatusElement;
use hume_lsp::client::ClientAction;
use hume_lsp::test_util::RecordingLspBackend;

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn diagnostics(ranges_and_severity: &[DiagFixture]) -> serde_json::Value {
    ranges_and_severity
        .iter()
        .map(|&((sl, sc), (el, ec), severity)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": severity,
                "message": "boom",
            })
        })
        .collect()
}

struct DiagCtx {
    _tmp: tempfile::TempDir,
    ed: Editor,
    sid: hume_lsp::backend::ServerId,
}

/// A `Running` server on a file holding `content`, which then publishes
/// each of `publishes` in order: a later publish replaces an earlier one,
/// the "server republishes with the error fixed" scenario.
fn setup(content: &str, publishes: &[&[DiagFixture]]) -> DiagCtx {
    let tmp = tempfile::tempdir().unwrap();
    let marked = format!("-[{}]>{}", &content[..1], &content[1..]);
    let mut rig = LspRig::rust(
        tmp.path(),
        &marked,
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");
    for diags in publishes {
        rig.publish(sid, diagnostics(diags));
    }
    DiagCtx {
        _tmp: tmp,
        ed: rig.ed,
        sid,
    }
}

#[test]
fn diagnostics_element_empty_with_no_diagnostics() {
    let c = setup("abcdefgh\n", &[]);
    let colors = crate::statusline::colors::EditorColors::default();
    let (text, _) = crate::statusline::render_element(
        &StatusElement::Diagnostics,
        &c.ed.statusline(),
        &colors,
        "",
    );
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

    let colors = crate::statusline::colors::EditorColors::default();
    let (text, _) = crate::statusline::render_element(
        &StatusElement::Diagnostics,
        &c.ed.statusline(),
        &colors,
        "",
    );
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
    let mut ed = open_headless(None);
    let fp = FocusedPane::current(&ed.state);
    crate::editor::commands::typed_set(&mut ed, fp, Some("global statusline=Diagnostics||"), false)
        .unwrap();
    assert_eq!(
        ed.state.settings.statusline().left,
        vec![StatusElement::Diagnostics]
    );
}

/// Counts must track a second, corrected publish for the same file, not
/// just the first snapshot.
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
/// the transport. This exercises `handle_progress`'s handling directly, the
/// same way the other `lsp_*` test files drive typed `ClientAction` variants
/// without a live backend round-trip.
fn progress_action(token: &str, value: serde_json::Value) -> ClientAction {
    ClientAction::Progress(
        serde_json::from_value(serde_json::json!({"token": token, "value": value})).unwrap(),
    )
}

#[test]
fn starting_server_displays_a_loading_indicator() {
    let tmp = tempfile::tempdir().unwrap();
    // No scripted `initialize` response: the server stays `Starting`, so
    // the spinner frame stays at its initial 0.
    let rig = LspRig::open(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n"),
        RecordingLspBackend::new().0,
    );
    let (ed, bid) = (&rig.ed, rig.bid);

    assert!(matches!(ed.lsp_activity(bid), LspActivity::Starting));

    let colors = crate::statusline::colors::EditorColors::default();
    let (text, _) = crate::statusline::render_element(
        &StatusElement::Diagnostics,
        &ed.statusline(),
        &colors,
        "",
    );
    assert!(
        !text.is_empty(),
        "a starting server must display a loading indicator"
    );
}

#[test]
fn progress_begin_report_end_tracks_the_active_task() {
    let mut c = setup("abcdefgh\n", &[]);
    let (sid, bid) = (c.sid, c.ed.focused_buffer_id());
    let ed = &mut c.ed;

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
    assert_eq!(ed.state.lsp.progress_title_for_test(sid), Some("Indexing"));

    // report: percentage arrives; title must persist (merged, not replaced:
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
        ed.state.lsp.progress_title_for_test(sid),
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

/// A `$/progress` begin missing the (lsp_types-required) `title`. Real
/// servers treat it as optional in practice. Drives the *real* transport
/// path (the client's `on_event`, not the `progress_action` helper
/// above, which builds a `ClientAction::Progress` via a strict deserialize
/// that would itself panic on this input) so `classify_notification`'s
/// lenient recovery is what's under test, not a hand-built action.
#[test]
fn progress_begin_missing_title_still_animates_the_spinner() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdefgh\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let (sid, bid) = (rig.sid("rust-analyzer"), rig.bid);
    rig.push(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "$/progress".to_string(),
            params: serde_json::json!({"token": "t1", "value": {"kind": "begin"}}),
        },
    );
    let ed = rig.ed;

    assert!(
        ed.state.lsp.has_animating_server(),
        "a recovered progress task must still animate the spinner"
    );
    assert!(
        matches!(ed.lsp_activity(bid), LspActivity::Progress { .. }),
        "must classify as Progress, not fall through to ServerNotification"
    );
}

/// A server that crashes mid-index must not leave the spinner animating for
/// a task it will never finish: `ClientAction::Crashed` clears the
/// instance's progress, so `activity()` falls through to `Idle` even though
/// the crashed instance stays attached.
#[test]
fn crash_clears_progress_so_the_spinner_stops() {
    let c = setup("abcdefgh\n", &[]);
    let (sid, bid) = (c.sid, c.ed.focused_buffer_id());
    let mut ed = c.ed;

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
    let (sid, bid) = (c.sid, c.ed.focused_buffer_id());
    assert_ne!(
        c.ed.diagnostic_counts(bid),
        (0, 0),
        "fixture must actually carry a count for this to be a meaningful check"
    );

    // `setup` already leaves the client `Running` (see its doc comment);
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
