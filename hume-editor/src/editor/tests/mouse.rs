use super::*;
use pretty_assertions::assert_eq;
use termina::event::{Event, Modifiers, MouseButton, MouseEvent, MouseEventKind};

fn mouse_left_down(col: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: Modifiers::NONE,
    })
}

/// Regression: `end_insert_session` can mutate the buffer (the blank-line
/// indent trim, code review fix #3) — a mouse click that exits Insert mode
/// must recompute its char offset AFTER that mutation, not before, or a
/// stale offset can land past the shrunk buffer's end (fix #2).
#[test]
fn click_after_blank_line_trim_lands_on_correct_char() {
    // "  x\ncd\n": enter Insert with the cursor on line 0's own trailing '\n'.
    let mut ed = editor_from("  x-[\n]>cd\n");
    // The click below is hit-tested against pane rects, which only
    // `prepare_frame` normally populates — set it directly, matching
    // `Pane::new`'s default 80×24 viewport, since this test exercises the
    // click/mode-transition path, not a full frame.
    ed.view.last_pane_area = ratatui::layout::Rect::new(0, 0, 80, 24);
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on *that* line's
    // trailing '\n' — a blank, auto-indented line (buffer is now
    // "  x\n  \ncd\n", cursor at char 6). `autoindent_pending` is set, so
    // exiting Insert now will trim that "  ".
    assert_eq!(state(&ed), "  x\n  -[\n]>cd\n");

    // Click on 'd' (line 2, column 1, no gutter in test harness) to exit
    // Insert mode via the mouse.
    ed.handle_event(mouse_left_down(1, 2));

    assert_eq!(ed.state.mode, Mode::Normal);
    // The blank line's "  " is trimmed on exit (buffer shrinks to
    // "  x\n\ncd\n"), and the click must land on 'd' in the *new* buffer —
    // not at the stale pre-trim offset, which would land 2 chars past 'd'
    // (out of bounds before the fix, since the buffer is now 2 chars
    // shorter than it was when the click coordinates were captured).
    assert_eq!(state(&ed), "  x\n\nc-[d]>\n");
}

// ── Multi-pane hit-testing ────────────────────────────────────────────────

/// After `:vsplit`, a click must resolve against the pane *under the
/// pointer*, not the currently focused one, and its coordinates must be
/// translated into that pane's own rect (subtracting the rect's origin) —
/// not used as if they were already pane-relative.
///
/// Terminal width 100, one real buffer line: `:vsplit` (1-column seam,
/// `split_rect`'s `0.5` ratio) gives pane A `x ∈ [0, 49)`, pane B
/// `x ∈ [50, 100)` — the same halves `vsplit_sizes_both_panes_from_layout`
/// (`multi_pane.rs`) pins.
///
/// Gutter width differs *by pane*, not just by test — worth spelling out
/// since it's easy to assume otherwise: pane A is the original
/// `editor_from`/`Pane::new` pane, which registers no gutter columns at all
/// (gutter width 0); pane B is `:vsplit`'s freshly-opened pane, built through
/// `open_pane` → `build_pane`, which *does* register the real line-number +
/// sign columns (gutter width 4 here: `LineNumberColumn` digit_count(1) + 1
/// padding = 2, `SignColumn`'s default width 2, `signcolumn` mode `Always`
/// so it never collapses).
#[test]
fn vsplit_click_focuses_and_resolves_against_the_clicked_pane() {
    let mut ed =
        editor_from("-[0]>123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id; // vsplit focuses the new pane
    assert_ne!(pid_a, pid_b);
    let bid = ed.view.panes[pid_a].buffer_id; // vsplit shares the source buffer

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(100, 25, &mut ctx);

    let head = |ed: &Editor, pid| ed.state.panes.state[pid][bid].selections.primary().head();
    assert_eq!(head(&ed, pid_a), 0, "sanity: both panes start at char 0");
    assert_eq!(head(&ed, pid_b), 0, "sanity: both panes start at char 0");

    // Click pane A (unfocused, left half): screen col 7 = rect.x(0) +
    // gutter(0) + content col 7 → the '7' in "0123456789...".
    ed.handle_event(mouse_left_down(7, 0));
    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "click in pane A must move focus there"
    );
    assert_eq!(head(&ed, pid_a), 7, "must land on content col 7 ('7')");
    assert_eq!(head(&ed, pid_b), 0, "pane B's selection must be untouched");

    // Click pane B (now unfocused, right half): screen col 57 = rect.x(50)
    // + gutter(4) + content col 3 → the '3'.
    ed.handle_event(mouse_left_down(57, 0));
    assert_eq!(
        ed.state.focused_pane_id, pid_b,
        "click in pane B must move focus back there"
    );
    assert_eq!(head(&ed, pid_b), 3, "must land on content col 3 ('3')");
    assert_eq!(
        head(&ed, pid_a),
        7,
        "pane A's selection from the first click must survive untouched"
    );

    // Click the statusline (row 24 — usable pane height is 24 after the
    // statusline reservation, so row 24 is outside every pane's rect).
    ed.handle_event(mouse_left_down(10, 24));
    assert_eq!(
        ed.state.focused_pane_id, pid_b,
        "a click outside every pane rect must not move focus"
    );
    assert_eq!(head(&ed, pid_a), 7, "statusline click must not move pane A");
    assert_eq!(head(&ed, pid_b), 3, "statusline click must not move pane B");
}

/// The stacked-split analogue: a click's *row* must also be translated by
/// the clicked pane's rect origin, not just its column. Before the fix, the
/// lower pane's own `viewport.height` guard alone rejected every click at an
/// absolute row at or past it — which, for the lower half of a stacked
/// split, is *every* row inside that pane, since its rect starts well past
/// row 0.
///
/// Terminal height 25 (24 usable after the statusline): `:split` (1-row
/// seam, ratio 0.5) gives pane A (top) `y ∈ [0, 11)`, pane B (bottom)
/// `y ∈ [12, 24)` — the same halves `split_sizes_both_panes_stacked`
/// (`multi_pane.rs`) pins. Pane B is `:split`'s freshly-opened pane (via
/// `build_pane`), so its gutter is 4 (line-number digit_count(5) + 1 = 2,
/// sign column default width 2 — see the `:vsplit` test above for why this
/// differs from the source pane).
#[test]
fn stacked_split_click_translates_row_by_the_panes_rect_origin() {
    let mut ed = editor_from("-[A]>\nBBBB\nCCCC\nDDDD\nEEEE\n");
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id; // split focuses the new (bottom) pane
    let bid = ed.view.panes[pid_b].buffer_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    // Absolute row 15 = pane B's rect.y(12) + relative row 3 → buffer line 3
    // ("DDDD"). Column 6 = gutter(4) + content col 2 → 'D' (any content col
    // 0..3 lands on 'D' — the whole line is the same character).
    ed.handle_event(mouse_left_down(6, 15));

    let sel = ed.state.panes.state[pid_b][bid].selections.primary();
    assert_eq!(
        ed.doc().text().char_to_line(sel.head()),
        3,
        "row 15 in pane B (rect.y=12) must resolve to buffer line 3, not \
         raw row 15 in the buffer (which would be past EOF) or be rejected \
         outright (row 15 >= pane B's own viewport.height of 12)"
    );
}
