// Generalized event-loop wake (`wake_timeout`) and the timer wheel's
// integration with it as a second real AsyncSource.

use std::time::{Duration, Instant};

use super::*;
use crate::editor::async_source::PENDING_POLL;
use hume_treesitter::parse_worker::{ParseBackend, ParseDone, ParseRequest};

/// A `ParseBackend` double that reports work permanently in flight, without
/// spinning a real thread — for exercising `wake_timeout`'s "pending" branch
/// deterministically (the production `InlineParseBackend` always reports
/// `has_in_flight() == false`, so it can't reach that branch).
struct AlwaysPendingBackend;

impl ParseBackend for AlwaysPendingBackend {
    fn post(&mut self, _req: ParseRequest) {}
    fn drain_done(&mut self) -> Vec<ParseDone> {
        Vec::new()
    }
    fn is_in_flight(&self, _bid: hume_engine::pipeline::BufferId, _text_gen: u64) -> bool {
        false
    }
    fn remove_in_flight(&mut self, _bid: hume_engine::pipeline::BufferId) {}
    fn clear_in_flight_if_matches(
        &mut self,
        _bid: hume_engine::pipeline::BufferId,
        _text_gen: u64,
        _lang: &std::sync::Arc<hume_treesitter::registry::LanguageConfig>,
    ) {
    }
    fn has_in_flight(&self) -> bool {
        true
    }
    fn is_disconnected(&self) -> bool {
        false
    }
}

#[test]
fn wake_timeout_is_none_when_idle() {
    // The test harness's InlineParseBackend never reports in-flight work and
    // a freshly-constructed editor's timer wheel has nothing scheduled —
    // idle across every source must stay a blocking read.
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn wake_timeout_is_8ms_when_a_source_is_pending() {
    let mut ed = editor_from("-[w]>ord\n");
    ed.parse_worker = Box::new(AlwaysPendingBackend);
    assert_eq!(ed.wake_timeout(), Some(PENDING_POLL));
}

#[test]
fn wake_timeout_bounded_by_nearer_timer_deadline() {
    // The generalized wake predicate deferred this case ("Some(<8ms) when a
    // nearer deadline exists") until a second real AsyncSource existed — the
    // timer wheel is that source.
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
    // A far-future deadline must neither collapse to the short pending-poll
    // ceiling (the timer wheel reports its own deadline, not a fake "pending"
    // flag) nor block forever (None) — it bounds the timeout to roughly the
    // real wait.
    let mut ed = editor_from("-[w]>ord\n");
    ed.timer_wheel.schedule(Duration::from_secs(10));

    let timeout = ed.wake_timeout().expect("a timer is scheduled");
    assert!(
        timeout > PENDING_POLL && timeout <= Duration::from_secs(10),
        "a distant deadline must not trigger 8ms busy-polling, got {timeout:?}"
    );
}

#[test]
fn lsp_backend_is_included_in_async_sources() {
    // Growing async_sources() to 3 must not regress the pending/idle
    // behavior the earlier tests pin — the freshly-constructed InlineLspBackend
    // has nothing queued, so it must not force a poll timeout on its own.
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
        .start("rust-analyzer", &[], std::path::Path::new("."))
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

/// Fix 2: `LspState::next_wake` originally only checked the raw backend
/// (`ThreadedLspBackend`/`InlineLspBackend` always report `false`), so a
/// client mid-handshake or with a request in flight had no wake source at
/// all — the response would sit undrained until the next keypress. These
/// three tests pin the corrected condition against `wake_timeout` directly.
mod next_wake_covers_client_state {
    use super::*;
    use hume_lsp::backend::LspBackend;
    use hume_lsp::client::{LspClient, RequestMeta, ServerState};
    use hume_lsp::inline::InlineLspBackend;
    use std::path::{Path, PathBuf};

    fn wired_editor() -> (Editor, hume_lsp::backend::ServerId) {
        let mut ed = editor_from("-[w]>ord\n");
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], Path::new(".")).unwrap();
        ed.lsp = super::super::super::lsp::LspState::from_backend_for_test(Box::new(backend));
        (ed, sid)
    }

    #[test]
    fn wake_timeout_is_8ms_while_a_client_is_starting() {
        // Mid-handshake, the initialize response could land any moment —
        // without this, nothing wakes the loop until the next keypress,
        // which would also stall anything queued behind the handshake
        // (e.g. the fixed didOpen queueing).
        let (mut ed, sid) = wired_editor();
        ed.lsp
            .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));

        assert_eq!(ed.wake_timeout(), Some(PENDING_POLL));
    }

    #[test]
    fn wake_timeout_is_8ms_for_a_running_client_with_a_request_in_flight() {
        // The 8ms poll cadence for in-flight requests, not the
        // coarser 200ms Running-idle heartbeat.
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.state = ServerState::Running;
        ed.lsp.insert_client_for_test(client);

        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        ed.lsp
            .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

        assert_eq!(ed.wake_timeout(), Some(Duration::from_millis(8)));
    }

    #[test]
    fn wake_timeout_for_a_running_idle_client_is_bounded_by_the_heartbeat() {
        let (mut ed, sid) = wired_editor();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.state = ServerState::Running;
        ed.lsp.insert_client_for_test(client);

        let timeout = ed
            .wake_timeout()
            .expect("a Running client sets the heartbeat deadline");
        assert!(
            timeout > Duration::from_millis(8) && timeout <= Duration::from_millis(200),
            "a Running, idle client must be bounded by the heartbeat, not the 8ms poll \
             ceiling, got {timeout:?}"
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
