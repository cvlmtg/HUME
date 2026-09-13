use super::*;
use hume_editing::selection::Selection;
use hume_grid::Rect;
use pretty_assertions::assert_eq;
use termina::event::{Event as TerminalEvent, Modifiers, MouseButton, MouseEvent, MouseEventKind};

fn mouse_drag(x: u16, y: u16) -> TerminalEvent {
    TerminalEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: x,
        row: y,
        modifiers: Modifiers::NONE,
    })
}

/// `tab`'s start column in the synced tabline view — computed the same way
/// `tabline_click` resolves one, without hardcoding a column.
fn tab_start_x(ed: &Editor, tab: crate::editor::tab::TabId) -> u16 {
    let guard = ed.state.tabline_view.read();
    let idx = guard
        .tabs
        .iter()
        .position(|e| e.id == tab)
        .expect("tab must be in the synced view");
    crate::tabline::tab_extents(&guard.tabs, guard.scroll, 0, 40).ranges[idx - guard.scroll].0
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
    ed.view.last_pane_area = Rect::new(0, 0, 80, 24);
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on *that* line's
    // trailing '\n' — a blank, auto-indented line (buffer is now
    // "  x\n  \ncd\n", cursor at char 6). `autoindent_pending` is set, so
    // exiting Insert now will trim that "  ".
    assert_eq!(state(&ed), "  x\n  -[\n]>cd\n");

    // Click on 'd' (line 2, column 1, no gutter in test harness) to exit
    // Insert mode via the mouse.
    ed.handle_input(mouse_left_down(1, 2));

    assert_eq!(ed.state.mode, Mode::Normal);
    // The blank line's "  " is trimmed on exit (buffer shrinks to
    // "  x\n\ncd\n"), and the click must land on 'd' in the *new* buffer —
    // not at the stale pre-trim offset, which would land 2 chars past 'd'
    // (out of bounds before the fix, since the buffer is now 2 chars
    // shorter than it was when the click coordinates were captured).
    assert_eq!(state(&ed), "  x\n\nc-[d]>\n");
}

// ── Drag ──────────────────────────────────────────────────────────────────

/// A left-drag after a click extends the selection from the click's anchor
/// to the drag's resolved head.
#[test]
fn drag_extends_selection_from_click_anchor() {
    let mut ed = editor_from("-[0]>123456789\n");
    ed.view.last_pane_area = Rect::new(0, 0, 80, 24);

    ed.handle_input(mouse_left_down(0, 0)); // anchor at char 0
    ed.handle_input(mouse_drag(4, 0)); // head at char 4 ('4')

    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), co(0));
    assert_eq!(sel.head(), co(4), "drag head must resolve to content col 4");
}

/// A drag whose coordinates fall inside a *different* pane's rect (a fast
/// mouse move during a `:vsplit` drag easily crosses the seam) must be
/// ignored, not translated as if it were still in the originating pane —
/// `rect_relative`'s `x - rect.x`/`y - rect.y` would otherwise underflow when
/// the drag lands left of/above the originating pane's own rect origin.
#[test]
fn drag_crossing_into_a_different_pane_is_ignored_not_underflowed() {
    let mut ed =
        editor_from("-[0]>123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\n");
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id(); // vsplit focuses the new (right) pane

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Click pane B (right half, gutter 4): screen col 57 = rect.x(50) +
    // gutter(4) + content col 3 (see vsplit_click_... below for the geometry).
    ed.handle_input(mouse_left_down(57, 0));
    assert_eq!(ed.state.focus.id(), pid_b);
    let head_after_click = ed.current_selections().primary().head();

    // Drag to col 0 — inside pane A's rect (x ∈ [0, 49)), left of pane B's
    // own rect.x (50). Without `rect_relative`'s `contains` guard, `x - rect.x`
    // underflows a u16 subtraction.
    ed.handle_input(mouse_drag(0, 0));

    assert_eq!(
        ed.current_selections().primary().head(),
        head_after_click,
        "a drag that crosses into another pane's rect must be ignored"
    );
}

// ── Scroll wheel ─────────────────────────────────────────────────────────

/// The scroll wheel moves the viewport AND every cursor together, by the
/// same `mouse_scroll_lines` amount — not just the viewport. Module doc:
/// "Moving the cursor with the viewport prevents `ensure_cursor_visible`
/// from snapping the viewport back on the next frame."
#[test]
fn scroll_up_moves_viewport_and_cursor_together() {
    let mut lines = String::from("-[l]>ine0\n");
    for i in 1..30 {
        lines.push_str(&format!("line{i}\n"));
    }
    let mut ed = editor_from(&lines);

    // Scroll the viewport down to line 10 first, then place the cursor at
    // that same top line — the state a real scroll-then-click leaves
    // behind, and the case that distinguishes "viewport moved" from
    // "cursor moved with it".
    let pid = ed.state.focus.id();
    ed.view.panes[pid].viewport.top_line = hume_rope::line::ContentLine::new(10);
    let head = ed
        .doc()
        .text()
        .line_to_char(hume_rope::line::RopeyLine::new(10));
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(head)));

    ed.handle_input(mouse_wheel(false));

    assert_eq!(
        ed.view.panes[pid].viewport.top_line,
        hume_rope::line::ContentLine::new(7),
        "viewport must scroll up by mouse_scroll_lines (3)"
    );
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        hume_rope::line::ContentLine::new(7),
        "cursor must move with the viewport so it stays at the same screen row"
    );
}

/// At the top of the document, the viewport can't move — and per
/// `mouse_scroll`'s own `vp_before != vp_after` guard, the cursor must
/// stay put too, not silently drift up on every wheel tick.
#[test]
fn scroll_up_at_top_moves_neither_viewport_nor_cursor() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    ed.handle_input(mouse_wheel(false));

    let pid = ed.state.focus.id();
    assert_eq!(
        ed.view.panes[pid].viewport.top_line,
        hume_rope::line::ContentLine::new(0)
    );
    assert_eq!(ed.current_selections().primary().head(), co(0));
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
    let pid_a = ed.state.focus.id();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id(); // vsplit focuses the new pane
    assert_ne!(pid_a, pid_b);
    let bid = ed.view.panes[pid_a].buffer_id; // vsplit shares the source buffer

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let head = |ed: &Editor, pid| ed.state.panes.state[pid][bid].selections.primary().head();
    assert_eq!(
        head(&ed, pid_a),
        co(0),
        "sanity: both panes start at char 0"
    );
    assert_eq!(
        head(&ed, pid_b),
        co(0),
        "sanity: both panes start at char 0"
    );

    // Click pane A (unfocused, left half): screen col 7 = rect.x(0) +
    // gutter(0) + content col 7 → the '7' in "0123456789...".
    ed.handle_input(mouse_left_down(7, 0));
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "click in pane A must move focus there"
    );
    assert_eq!(head(&ed, pid_a), co(7), "must land on content col 7 ('7')");
    assert_eq!(
        head(&ed, pid_b),
        co(0),
        "pane B's selection must be untouched"
    );

    // Click pane B (now unfocused, right half): screen col 57 = rect.x(50)
    // + gutter(4) + content col 3 → the '3'.
    ed.handle_input(mouse_left_down(57, 0));
    assert_eq!(
        ed.state.focus.id(),
        pid_b,
        "click in pane B must move focus back there"
    );
    assert_eq!(head(&ed, pid_b), co(3), "must land on content col 3 ('3')");
    assert_eq!(
        head(&ed, pid_a),
        co(7),
        "pane A's selection from the first click must survive untouched"
    );

    // Click the statusline (row 24 — usable pane height is 24 after the
    // statusline reservation, so row 24 is outside every pane's rect).
    ed.handle_input(mouse_left_down(10, 24));
    assert_eq!(
        ed.state.focus.id(),
        pid_b,
        "a click outside every pane rect must not move focus"
    );
    assert_eq!(
        head(&ed, pid_a),
        co(7),
        "statusline click must not move pane A"
    );
    assert_eq!(
        head(&ed, pid_b),
        co(3),
        "statusline click must not move pane B"
    );
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
    let pid_b = ed.state.focus.id(); // split focuses the new (bottom) pane
    let bid = ed.view.panes[pid_b].buffer_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Absolute row 15 = pane B's rect.y(12) + relative row 3 → buffer line 3
    // ("DDDD"). Column 6 = gutter(4) + content col 2 → 'D' (any content col
    // 0..3 lands on 'D' — the whole line is the same character).
    ed.handle_input(mouse_left_down(6, 15));

    let sel = ed.state.panes.state[pid_b][bid].selections.primary();
    assert_eq!(
        ed.doc().text().char_to_line(sel.head()),
        hume_rope::line::ContentLine::new(3),
        "row 15 in pane B (rect.y=12) must resolve to buffer line 3, not \
         raw row 15 in the buffer (which would be past EOF) or be rejected \
         outright (row 15 >= pane B's own viewport.height of 12)"
    );
}

// ── Tabline click ────────────────────────────────────────────────────────────

/// A click on the tab bar switches to the tab it lands on, and — unlike a
/// click that misses every pane's rect (the statusline case above) — never
/// falls through to `pane_at_screen_pos`.
#[test]
fn click_on_a_tab_switches_to_it() {
    let mut ed = editor_from("-[a]>bc\n");
    let tab_a = ed.state.tabs.current();
    let pid_a = ed.state.focus.id();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    assert_ne!(tab_a, tab_b, "setup: tabnew must have opened a second tab");

    frame(&mut ed, 40, 10);
    assert_eq!(
        ed.view.last_pane_area.y, 1,
        "setup: the tabline must have reserved row 0"
    );

    // Row 0, column 0 — inside the padded label of the first tab (tab A,
    // scroll starts at 0 with no overflow indicator at this width).
    ed.handle_input(mouse_left_down(0, 0));

    assert_eq!(ed.state.tabs.current(), tab_a, "click must switch to tab A");
    assert_eq!(ed.state.focus.id(), pid_a);
}

/// A click past every tab's extent (the row's blank tail) is a no-op —
/// it must not fall through to pane hit-testing either, since the tabline
/// row sits outside every pane's rect regardless.
#[test]
fn click_on_the_tabline_s_blank_tail_is_a_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    let pid_b = ed.state.focus.id();

    frame(&mut ed, 40, 10);

    // Column 39 (last column, 40-wide row): well past both tabs' short
    // labels (" *scratch* │ *scratch* " is nowhere near 39 columns).
    ed.handle_input(mouse_left_down(39, 0));

    assert_eq!(ed.state.tabs.current(), tab_b, "no tab switch");
    assert_eq!(ed.state.focus.id(), pid_b, "no pane focus change");
}

/// A terminal too short to fit the tab bar plus the statusline (a single
/// row) pushes both `pane_area` and `tabbar_area` into their degenerate
/// branches — the statusline unconditionally owns that one row (`render`'s
/// own `sl_y = area.bottom() - 1`), so the tab bar must yield it rather than
/// have both chrome rows paint on top of each other, and a click there must
/// hit the statusline, not switch tabs (code review fix #6, commit range
/// 82ce1d7c..10a81a18).
#[test]
fn a_one_row_terminal_leaves_the_tabbar_no_room_and_a_click_there_does_not_switch_tabs() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();

    // chrome_height = 1 (tab bar) + 1 (statusline) = 2, not less than a
    // 1-row terminal — degenerate.
    frame(&mut ed, 40, 1);
    assert_eq!(
        ed.view.last_pane_area.height, 0,
        "setup: pane area is degenerate"
    );
    assert_eq!(
        ed.view.tabbar_area(ed.view.last_terminal_area).height,
        0,
        "the tab bar must yield its row to the statusline, not paint over it"
    );

    ed.handle_input(mouse_left_down(0, 0));

    assert_eq!(
        ed.state.tabs.current(),
        tab_b,
        "a click on row 0 must not switch tabs once the tab bar has no room there"
    );

    insta::assert_snapshot!(render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 1)));
}

/// A click on another tab's label must leave the outgoing pane the same way
/// a click on another *pane* does: exit Insert while it's still focused, so
/// the blank-line indent trim and the edit-group commit land on the
/// buffer actually being left, not on the tab just switched to.
#[test]
fn clicking_another_tab_while_in_insert_exits_insert_and_commits_the_outgoing_pane() {
    let tmp = safe_tempdir();
    let path = tmp.path().join("other.txt");
    std::fs::write(&path, "zz\n").unwrap();

    // "  x\ncd\n": cursor on line 0's own trailing '\n', same setup as
    // `click_after_blank_line_trim_lands_on_correct_char`.
    let mut ed = editor_from("  x-[\n]>cd\n");
    let tab_a = ed.state.tabs.current();
    let bid_a = ed.focused_buffer_id();

    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();
    let tab_b = ed.state.tabs.current();
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: distinct buffers");

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_a, "setup: back on A");

    frame(&mut ed, 40, 10);

    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on that blank,
    // auto-indented line — `autoindent_pending` is set, so exiting Insert
    // now will trim it.
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "  x\n  \ncd\n",
        "setup: blank auto-indented line pending trim"
    );
    assert_eq!(ed.state.mode(), Mode::Insert, "setup: still typing in A");

    // Click tab B's own label.
    let start_b = tab_start_x(&ed, tab_b);
    ed.handle_input(mouse_left_down(start_b, 0));

    assert_eq!(ed.state.tabs.current(), tab_b, "click switched to tab B");
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "leaving A via a tab click must exit Insert, same as a pane click"
    );
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "  x\n\ncd\n",
        "A's blank auto-indented line's whitespace must have been trimmed on exit"
    );
    assert_eq!(
        ed.state.buffers.get(bid_b).text().to_string(),
        "zz\n",
        "B's own buffer must be untouched by A's exit"
    );
}

/// A click on another tab's label must commit an open paste session on the
/// outgoing pane, same as every keyboard-dispatched focus switch does via
/// `step_paste_commit` — `tabline_click` reaches `switch_to_tab` directly,
/// bypassing dispatch entirely, so `focus_pane` is the only remaining place
/// that can close the gap (code review fix #2, commit range
/// 48c11211..ebc3b2e0). Left uncommitted, `commit_paste_session`'s own
/// debug assert fires on the very next dispatched command.
#[test]
fn clicking_another_tab_commits_the_outgoing_pane_s_open_paste_session() {
    let mut ed = editor_from("-[hello]>world\n");
    let tab_a = ed.state.tabs.current();
    let pid_a = ed.state.focus.id();
    let bid_a = ed.focused_buffer_id();

    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_a, "setup: back on A");

    frame(&mut ed, 40, 10);

    ed.feed_key(key('d')); // delete "hello" → ring head = ["hello"]
    ed.feed_key(key('p')); // paste it back — opens a paste session on A

    assert!(
        ed.state.panes.state[pid_a][bid_a].paste_group.is_some(),
        "setup: paste session open on A"
    );

    let start_b = tab_start_x(&ed, tab_b);
    ed.handle_input(mouse_left_down(start_b, 0));

    assert_eq!(ed.state.tabs.current(), tab_b, "click switched to tab B");
    assert!(
        ed.state.panes.state[pid_a][bid_a].paste_group.is_none(),
        "focus_pane must commit A's open paste session before leaving it"
    );

    // The commit having actually happened (not just the field having been
    // cleared some other way) shows up as one committed undo step on A.
    assert!(ed.state.buffers.get(bid_a).can_undo());

    // No open session left anywhere means the next dispatched command can't
    // trip `commit_paste_session`'s debug assert.
    ed.feed_key(key('x'));
}

/// A drag right after a tab click must not extend a selection from the
/// anchor the previous tab's click left behind — that anchor belongs to a
/// buffer that isn't even focused anymore.
#[test]
fn drag_right_after_a_tab_click_does_not_extend_from_the_stale_anchor() {
    let mut ed = editor_from("-[a]>bc\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_a, "setup: back on A");

    frame(&mut ed, 40, 10);

    // A pane click on A's own content first, to seed a drag anchor the way
    // any ordinary click would — then a tab click to B, then a drag. The
    // drag must not resolve against the first click's now-stale anchor.
    // Column 0: A is `editor_from`'s original pane, which registers no
    // gutter columns (see `vsplit_click_focuses_and_resolves_against_the_clicked_pane`'s
    // own doc), so column 0 is content, not gutter.
    ed.handle_input(mouse_left_down(0, 1));
    assert!(ed.state.mouse_drag_anchor.is_some(), "setup: anchor seeded");

    let start_b = tab_start_x(&ed, tab_b);
    ed.handle_input(mouse_left_down(start_b, 0));
    assert_eq!(ed.state.tabs.current(), tab_b);
    assert!(
        ed.state.mouse_drag_anchor.is_none(),
        "a tab click must clear the previous click's drag anchor"
    );

    ed.handle_input(mouse_drag(1, 1));
    // With no anchor, the drag is a no-op — the selection must stay
    // whatever the tab switch left it at, not extend from A's old anchor.
    assert!(ed.state.mouse_drag_anchor.is_none());
}
