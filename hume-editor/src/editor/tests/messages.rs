// `:messages` severity highlighting — the `format_with_spans` → extra-highlights
// store → `ScopedHighlighter` Extra tier wiring added alongside the message
// log's existing `[severity]` prefix formatting.
//
// Like `lsp_render.rs`, uses `Editor::open` (not `editor_from`) so `build_pane`
// has registered the pane's `PaneHighlights` providers.

use super::*;
use hume_engine::pipeline::{PaneId, RenderContext};
use hume_engine::types::ScopeId;

fn scope(ed: &Editor, name: &str) -> ScopeId {
    ed.view
        .registry
        .get(name)
        .unwrap_or_else(|| panic!("scope '{name}' must already be interned"))
}

fn extra_arc(ed: &Editor, pid: PaneId) -> Vec<(usize, usize, usize, ScopeId)> {
    ed.state.panes.render[pid]
        .highlights
        .extra
        .read()
        .unwrap()
        .clone()
}

#[test]
fn messages_populates_the_extra_highlights_store() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());
    ed.state
        .message_log
        .push(Severity::Error, "crash".to_string());

    ed.execute_typed("messages", None).unwrap();
    let bid = ed.focused_buffer_id();

    // Independent oracle, same as message_log's own offset test: "[warning]"
    // is 9 chars (0..9), "bad key" is 7 (10..17); "[error]" is 7 chars
    // (18..25), "crash" is 5 (26..31).
    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(spans.len(), 4);
    assert_eq!(
        (spans[0].start, spans[0].end, &spans[0].scope[..]),
        (0, 9, "diagnostic.warning.message")
    );
    assert_eq!(
        (spans[1].start, spans[1].end, &spans[1].scope[..]),
        (10, 17, "diagnostic.warning.message-text")
    );
    assert_eq!(
        (spans[2].start, spans[2].end, &spans[2].scope[..]),
        (18, 25, "diagnostic.error.message")
    );
    assert_eq!(
        (spans[3].start, spans[3].end, &spans[3].scope[..]),
        (26, 31, "diagnostic.error.message-text")
    );
}

#[test]
fn messages_spans_reach_the_pane_extra_highlight_arc() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());
    ed.state
        .message_log
        .push(Severity::Error, "crash".to_string());

    ed.execute_typed("messages", None).unwrap();
    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let warn_badge = scope(&ed, "diagnostic.warning.message");
    let warn_text = scope(&ed, "diagnostic.warning.message-text");
    let err_badge = scope(&ed, "diagnostic.error.message");
    let err_text = scope(&ed, "diagnostic.error.message-text");

    // Line-relative byte offsets, matching `lsp_render.rs`'s `extra_arc`
    // convention: line 0 is "[warning] bad key", line 1 is "[error] crash".
    assert_eq!(
        extra_arc(&ed, pid),
        vec![
            (0, 0, 9, warn_badge),
            (0, 10, 17, warn_text),
            (1, 0, 7, err_badge),
            (1, 8, 13, err_text),
        ]
    );
}

#[test]
fn repeat_messages_replaces_stale_spans_not_appends() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state
        .message_log
        .push(Severity::Warning, "first".to_string());
    ed.execute_typed("messages", None).unwrap();
    let bid = ed.focused_buffer_id();

    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(spans.len(), 2, "one entry -> one badge + one body span");
    assert_eq!(
        spans[1].end, 15,
        "\"[warning] first\" -> body ends at char 15"
    );

    // set_view_content (the repeat-call path) replaces the rope without a
    // ChangeSet, so remap_through can't carry these spans forward — the
    // command handler must recompute and overwrite them wholesale.
    ed.state
        .message_log
        .push(Severity::Error, "second".to_string());
    ed.execute_typed("messages", None).unwrap();
    let bid_again = ed.focused_buffer_id();
    assert_eq!(
        bid_again, bid,
        "the [messages] buffer is reused, not duplicated"
    );

    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(spans.len(), 4, "two entries -> four spans, not six");
    // "[warning] first\n[error] second\n": last body span ends at char 30
    // ("[warning]"=9 + " "=1 + "first"=5 + "\n"=1 + "[error]"=7 + " "=1 + "second"=6).
    assert_eq!(spans[3].end, 30);
}
