// Generalized event-loop wake (`wake_timeout`) and the timer wheel's
// integration with it as a real AsyncSource. Response/completion arrival is
// not a poll cadence this module tracks — background threads wake the
// event loop's wait primitive directly via `termina::PlatformWaker`, so
// `wake_timeout` reports real deadlines only: the timer wheel's own
// schedule, and the LSP source's earliest pending-request deadline / spinner
// cadence (see the `next_wake_covers_client_state` module below).

use std::time::{Duration, Instant};

use super::*;
use hume_treesitter::parse_worker::{ParseBackend, ParseDone, ParseRequest};

/// A `ParseBackend` double that never completes a request, without spinning
/// a real thread. The parse worker does not contribute an `AsyncSource` (see
/// `in_flight_parse_no_longer_forces_a_wake` below) — `ParseBackend` has no
/// in-flight query at all (that state lives on `Syntax` now), so this double
/// exists purely to prove `wake_timeout` stays unaffected by a backend that
/// never drains anything, arrival being wake-driven instead.
struct AlwaysPendingBackend;

impl ParseBackend for AlwaysPendingBackend {
    fn post(&mut self, _req: ParseRequest) {}
    fn drain_done(&mut self) -> Vec<ParseDone> {
        Vec::new()
    }
    fn is_disconnected(&self) -> bool {
        false
    }
}

#[test]
fn wake_timeout_is_none_when_idle() {
    // The test harness's InlineParseBackend never reports in-flight work and
    // a freshly-constructed editor's timer wheel has nothing scheduled —
    // idle across every source must stay a blocking wait.
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn in_flight_parse_no_longer_forces_a_wake() {
    // The parse worker must not contribute an `AsyncSource` regardless of
    // backend state — parse completion wakes the loop through the platform
    // waker, not a deadline — so this must stay `None` even against a
    // backend that never drains anything.
    let mut ed = editor_from("-[w]>ord\n");
    ed.parse_worker = Box::new(AlwaysPendingBackend);
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn wake_timeout_bounded_by_nearer_timer_deadline() {
    // The generalized wake predicate deferred this case ("Some(<timeout>)
    // when a nearer deadline exists") until a second real AsyncSource
    // existed — the timer wheel is that source.
    let mut ed = editor_from("-[w]>ord\n");
    ed.timer_wheel.schedule(Duration::from_millis(2));

    let timeout = ed.wake_timeout().expect("a timer is scheduled");
    assert!(
        timeout <= Duration::from_millis(2),
        "wake_timeout must be bounded by the nearer 2ms timer deadline, got {timeout:?}"
    );
}

#[test]
fn wake_timeout_distant_timer_bounds_without_busy_polling() {
    // A far-future deadline must be honored almost exactly, not collapsed to
    // some artificially short cadence — there is no "pending" poll source
    // left to do that collapsing (arrival is wake-driven now), but this
    // still pins that a distant timer isn't somehow shortened.
    let mut ed = editor_from("-[w]>ord\n");
    ed.timer_wheel.schedule(Duration::from_secs(10));

    let timeout = ed.wake_timeout().expect("a timer is scheduled");
    assert!(
        timeout > Duration::from_secs(5) && timeout <= Duration::from_secs(10),
        "a distant deadline must not be shortened, got {timeout:?}"
    );
}

#[test]
fn lsp_backend_is_included_in_async_sources() {
    // The freshly-constructed InlineLspBackend has nothing queued and no
    // servers registered, so it must not force a wake on its own.
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn scripted_initialize_round_trip_through_editor() {
    // A scripted initialize round-trip passes end-to-end
    // through the editor's LspState, via the same LspBackend trait object
    // the production ThreadedLspBackend implements.
    use hume_lsp::codec::{Message, RequestId};
    use hume_lsp::inline::InlineLspBackend;
    use hume_lsp::transport::InboundEvent;

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = super::super::lsp::LspState::from_backend_for_test(Box::new(
        InlineLspBackend::with_default_handshake(),
    ));

    let backend = ed.lsp.backend_mut();
    let server = backend
        .start("rust-analyzer", &[], std::path::Path::new("."), &[])
        .expect("inline start never fails");
    backend.send(
        server,
        Message::Request {
            id: RequestId::Int(1),
            method: "initialize".to_string(),
            params: Default::default(),
        },
    );

    let events = backend.drain();
    assert_eq!(events.len(), 1);
    match &events[0] {
        (sid, InboundEvent::Message(Message::Response { id, result })) => {
            assert_eq!(*sid, server);
            assert_eq!(*id, RequestId::Int(1));
            assert!(result.is_ok(), "expected the canned initialize success");
        }
        other => panic!("expected a Response event, got {other:?}"),
    }
}

/// `LspState::next_wake` now reports real deadlines only: the earliest
/// pending-request deadline across every server, and the spinner cadence
/// while a client is `Starting` or reporting `$/progress`. These tests pin
/// that rewrite directly against `wake_timeout`.
mod next_wake_covers_client_state {
    use super::*;
    use hume_lsp::backend::LspBackend;
    use hume_lsp::client::{ClientAction, LspClient, RequestMeta, ServerState};
    use hume_lsp::inline::InlineLspBackend;
    use std::path::{Path, PathBuf};

    /// The spinner's own cadence (`SPINNER_INTERVAL` in `lsp/mod.rs`, which
    /// is private to that module) — duplicated here as a literal rather than
    /// widening that constant's visibility just for test comparisons.
    const SPINNER_INTERVAL: Duration = Duration::from_millis(100);

    fn wired_editor() -> (Editor, hume_lsp::backend::ServerId) {
        let mut ed = editor_from("-[w]>ord\n");
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
        ed.lsp = super::super::super::lsp::LspState::from_backend_for_test(Box::new(backend));
        (ed, sid)
    }

    #[test]
    fn starting_client_wakes_at_spinner_cadence() {
        // Mid-handshake, the initialize response could land any moment, and
        // the statusline spinner must keep animating while it waits, so
        // `Starting` must be folded into the spinner-cadence condition
        // directly rather than relying on a separate pending-poll arm.
        let (mut ed, sid) = wired_editor();
        ed.lsp
            .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));

        let timeout = ed
            .wake_timeout()
            .expect("a Starting client must force a wake");
        assert!(
            timeout <= SPINNER_INTERVAL,
            "a Starting client must wake at the spinner cadence, got {timeout:?}"
        );
    }

    #[test]
    fn starting_client_with_pending_initialize_keeps_spinner_cadence() {
        // The in-flight `initialize` request itself carries a 30s deadline —
        // far longer than the spinner cadence. The spinner arm must still
        // win so the handshake animation doesn't freeze for 30 seconds.
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        let mut backend = InlineLspBackend::new();
        client.start_handshake(&mut backend);
        ed.lsp.insert_client_for_test(client);

        let timeout = ed
            .wake_timeout()
            .expect("a Starting client must force a wake");
        assert!(
            timeout <= SPINNER_INTERVAL,
            "the spinner cadence must win over the 30s initialize deadline, got {timeout:?}"
        );
    }

    #[test]
    fn running_request_deadline_bounds_wake() {
        // A Running client with an ordinary request in flight: `next_wake`
        // must report that request's own deadline (not a poll cadence, and
        // not the (now-deleted) coarser heartbeat).
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.set_state_for_test(ServerState::Running);
        ed.lsp.insert_client_for_test(client);

        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        ed.lsp
            .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

        let timeout = ed
            .wake_timeout()
            .expect("a pending request sets its own deadline");
        assert!(
            timeout > SPINNER_INTERVAL && timeout <= Duration::from_secs(10),
            "must be bounded by the request's own ~10s deadline, not the spinner cadence, \
             got {timeout:?}"
        );
    }

    #[test]
    fn earliest_deadline_wins_across_multiple_servers() {
        // `next_wake` aggregates `earliest_deadline()` across every server
        // with `.min()` — a far deadline on one server must never hide a
        // near one on another.
        let mut ed = editor_from("-[w]>ord\n");
        let mut backend = InlineLspBackend::new();
        let sid_near = backend.start("x", &[], Path::new("."), &[]).unwrap();
        let sid_far = backend.start("y", &[], Path::new("."), &[]).unwrap();
        ed.lsp = super::super::super::lsp::LspState::from_backend_for_test(Box::new(backend));

        let mut client_near = LspClient::new(sid_near, PathBuf::from("."));
        client_near.set_state_for_test(ServerState::Running);
        ed.lsp.insert_client_for_test(client_near);
        let mut client_far = LspClient::new(sid_far, PathBuf::from("."));
        client_far.set_state_for_test(ServerState::Running);
        ed.lsp.insert_client_for_test(client_far);

        ed.lsp.send_request(
            sid_far,
            "textDocument/hover",
            serde_json::Value::Null,
            RequestMeta {
                method: "textDocument/hover".to_string(),
                allow_stale: false,
                deadline: Instant::now() + Duration::from_secs(20),
            },
        );
        ed.lsp.send_request(
            sid_near,
            "textDocument/hover",
            serde_json::Value::Null,
            RequestMeta {
                method: "textDocument/hover".to_string(),
                allow_stale: false,
                deadline: Instant::now() + Duration::from_secs(2),
            },
        );

        let timeout = ed.wake_timeout().expect("two pending requests");
        assert!(
            timeout <= Duration::from_secs(2),
            "must be bounded by the nearer (2s) server's deadline, not the farther (20s) one, \
             got {timeout:?}"
        );
    }

    #[test]
    fn running_idle_client_blocks_fully() {
        // Pins the heartbeat's deletion: a Running client with nothing
        // pending and no progress must not force any wake at all — arrival
        // is wake-driven now, so idle-Running is genuinely idle.
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.set_state_for_test(ServerState::Running);
        ed.lsp.insert_client_for_test(client);

        assert_eq!(ed.wake_timeout(), None);
    }

    #[test]
    fn progress_task_wakes_at_spinner_cadence() {
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.set_state_for_test(ServerState::Running);
        ed.lsp.insert_client_for_test(client);

        let action = ClientAction::Progress(
            serde_json::from_value(serde_json::json!({
                "token": "t1",
                "value": {"kind": "begin", "title": "Indexing"},
            }))
            .unwrap(),
        );
        ed.dispatch_lsp_action(sid, action);

        let timeout = ed
            .wake_timeout()
            .expect("an active progress task must force a wake");
        assert!(
            timeout <= SPINNER_INTERVAL,
            "a progress task must wake at the spinner cadence, got {timeout:?}"
        );
    }
}

#[test]
fn timer_wheel_end_to_end_tick_via_editor() {
    // Sleep-free: jump the query point 20ms past scheduling instead of
    // sleeping in the test.
    let mut ed = editor_from("-[w]>ord\n");
    let id = ed.timer_wheel.schedule(Duration::from_millis(10));

    let due = ed
        .timer_wheel
        .take_due(Instant::now() + Duration::from_millis(20));
    assert_eq!(due, vec![id]);
}
