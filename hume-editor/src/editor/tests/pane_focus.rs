use super::*;
use crate::editor::EditorState;
use crate::editor::commands::{
    cmd_pane_focus_down, cmd_pane_focus_left, cmd_pane_focus_next, cmd_pane_focus_right,
    cmd_pane_focus_up, open_pane,
};
use crate::editor::error::CommandError;
use hume_engine::pipeline::{Direction, EngineView, PaneId, RenderContext};
use hume_ops::MotionMode;

type Cmd = fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>;

// Build a deterministic 2x2 pane grid in `editor_from`'s scratch buffer.
//
// `open_pane` seeds per-pane maps but does not touch `view.layout`; the test
// splits the layout itself. `prepare_frame` partitions the 100x50 pane area
// (height 50 = terminal_height 51 minus 1 statusline row) and the cache ends
// up in DFS pre-order: pid_a (TL), pid_c (TR), pid_b (BL), pid_d (BR).
fn build_2x2() -> (Editor, [PaneId; 4]) {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id; // top-left
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.view
        .layout
        .split_leaf(pid_a, pid_b, Direction::Vertical, 0.5); // stack a over b
    let pid_c = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.view
        .layout
        .split_leaf(pid_a, pid_c, Direction::Horizontal, 0.5); // a | c (top row)
    let pid_d = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.view
        .layout
        .split_leaf(pid_b, pid_d, Direction::Horizontal, 0.5); // b | d (bottom row)
    let mut ctx = RenderContext::new();
    ed.prepare_frame(100, 51, &mut ctx); // cache: DFS order a, c, b, d
    (ed, [pid_a, pid_c, pid_b, pid_d])
}

/// Drive `cmd` on `ed`, asserting `focused_pane_id` is `expected` afterwards
/// and no status message was raised.
fn expect_focus(ed: &mut Editor, start: PaneId, expected: PaneId, cmd: Cmd) {
    ed.state.focused_pane_id = start;
    cmd(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
    assert_eq!(ed.state.focused_pane_id, expected);
    assert!(ed.state.status_msg.is_none());
}

/// Assert the directional command is a silent no-op from `start` (focused pane
/// unchanged, no status message).
fn expect_noop(ed: &mut Editor, start: PaneId, cmd: Cmd) {
    expect_focus(ed, start, start, cmd);
}

#[test]
fn t4_directional_focus_in_2x2_grid() {
    let (mut ed, [pid_a, pid_c, pid_b, pid_d]) = build_2x2();
    // start | left     | right    | up       | down
    // a(TL) | —        | pid_c    | —        | pid_b
    // c(TR) | pid_a    | —        | —        | pid_d
    // b(BL) | —        | pid_d    | pid_a    | —
    // d(BR) | pid_b    | —        | pid_c    | —

    expect_noop(&mut ed, pid_a, cmd_pane_focus_left);
    expect_focus(&mut ed, pid_a, pid_c, cmd_pane_focus_right);
    expect_noop(&mut ed, pid_a, cmd_pane_focus_up);
    expect_focus(&mut ed, pid_a, pid_b, cmd_pane_focus_down);

    expect_focus(&mut ed, pid_c, pid_a, cmd_pane_focus_left);
    expect_noop(&mut ed, pid_c, cmd_pane_focus_right);
    expect_noop(&mut ed, pid_c, cmd_pane_focus_up);
    expect_focus(&mut ed, pid_c, pid_d, cmd_pane_focus_down);

    expect_noop(&mut ed, pid_b, cmd_pane_focus_left);
    expect_focus(&mut ed, pid_b, pid_d, cmd_pane_focus_right);
    expect_focus(&mut ed, pid_b, pid_a, cmd_pane_focus_up);
    expect_noop(&mut ed, pid_b, cmd_pane_focus_down);

    expect_focus(&mut ed, pid_d, pid_b, cmd_pane_focus_left);
    expect_noop(&mut ed, pid_d, cmd_pane_focus_right);
    expect_focus(&mut ed, pid_d, pid_c, cmd_pane_focus_up);
    expect_noop(&mut ed, pid_d, cmd_pane_focus_down);
}

/// Regression: tie-break must use perpendicular *center* distance, not origin
/// distance. Layout: `a` full-height on the left; the right column split into
/// a short top pane `b` (rows 0-15, center row 7) and a tall bottom pane `c`
/// (rows 15-50, center row 32). `a`'s center row is 25.
///
/// Both `b` and `c` tie on primary-axis gap (0, both touch `a`'s right edge).
/// Center distance favors `c` (|25-32|=7 vs |25-7|=18); an origin-distance
/// tie-break would wrongly favor `b` (|0-0|=0 vs |0-15|=15).
#[test]
fn t4_tie_break_uses_center_distance_not_origin() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.view
        .layout
        .split_leaf(pid_a, pid_b, Direction::Horizontal, 0.5); // a | b
    let pid_c = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.view
        .layout
        .split_leaf(pid_b, pid_c, Direction::Vertical, 0.3); // b (top, short) / c (bottom, tall)
    let mut ctx = RenderContext::new();
    ed.prepare_frame(100, 51, &mut ctx);

    ed.state.focused_pane_id = pid_a;
    cmd_pane_focus_right(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
    assert_eq!(ed.state.focused_pane_id, pid_c);
}

#[test]
fn t4_pane_focus_next_cycles_dfs_order_and_wraps() {
    let (mut ed, [pid_a, pid_c, pid_b, pid_d]) = build_2x2();

    // DFS cache order: a → c → b → d → a ...
    for (start, expected) in [
        (pid_a, pid_c),
        (pid_c, pid_b),
        (pid_b, pid_d),
        (pid_d, pid_a),
    ] {
        ed.state.focused_pane_id = start;
        cmd_pane_focus_next(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
        assert_eq!(ed.state.focused_pane_id, expected);
        assert!(ed.state.status_msg.is_none());
    }
}

#[test]
fn t4_pane_focus_next_single_pane_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = ed.state.focused_pane_id;
    cmd_pane_focus_next(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
    assert_eq!(ed.state.focused_pane_id, before);
    assert!(ed.state.status_msg.is_none());
}

#[test]
fn t4_directional_no_neighbour_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = ed.state.focused_pane_id;
    for cmd in [
        cmd_pane_focus_left as Cmd,
        cmd_pane_focus_right as Cmd,
        cmd_pane_focus_up as Cmd,
        cmd_pane_focus_down as Cmd,
    ] {
        cmd(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
        assert_eq!(ed.state.focused_pane_id, before);
        assert!(ed.state.status_msg.is_none());
    }
}
