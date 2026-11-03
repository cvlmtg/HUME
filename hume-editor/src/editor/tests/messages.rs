// `:messages` severity highlighting: the `format_with_spans` → extra-highlights
// store → `ScopedHighlighter` Extra tier wiring added alongside the message
// log's existing `[severity]` prefix formatting. Also the
// `messages-min-severity` floor on what `:messages` shows, and the open view
// following the log.
//
// The highlighting tests use `Editor::open` (not `editor_from`), like
// `lsp_render.rs`, so `build_pane` has registered the pane's `PaneHighlights`
// providers.

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_engine::providers::HighlightTier;
use hume_grid::Rect;

/// Regression test for the reported crash: `:messages` interns its severity
/// scope names synchronously during command dispatch, then the very next
/// frame both writes their spans AND renders them. `render_to_buf`'s internal
/// `prepare_frame` is the ONLY frame here (no warm-up frame), which is
/// exactly the shape that panicked in `hume-engine/src/theme/mod.rs`'s
/// `Theme::resolve` before `prepare_frame` gained its end-of-frame
/// `bake_if_stale`.
#[test]
fn messages_renders_its_badge_color_on_the_first_frame() {
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());
    ed.execute_typed("messages", None).unwrap();

    let area = Rect::new(0, 0, 40, 3);
    let buf = ed.render_to_buf(area);

    let badge_scope = ed
        .view
        .registry
        .get("diagnostic.warning.message")
        .expect("`:messages` must have interned its badge scope");
    let resolved = ed.view.theme.resolve(badge_scope);
    assert!(
        resolved.bg.is_some(),
        "sanity: the badge scope must resolve to a real background color"
    );

    let bg_colors: Vec<_> = (area.left()..area.right())
        .flat_map(|x| (area.top()..area.bottom()).map(move |y| (x, y)))
        .map(|(x, y)| buf[(x, y)].style().bg)
        .collect();
    assert!(
        bg_colors.contains(&resolved.bg),
        "the [warning] badge's real background must appear on the frame :messages first interns it"
    );
}

#[test]
fn messages_populates_the_extra_highlights_store() {
    let mut ed = open_headless(None);
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());
    ed.state
        .message_log
        .push(Severity::Error, "crash".to_string());

    ed.execute_typed("messages", None).unwrap();
    let bid = ed.focused_buffer_id();

    // Offsets counted by hand, as in message_log's own offset test: "[warning]"
    // is 9 chars (0..9), "bad key" is 7 (10..17); "[error]" is 7 chars
    // (18..25), "crash" is 5 (26..31).
    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(spans.len(), 4);
    assert_eq!(
        (spans[0].start, spans[0].end, spans[0].scope),
        (co(0), co(9), scope(&ed, "diagnostic.warning.message"))
    );
    assert_eq!(
        (spans[1].start, spans[1].end, spans[1].scope),
        (
            co(10),
            co(17),
            scope(&ed, "diagnostic.warning.message-text")
        )
    );
    assert_eq!(
        (spans[2].start, spans[2].end, spans[2].scope),
        (co(18), co(25), scope(&ed, "diagnostic.error.message"))
    );
    assert_eq!(
        (spans[3].start, spans[3].end, spans[3].scope),
        (co(26), co(31), scope(&ed, "diagnostic.error.message-text"))
    );
}

#[test]
fn messages_spans_reach_the_pane_extra_highlight_arc() {
    let mut ed = open_headless(None);
    ed.state
        .message_log
        .push(Severity::Warning, "bad key".to_string());
    ed.state
        .message_log
        .push(Severity::Error, "crash".to_string());

    ed.execute_typed("messages", None).unwrap();
    let pid = ed.view.focused_pane();
    let mut ctx = RenderContext::new();
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let warn_badge = scope(&ed, "diagnostic.warning.message");
    let warn_text = scope(&ed, "diagnostic.warning.message-text");
    let err_badge = scope(&ed, "diagnostic.error.message");
    let err_text = scope(&ed, "diagnostic.error.message-text");

    // Line-relative byte offsets: line 0 is "[warning] bad key", line 1 is
    // "[error] crash".
    assert_eq!(
        pane_highlights(&ed, pid, HighlightTier::Extra),
        vec![
            (
                hume_rope::line::ContentLine::new(0),
                bc(0),
                bc(9),
                warn_badge
            ),
            (
                hume_rope::line::ContentLine::new(0),
                bc(10),
                bc(17),
                warn_text
            ),
            (
                hume_rope::line::ContentLine::new(1),
                bc(0),
                bc(7),
                err_badge
            ),
            (
                hume_rope::line::ContentLine::new(1),
                bc(8),
                bc(13),
                err_text
            ),
        ]
    );
}

#[test]
fn repeat_messages_replaces_stale_spans_not_appends() {
    let mut ed = open_headless(None);
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
        spans[1].end,
        co(15),
        "\"[warning] first\" -> body ends at char 15"
    );

    // A repeat call carries the first call's spans through its edit, then
    // sets every span again: the new set replaces the carried one, never adds
    // to it.
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
    assert_eq!(spans[3].end, co(30));
}

fn shown_messages(ed: &Editor) -> String {
    ed.state
        .message_log
        .format_with_spans(ed.state.settings.messages_min_severity)
        .0
}

#[test]
fn messages_min_severity_hides_reports_below_it() {
    let mut ed = editor_from("-[a]>\n");
    run_set(&mut ed, "global messages-min-severity=warning").expect("the floor is a valid setting");
    ed.report(Severity::Trace, "trace detail");
    ed.report(Severity::Warning, "a warning");
    let shown = shown_messages(&ed);
    assert!(shown.contains("a warning"));
    assert!(!shown.contains("trace detail"));
}

#[test]
fn lowering_the_floor_shows_trace_reported_before_it() {
    let mut ed = editor_from("-[a]>\n");
    ed.report(Severity::Trace, "trace detail");
    assert!(!shown_messages(&ed).contains("trace detail"));
    run_set(&mut ed, "global messages-min-severity=trace").expect("trace is a valid floor");
    assert!(shown_messages(&ed).contains("trace detail"));
}

// ── The open view follows the log ────────────────────────────────────────────

/// Open `:messages` on the focused pane and return the view's buffer.
fn open_messages(ed: &mut Editor) -> BufferId {
    ed.execute_typed("messages", None)
        .expect(":messages opens the view");
    ed.focused_buffer_id()
}

fn buffer_text(ed: &Editor, bid: BufferId) -> String {
    ed.state.buffers.get(bid).text().to_string()
}

fn head_line(ed: &Editor, pane: PaneId, bid: BufferId) -> usize {
    let sels = ed
        .selections_for(pane, bid)
        .expect("the pane has shown the buffer");
    EditView::bind(ed.state.buffers.get(bid).text(), sels)
        .primary()
        .head_line()
        .index()
}

fn summary(ed: &Editor) -> Option<String> {
    ed.state
        .message_log
        .summary_text(ed.state.settings.messages_min_severity)
}

#[test]
fn a_message_logged_while_the_view_is_open_appears_on_the_next_frame() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);

    ed.report(Severity::Error, "second");
    frame(&mut ed, 60, 12);

    insta::assert_snapshot!(buffer_text(&ed, bid), @"
    [warning] first
    [error] second
    ");
    let spans = ed
        .state
        .config
        .decorations
        .extra_highlights_for("messages", bid);
    assert_eq!(spans.len(), 4, "two entries, a badge and a body span each");
    // "[warning] first\n" is 16 chars, "[error] second" ends 14 later.
    assert_eq!(spans[3].end, co(30));
}

#[test]
fn the_open_view_refilters_when_the_floor_changes() {
    let mut ed = open_headless(None);
    ed.report(Severity::Trace, "detail");
    ed.report(Severity::Warning, "careful");
    let bid = open_messages(&mut ed);
    insta::assert_snapshot!(buffer_text(&ed, bid), @"[warning] careful");

    run_set(&mut ed, "global messages-min-severity=trace").expect("trace is a valid floor");
    frame(&mut ed, 60, 12);
    insta::assert_snapshot!(buffer_text(&ed, bid), @"
    [trace] detail
    [warning] careful
    ");

    run_set(&mut ed, "global messages-min-severity=error").expect("error is a valid floor");
    frame(&mut ed, 60, 12);
    insta::assert_snapshot!(format!("{:?}", buffer_text(&ed, bid)), @r#""\n""#);
    assert!(
        ed.state
            .config
            .decorations
            .extra_highlights_for("messages", bid)
            .is_empty()
    );
}

#[test]
fn a_cursor_on_the_last_entry_stays_when_a_new_one_arrives() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "a");
    ed.report(Severity::Warning, "b");
    let bid = open_messages(&mut ed);
    let pane = ed.view.focused_pane();
    assert_eq!(
        head_line(&ed, pane, bid),
        1,
        "sanity: :messages parks on the last entry"
    );

    ed.report(Severity::Warning, "c");
    frame(&mut ed, 60, 12);

    insta::assert_snapshot!(buffer_text(&ed, bid), @"
    [warning] a
    [warning] b
    [warning] c
    ");
    assert_eq!(head_line(&ed, pane, bid), 1);
}

#[test]
fn every_pane_keeps_its_entry_when_earlier_ones_leave_the_view() {
    let mut ed = open_headless(None);
    run_set(&mut ed, "global messages-min-severity=trace").expect("trace is a valid floor");
    for (severity, text) in [
        (Severity::Trace, "t0"),
        (Severity::Warning, "w0"),
        (Severity::Trace, "t1"),
        (Severity::Warning, "w1"),
        (Severity::Warning, "w2"),
    ] {
        ed.report(severity, text);
    }
    let bid = open_messages(&mut ed);
    let left = ed.view.focused_pane();
    let right = vsplit_focused(&mut ed);
    seek_to_line(&mut ed, 4);
    ed.switch_focused_pane(left);
    seek_to_line(&mut ed, 1);

    run_set(&mut ed, "global messages-min-severity=warning").expect("warning is a valid floor");
    frame(&mut ed, 60, 12);

    insta::assert_snapshot!(buffer_text(&ed, bid), @"
    [warning] w0
    [warning] w1
    [warning] w2
    ");
    assert_eq!(head_line(&ed, left, bid), 0);
    assert_eq!(head_line(&ed, right, bid), 2);
}

#[test]
fn following_the_log_leaves_the_view_unmodified() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);

    ed.report(Severity::Warning, "second");
    frame(&mut ed, 60, 12);

    insta::assert_snapshot!(buffer_text(&ed, bid), @"
    [warning] first
    [warning] second
    ");
    assert!(!ed.state.has_unsaved_changes(bid));
}

#[test]
fn a_view_on_the_active_tab_marks_new_messages_seen() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    open_messages(&mut ed);

    ed.report(Severity::Warning, "second");
    frame(&mut ed, 60, 12);

    assert_eq!(summary(&ed), None);
}

#[test]
fn a_view_open_but_not_shown_leaves_new_messages_unseen() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);
    frame(&mut ed, 60, 12);
    ed.execute_typed("bp", None)
        .expect("a previous buffer exists");
    assert_ne!(ed.focused_buffer_id(), bid, "sanity: the view is hidden");

    ed.report(Severity::Warning, "second");
    frame(&mut ed, 60, 12);

    assert_eq!(summary(&ed).as_deref(), Some("1 warning (see :messages)"));
}

#[test]
fn a_view_on_another_tab_leaves_new_messages_unseen() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    ed.execute_typed("tabnew", None).expect("a tab opens");
    open_messages(&mut ed);
    frame(&mut ed, 60, 12);
    ed.execute_typed("tabprev", None)
        .expect("the first tab exists");

    ed.report(Severity::Warning, "second");
    frame(&mut ed, 60, 12);

    assert_eq!(summary(&ed).as_deref(), Some("1 warning (see :messages)"));
}

#[test]
fn repeating_messages_keeps_the_other_panes_cursor() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "a");
    ed.report(Severity::Warning, "b");
    let bid = open_messages(&mut ed);
    let left = ed.view.focused_pane();
    let right = vsplit_focused(&mut ed);
    seek_to_line(&mut ed, 1);
    ed.switch_focused_pane(left);

    ed.report(Severity::Warning, "c");
    open_messages(&mut ed);

    assert_eq!(head_line(&ed, right, bid), 1);
    assert_eq!(
        head_line(&ed, left, bid),
        2,
        "the pane running :messages parks on the last entry"
    );
}

#[test]
fn a_config_reload_keeps_the_open_views_highlights() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);

    ed.reset_config_state();
    frame(&mut ed, 60, 12);

    assert_eq!(
        ed.state
            .config
            .decorations
            .extra_highlights_for("messages", bid)
            .len(),
        2
    );
}

#[test]
fn a_message_below_the_shown_rows_stays_unseen_until_the_end_is_drawn() {
    let mut ed = open_headless(None);
    for i in 0..30 {
        ed.report(Severity::Warning, format!("w{i}"));
    }
    open_messages(&mut ed);
    frame(&mut ed, 60, 12);
    seek_to_line(&mut ed, 0);
    frame(&mut ed, 60, 12);

    ed.report(Severity::Error, "below");
    frame(&mut ed, 60, 12);
    assert_eq!(summary(&ed).as_deref(), Some("1 error (see :messages)"));

    seek_to_line(&mut ed, 30);
    frame(&mut ed, 60, 12);
    assert_eq!(summary(&ed), None);
}

#[test]
fn a_message_logged_after_the_view_was_built_wakes_the_loop_while_it_is_shown() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    open_messages(&mut ed);
    frame(&mut ed, 60, 12);

    ed.report(Severity::Warning, "during the frame");
    assert_eq!(ed.wake_timeout(), Some(std::time::Duration::ZERO));
}

#[test]
fn a_hidden_view_does_not_wake_the_loop() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    open_messages(&mut ed);
    ed.execute_typed("bp", None)
        .expect("a previous buffer exists");
    frame(&mut ed, 60, 12);

    ed.report(Severity::Warning, "second");
    assert_ne!(ed.wake_timeout(), Some(std::time::Duration::ZERO));
}

#[test]
fn a_hidden_view_catches_up_when_it_is_shown_again() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);
    ed.execute_typed("bp", None)
        .expect("a previous buffer exists");
    let before = buffer_text(&ed, bid);

    ed.report(Severity::Warning, "second");
    frame(&mut ed, 60, 12);
    assert_eq!(buffer_text(&ed, bid), before, "a hidden view waits");

    ed.execute_typed("bn", None).expect("the view is next");
    assert_eq!(ed.focused_buffer_id(), bid, "sanity: the view is shown");
    frame(&mut ed, 60, 12);
    assert!(buffer_text(&ed, bid).contains("second"));
}

#[test]
fn settling_brings_a_shown_view_up_to_date() {
    let mut ed = open_headless(None);
    ed.report(Severity::Warning, "first");
    let bid = open_messages(&mut ed);

    ed.report(Severity::Warning, "second");
    ed.settle();

    assert!(buffer_text(&ed, bid).contains("second"));
}

#[test]
fn a_cursor_on_an_entry_the_floor_hides_moves_to_the_next_shown_entry() {
    let mut ed = open_headless(None);
    run_set(&mut ed, "global messages-min-severity=trace").expect("trace is a valid floor");
    ed.report(Severity::Warning, "w0");
    ed.report(Severity::Trace, "t0");
    ed.report(Severity::Warning, "w1");
    let bid = open_messages(&mut ed);
    let pane = ed.view.focused_pane();
    seek_to_line(&mut ed, 1);

    run_set(&mut ed, "global messages-min-severity=warning").expect("warning is a valid floor");
    frame(&mut ed, 60, 12);

    assert_eq!(head_line(&ed, pane, bid), 1, "on w1, the entry after t0");
}

#[test]
fn a_cursor_on_the_last_entry_the_floor_hides_moves_to_the_entry_before() {
    let mut ed = open_headless(None);
    run_set(&mut ed, "global messages-min-severity=trace").expect("trace is a valid floor");
    ed.report(Severity::Warning, "w0");
    ed.report(Severity::Trace, "t0");
    let bid = open_messages(&mut ed);
    let pane = ed.view.focused_pane();

    run_set(&mut ed, "global messages-min-severity=warning").expect("warning is a valid floor");
    frame(&mut ed, 60, 12);

    assert_eq!(head_line(&ed, pane, bid), 0);
}
