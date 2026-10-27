/// The message-log summary auto-dismisses after exactly 3 keystrokes of visibility
/// (`SUMMARY_TTL = 3`).
///
/// Timeline:
///   - report() → status_msg set, summary hidden behind it
///   - key 1 → status_msg cleared, summary appears, TTL armed (3)
///   - key 2 → TTL ticks 3→2, summary still visible
///   - key 3 → TTL ticks 2→1, summary still visible
///   - key 4 → TTL ticks 1→0 → mark_all_seen() fires, summary gone
#[test]
fn message_log_summary_ttl() {
    use crate::editor::Editor;
    use hume_core::Severity;
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let noop = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);

    let (text, sels) = test_fixtures::testing::parse_state("-[a]>\n");
    let mut ed = Editor::for_testing_with(test_fixtures::testing::state(text, sels));

    // report() sets status_msg AND logs to message_log.
    ed.report(Severity::Error, "boom".to_string());
    assert!(ed.state.status_msg.is_some());
    assert!(ed.state.message_log.has_unseen());

    // Key 1: status_msg clears, TTL armed, summary visible.
    ed.handle_key(noop);
    assert!(ed.state.status_msg.is_none());
    assert!(
        ed.state.message_log.has_unseen(),
        "summary should still be visible after key 1"
    );

    // Key 2: TTL ticks 3→2, summary still visible.
    ed.handle_key(noop);
    assert!(
        ed.state.message_log.has_unseen(),
        "summary should still be visible after key 2"
    );

    // Key 3: TTL ticks 2→1, summary still visible.
    ed.handle_key(noop);
    assert!(
        ed.state.message_log.has_unseen(),
        "summary should still be visible after key 3"
    );

    // Key 4: TTL ticks 1→0 → auto-dismissed.
    ed.handle_key(noop);
    assert!(
        !ed.state.message_log.has_unseen(),
        "summary should be gone after key 4"
    );
}
