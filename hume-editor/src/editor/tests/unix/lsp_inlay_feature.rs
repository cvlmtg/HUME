// Inlay hints: debounced textDocument/inlayHint on viewport change and
// diagnostics change, composing `lsp-request!`, `lsp-capabilities`, debounce,
// `set-inlay-hints!`, `on-viewport-change`, `on-diagnostics-changed`,
// and rendering (not tested here, its own pinned snapshots cover that).
// Named lsp_inlay_feature.rs because lsp_inlay_hints.rs already covers rendering
// of the decoration store directly; this file drives the same store through
// the real shipped plugin and a real LSP round trip. Loads the real shipped
// `core:lsp` plugin in place (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_engine::pipeline::{BufferId, RenderContext};
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};

/// The rig's file, "let x = 1;\n", with the cursor at its start.
const MARKED_FIXTURE: &str = "-[l]>et x = 1;\n";

/// A [`core_lsp_rig`] over [`MARKED_FIXTURE`] with an inlay-hint provider.
fn setup(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs, RequestLog) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        MARKED_FIXTURE,
        serde_json::json!({"capabilities": {"inlayHintProvider": true}}),
        configure,
    );
    (rig.ed, guard, rig.requests)
}

/// Everything [`setup_two_servers`] builds: the rig's `src/main.rs`
/// (`bid_a`, on `rust-analyzer`) focused, and `main.py` (`bid_b`, on
/// `pylsp`) shown in `pane_b` beside it.
struct TwoServers {
    ed: Editor,
    _guard: RealRuntimeDirs,
    requests: RequestLog,
    sid_a: ServerId,
    sid_b: ServerId,
    bid_a: BufferId,
    bid_b: BufferId,
    pane_b: hume_engine::pipeline::PaneId,
}

/// Two buffers, each attached to its own server: `rust-analyzer` answers
/// `initialize` with `initialize_a`, `pylsp` with `initialize_b`, and the
/// shared FIFO holds `inlay_answers` empty `textDocument/inlayHint` results.
fn setup_two_servers(
    tmp: &Path,
    initialize_a: serde_json::Value,
    initialize_b: serde_json::Value,
    inlay_answers: usize,
) -> TwoServers {
    let guard = RealRuntimeDirs::new();
    let (mut backend, _notifications, _requests) = RecordingLspBackend::new();
    backend.respond_to_server(ServerId(0), "initialize", initialize_a);
    backend.respond_to_server(ServerId(1), "initialize", initialize_b);
    for _ in 0..inlay_answers {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    }
    let init = format!(
        "{}\n{}",
        core_lsp_init(),
        r#"(register-lsp-server! "pylsp" #:command "pylsp")
(set-language-servers! "python" '("pylsp"))"#
    );
    let mut rig = LspRig::drained(
        tmp,
        RigSpec::rust(MARKED_FIXTURE)
            .with_init(&init)
            .with_dirs(guard.dirs()),
        backend,
    );
    rig.ed
        .state
        .config
        .languages
        .register_identity("python", &["py"], &[], &[], None)
        .unwrap();
    let file_b = rig.root.join("main.py");
    std::fs::write(&file_b, "x = 1\n").unwrap();
    let bid_b = rig
        .ed
        .open_extra_file(&file_b)
        .expect("main.py opens as a buffer");
    // Both buffers must be *shown*: `lsp/refresh-hints` skips a hidden bid.
    let pid_a = rig.ed.state.focus.id();
    let pane_b = open_pane_in_layout(
        &mut rig.ed.state,
        &mut rig.ed.view,
        pid_a,
        bid_b,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    rig.ed.drain_lsp();
    rig.ed.settle();

    let sid_a = rig.sid("rust-analyzer");
    let sid_b = rig.sid("pylsp");
    assert_eq!(
        rig.attached(),
        vec![sid_a],
        "sanity: main.rs is on rust-analyzer"
    );
    assert_eq!(
        rig.ed
            .state
            .buffer_positions
            .lsp
            .servers(bid_b)
            .collect::<Vec<_>>(),
        vec![sid_b],
        "sanity: main.py is on pylsp"
    );
    TwoServers {
        bid_a: rig.bid,
        requests: rig.requests,
        ed: rig.ed,
        _guard: guard,
        sid_a,
        sid_b,
        bid_b,
        pane_b,
    }
}

fn fire_viewport_change(ed: &mut Editor) {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let pid = ed.state.focus.id();
    ed.queue_viewport_change(pid);
}

/// Two `settle()` rounds after the sleep, not one: `settle()`'s own
/// `drain_async_sources` runs once, at its top, before its fixpoint drains
/// `pending_work`:
/// - **Round 1** picks up the now-due debounce timer (`drain_due_timers`
///   queues the thunk via `queue_steel_call`), and the same call's fixpoint
///   runs it immediately, sending the wire request. The scripted backend
///   auto-queues its response synchronously, but this round's
///   `drain_async_sources` already ran, so it isn't seen yet.
/// - **Round 2**'s `drain_async_sources` is what picks the response up (via
///   `drain_lsp`), and its fixpoint runs the response callback.
///
/// `Editor::run`'s loop does exactly this over two real frames; a test not
/// driving that loop needs the two `settle()` calls explicitly.
fn settle_after_debounce(ed: &mut Editor) {
    ed.settle();
    std::thread::sleep(Duration::from_millis(300));
    ed.settle();
    ed.settle();
}

fn inlay_hint_response(entries: &[(u32, u32, serde_json::Value)]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|(line, character, label)| {
                serde_json::json!({
                    "position": {"line": line, "character": character},
                    "label": label,
                })
            })
            .collect(),
    )
}

#[test]
fn viewport_change_triggers_one_debounced_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 1);
}

/// `lsp/inlay-hint-params` builds its wire `range` straight from
/// `viewport-range`, already 0-based end-exclusive, so no `+ 1` needed.
/// Nothing else in this file inspects the request's `params`, so a stray
/// `+ 1` (re-adding the pre-exclusive-range LSP-end-conversion) would ask
/// for one line past the pane's actual viewport with no test failing.
///
/// With `(hash "line" (+ end 1) ...)` in `inlay.scm`'s `"end"`,
/// `range.end.line` below would be 2.
#[test]
fn inlay_hint_request_range_matches_the_viewport() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    let log = requests.borrow();
    let (_, _, params) = log
        .iter()
        .find(|(_, method, _)| method == "textDocument/inlayHint")
        .expect("one textDocument/inlayHint request must have fired");
    // Fixture buffer ("let x = 1;\n") has exactly one content line, so
    // `viewport-range` clamps to it regardless of the pane's exact height.
    assert_eq!(params["range"]["start"]["line"], 0);
    assert_eq!(params["range"]["end"]["line"], 1);
}

#[test]
fn setting_off_sends_no_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    // lsp_inlay_hints defaults to false: left untouched.

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 0);
}

#[test]
fn hints_land_in_the_store_at_the_correct_char_offset() {
    let tmp = tempfile::tempdir().unwrap();
    // "let x = 1;\n": wire {line:0, character:4} is 'x' (char offset 4,
    // ASCII text, UTF-16 code units == char offsets).
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    let hints: Vec<_> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .collect();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].pos, co(4));
    assert_eq!(hints[0].text, ": i32");
    assert!(hints[0].before);
}

/// The render bridge (`decoration_providers.rs`'s
/// `update_inlay_hint_providers`) does not gate on `lsp.inlay-hints` itself,
/// so the real shipped plugin must make toggling the setting off clear its
/// own hints, via the `on-option-change` hook (`inlay.scm`). Goes
/// through `:set global`, not a direct field write, so the real event fires.
#[test]
fn setting_off_via_set_command_clears_hints_through_the_plugin_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
    });
    let bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":set global lsp.inlay-hints=true");
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "sanity: the hint lands once the setting is on"
    );

    type_cmd(&mut ed, ":set global lsp.inlay-hints=false");
    ed.settle();
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        0,
        "the plugin's on-option-change handler must clear its own \
         (\"lsp-inlay-hints\") source when the setting turns off"
    );
}

/// The `on-option-change` hook (`inlay.scm`) receives the coerced bool, so
/// any of `parse-bool`'s accepted spellings (`on`/`yes`/`1`) must take the
/// branch that requests hints rather than the one that clears them.
///
/// Writes through `settings::ops::apply_global` directly (the production
/// path `:set global`/`set-option!`/`:theme` all funnel through, see its
/// module doc) rather than `type_cmd(":set global …")`: typing and executing
/// a command line opens and closes the minibuffer, which resizes the pane
/// and queues its own `on-viewport-change`. That event independently
/// re-requests hints via its own `get-option` check, which would mask a
/// wrong hook branch. The hook's own branch, and nothing else, must be what
/// re-requests them here. No `fire_viewport_change` for the same reason.
#[test]
fn setting_on_via_a_non_true_spelling_still_requests_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        // Three responses queued: the seeding "true" phase below sends two
        // requests (the direct fire `fire_viewport_change` queues, plus the
        // second, independent one `prepare_frame`'s scroll step arms via
        // `debounce_viewport_change`; see the comment below), and the
        // final "on" toggle sends a third.
        for _ in 0..3 {
            backend.respond_to(
                "textDocument/inlayHint",
                inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
            );
        }
    });
    let bid = ed.focused_buffer_id();

    // Seed a synced viewport and one landed hint. Uses `fire_viewport_change`
    // (unlike the toggles below) purely to establish the viewport
    // `lsp/refresh-hints` needs. That event's own hint request is
    // legitimate here, since the setting is already correctly on.
    crate::editor::settings::ops::apply_global(
        &mut ed.state,
        &mut ed.view,
        "lsp.inlay-hints",
        "true",
    )
    .unwrap();
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    // `fire_viewport_change` -> `prepare_frame` also arms
    // `debounce_viewport_change`'s own Rust-side timer (`lsp.viewport-debounce-ms`,
    // 150ms by default) as a side effect of its scroll step seeing the
    // pane's visible range change for the first time: a *second*,
    // independent `on-viewport-change` fire, on top of the direct one
    // `fire_viewport_change` queues itself. One `settle_after_debounce`
    // round only guarantees the direct fire's request/response round trip
    // completes; without draining this second one here too, it leaks into
    // the toggles below and masks their outcome by re-requesting hints on
    // its own, independent of what `on-option-change`'s branch does.
    settle_after_debounce(&mut ed);
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "sanity: the hint lands once the setting is on"
    );

    crate::editor::settings::ops::apply_global(
        &mut ed.state,
        &mut ed.view,
        "lsp.inlay-hints",
        "off",
    )
    .unwrap();
    ed.settle();
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        0,
        "sanity: \"off\" clears, same as \"false\""
    );

    crate::editor::settings::ops::apply_global(
        &mut ed.state,
        &mut ed.view,
        "lsp.inlay-hints",
        "on",
    )
    .unwrap();
    settle_after_debounce(&mut ed);
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "\"on\" must behave identically to \"true\" and re-request the hint \
         via the hook's own branch, not silently stay cleared"
    );
}

#[test]
fn label_parts_concatenate_and_padding_becomes_literal_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            serde_json::json!([{
                "position": {"line": 0, "character": 4},
                "label": [{"value": ":"}, {"value": " i32"}],
                "paddingLeft": true,
                "paddingRight": true
            }]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    let hints: Vec<_> = ed
        .state
        .config
        .decorations
        .inlay_hints_for_buffer(bid)
        .collect();
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].text, " : i32 ",
        "label parts must concatenate in order, then get padding spaces on both sides"
    );
}

#[test]
fn diagnostics_changed_also_refreshes_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;
    let sid = ed
        .state
        .buffer_positions
        .lsp
        .servers(ed.focused_buffer_id())
        .next()
        .expect("buffer must be attached");

    // Fire a real viewport-change first: the request count assertions
    // below need a known baseline (1 request from this fire, not 0 or 2).
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 1);

    let bid = ed.focused_buffer_id();
    ed.ingest_typed_publish_for_test(
        sid,
        serde_json::from_value(
            serde_json::json!({"uri": rust_rig_uri(tmp.path()), "diagnostics": []}),
        )
        .unwrap(),
    );
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    settle_after_debounce(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/inlayHint"),
        2,
        "on-diagnostics-changed must also trigger a refresh once a viewport is known"
    );
}

#[test]
fn hidden_buffer_skips_diagnostics_triggered_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;
    let sid = ed
        .state
        .buffer_positions
        .lsp
        .servers(ed.focused_buffer_id())
        .next()
        .expect("buffer must be attached");
    let bid = ed.focused_buffer_id();

    // Switch the (only) pane to a second file. `bid` stays open in the
    // buffer list (and stays attached to `sid`) but is no longer shown in
    // any pane, so `(viewport-range bid)` must be `#f`.
    let other_file = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other_file, "let y = 2;\n").unwrap();
    ed.execute_typed("e", Some(other_file.to_str().unwrap()))
        .unwrap();
    assert_ne!(
        ed.focused_buffer_id(),
        bid,
        "test setup: pane must have switched"
    );

    ed.ingest_typed_publish_for_test(
        sid,
        serde_json::from_value(
            serde_json::json!({"uri": rust_rig_uri(tmp.path()), "diagnostics": []}),
        )
        .unwrap(),
    );
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    settle_after_debounce(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/inlayHint"),
        0,
        "a buffer not shown in any pane must skip the refresh, viewport-range being #f"
    );
}

#[test]
fn an_empty_response_clears_previously_stored_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
        // Second refresh (below) gets this canned response: an empty
        // result must still clear the hint the first response stored.
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "first response must land the hint"
    );

    // Viewport is already known; on-diagnostics-changed alone re-triggers
    // the debounced refresh without moving anything.
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    settle_after_debounce(&mut ed);

    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        0,
        "an empty/null inlayHint response must clear stale hints from a previous, larger response"
    );
}

/// Detaching the only server that gives hints clears them: with no server
/// left to ask, the refresh the detach triggers empties the store.
#[test]
fn stopping_the_only_server_clears_its_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "sanity: the hint landed"
    );

    super::super::lsp_rig::stop_server(&mut ed, "rust-analyzer");
    settle_after_debounce(&mut ed);

    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        0
    );
}

/// A buffer no pane shows still loses its hints when its server stops.
#[test]
fn stopping_the_server_clears_the_hints_of_a_hidden_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    let other_file = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other_file, "let y = 2;\n").unwrap();
    ed.execute_typed("e", Some(other_file.to_str().unwrap()))
        .unwrap();
    assert_ne!(ed.focused_buffer_id(), bid, "test setup: the pane switched");
    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1,
        "sanity: the hint survives the switch"
    );

    super::super::lsp_rig::stop_server(&mut ed, "rust-analyzer");
    settle_after_debounce(&mut ed);

    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        0
    );
}

/// A refresh every server fails keeps the hints already shown and reports
/// the error, where an empty answer clears them.
#[test]
fn a_failed_response_keeps_the_stored_hints_and_reports_the_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
        backend.fail_with("textDocument/inlayHint", -32603, "hints exploded");
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    settle_after_debounce(&mut ed);

    assert_eq!(
        ed.state
            .config
            .decorations
            .inlay_hints_for_buffer(bid)
            .count(),
        1
    );
    let log = ed.state.message_log.format_for_display();
    assert!(log.contains("lsp inlay hints: hints exploded"), "{log:?}");
}

/// Two attached, visible
/// buffers each get an `on-diagnostics-changed` fire within the same
/// debounce window. A plain `debounce` shares one pending timer across
/// every call regardless of args, so buffer B's call would cancel buffer A's
/// still-pending call, and only B would ever refresh. `debounce-by` keys
/// per buffer, so both must refresh.
#[test]
fn diagnostics_changed_for_two_buffers_in_the_same_window_both_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let TwoServers {
        mut ed,
        _guard,
        requests,
        sid_a,
        sid_b,
        bid_a,
        bid_b,
        ..
    } = setup_two_servers(
        tmp.path(),
        serde_json::json!({"capabilities": {"inlayHintProvider": true}}),
        serde_json::json!({"capabilities": {"inlayHintProvider": true}}),
        2,
    );
    ed.state.settings.lsp_inlay_hints = true;

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Both fires land inside the same 200ms debounce window, with no settle in
    // between.
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid_a });
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid_b });
    settle_after_debounce(&mut ed);

    let sent_to = |sid: ServerId| {
        requests
            .borrow()
            .iter()
            .any(|(s, m, _)| *s == sid && m == "textDocument/inlayHint")
    };
    assert!(
        sent_to(sid_a),
        "buffer A's refresh must not be cancelled by buffer B's fire in the same window"
    );
    assert!(sent_to(sid_b), "buffer B must also refresh");
}

/// Two buffers, each attached to its own server with different
/// capabilities: buffer A ("rust", no inlayHintProvider) stays focused;
/// buffer B ("python", inlayHintProvider: true) sits in a background pane.
/// `on-viewport-change` fires per-pane, so a viewport event for the
/// background pane must resolve capabilities and the request target
/// against buffer B's own server, never the focused buffer's.
#[test]
fn refresh_hints_resolves_against_the_buffers_own_server_not_the_focused_buffers() {
    let tmp = tempfile::tempdir().unwrap();
    let TwoServers {
        mut ed,
        _guard,
        requests,
        sid_b,
        pane_b,
        ..
    } = setup_two_servers(
        tmp.path(),
        serde_json::json!({"capabilities": {}}),
        serde_json::json!({"capabilities": {"inlayHintProvider": true}}),
        1,
    );
    ed.state.settings.lsp_inlay_hints = true;

    // Buffer A (focused, server A, no inlayHintProvider) never changes
    // focus. The viewport event below is for the background pane only.
    ed.queue_viewport_change(pane_b);
    settle_after_debounce(&mut ed);

    let sent_to_b = requests
        .borrow()
        .iter()
        .any(|(sid, m, _)| *sid == sid_b && m == "textDocument/inlayHint");
    assert!(
        sent_to_b,
        "a viewport event for buffer B's pane must query buffer B's own server's \
         capabilities and send the request there, not the focused buffer's server"
    );
}

/// Reproduces the "hint doesn't come back after undo" bug: a hint dropped by
/// `remap_points`'s deletion-anchor fix (`hume-decorations`'s `decorations.rs`) must be
/// re-requested once the deleting edit is undone. `on-text-changed` fires
/// for undo exactly like any other edit (`event.rs`'s doc comment), so
/// `inlay.scm` hooking it must pick this up without any viewport scroll or
/// diagnostics republish.
#[test]
fn undo_also_refreshes_hints() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;

    // Insert a character and settle. Its own on-text-changed fire is the
    // baseline (1). No `fire_viewport_change`/`prepare_frame` call in the
    // mix: that helper arms Rust's own viewport-debounce timer as a side
    // effect (`frame.rs`'s `debounce_viewport_change`), which would cascade
    // into a second, unrelated `on-viewport-change` fire during the next
    // `settle_after_debounce`'s sleep: noise this test doesn't want.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    settle_after_debounce(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 1);

    ed.feed_key(key('u')); // undo
    settle_after_debounce(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/inlayHint"),
        2,
        "undo must also trigger a refresh via on-text-changed: it bumps \
         generation exactly like the insert above did"
    );
}
