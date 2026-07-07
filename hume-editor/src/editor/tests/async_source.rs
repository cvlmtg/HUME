// P3 (docs/lsp/step-0.md) — generalized event-loop wake (`wake_timeout`).

use std::time::Duration;

use super::*;
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
    // P3 introduces no other source yet — idle must stay a blocking read.
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn wake_timeout_is_8ms_when_a_source_is_pending() {
    let mut ed = editor_from("-[w]>ord\n");
    ed.parse_worker = Box::new(AlwaysPendingBackend);
    assert_eq!(ed.wake_timeout(), Some(Duration::from_millis(8)));
}
