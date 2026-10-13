use super::*;
use crate::editor::commands::{
    FocusedPane, cmd_pane_focus_down, cmd_pane_focus_left, cmd_pane_focus_next,
    cmd_pane_focus_right, cmd_pane_focus_up, open_pane_in_layout,
};
use crate::editor::registry::Count;
use hume_engine::pipeline::{Direction, FocusedLayout, LayoutTree, PaneId, RenderContext};
use hume_ops::MotionMode;

type Cmd = crate::editor::registry::FocusedCmdFn;

// Build a deterministic 2x2 pane grid in `editor_from`'s scratch buffer.
//
// `prepare_frame` partitions the 100x50 pane area (height 50 =
// terminal_height 51 minus 1 statusline row) and the cache ends up in DFS
// pre-order: pid_a (TL), pid_c (TR), pid_b (BL), pid_d (BR).
fn build_2x2() -> (Editor, [PaneId; 4]) {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.view.focused_pane(); // top-left
    let pid_b = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Vertical); // stack a over b
    let pid_c = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Horizontal); // a | c (top row)
    ed.state.focus.set_for_test(&mut ed.view, pid_b);
    let pid_d = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Horizontal); // b | d (bottom row)
    ed.state.focus.set_for_test(&mut ed.view, pid_a);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // cache: DFS order a, c, b, d
    (ed, [pid_a, pid_c, pid_b, pid_d])
}

/// Drive `cmd` on `ed`, asserting `focus` is `expected` afterwards and no
/// status message was raised.
fn expect_focus(ed: &mut Editor, start: PaneId, expected: PaneId, cmd: Cmd) {
    ed.state.focus.set_for_test(&mut ed.view, start);
    let fp = FocusedPane::current(&ed.view);
    cmd(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::new(Some(1)),
        MotionMode::Move,
    )
    .unwrap();
    assert_eq!(ed.view.focused_pane(), expected);
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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Horizontal);
    let pid_c = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Horizontal);
    // The tree `open_pane_in_layout` grafted `pid_b`/`pid_c` into above is
    // overwritten wholesale here; only that both were created through it
    // matters, not where its own `split_leaf` call placed them. Built
    // directly (not via `split_leaf`, which now equalizes every same-axis
    // split) so `b`/`c` keep the deliberate short/tall asymmetry this
    // tie-break test needs. Rebuilds the tree over the same pane set created
    // above; nothing detached or leaked: the discarded tree names the same
    // pool entries the replacement does, just reshaped.
    let tree = LayoutTree::Split {
        direction: Direction::Horizontal,
        ratio: 0.5,
        children: Box::new((
            LayoutTree::Leaf(pid_a),
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.3,
                children: Box::new((LayoutTree::Leaf(pid_b), LayoutTree::Leaf(pid_c))),
            },
        )),
    };
    let layout = FocusedLayout::from_parts(tree, pid_a).expect("pid_a is a leaf of the tree");
    let _ = ed.state.focus.replace_layout_for_test(&mut ed.view, layout);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    ed.state.focus.set_for_test(&mut ed.view, pid_a);
    let fp = FocusedPane::current(&ed.view);
    cmd_pane_focus_right(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::new(Some(1)),
        MotionMode::Move,
    )
    .unwrap();
    assert_eq!(ed.view.focused_pane(), pid_c);
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
        ed.state.focus.set_for_test(&mut ed.view, start);
        let fp = FocusedPane::current(&ed.view);
        cmd_pane_focus_next(
            &mut ed.state,
            &mut ed.view,
            fp,
            Count::new(Some(1)),
            MotionMode::Move,
        )
        .unwrap();
        assert_eq!(ed.view.focused_pane(), expected);
        assert!(ed.state.status_msg.is_none());
    }
}

#[test]
fn t4_pane_focus_next_single_pane_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = ed.view.focused_pane();
    let fp = FocusedPane::current(&ed.view);
    cmd_pane_focus_next(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::new(Some(1)),
        MotionMode::Move,
    )
    .unwrap();
    assert_eq!(ed.view.focused_pane(), before);
    assert!(ed.state.status_msg.is_none());
}

#[test]
fn t4_directional_no_neighbour_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = ed.view.focused_pane();
    let fp = FocusedPane::current(&ed.view);
    for cmd in [
        cmd_pane_focus_left as Cmd,
        cmd_pane_focus_right as Cmd,
        cmd_pane_focus_up as Cmd,
        cmd_pane_focus_down as Cmd,
    ] {
        cmd(
            &mut ed.state,
            &mut ed.view,
            fp,
            Count::new(Some(1)),
            MotionMode::Move,
        )
        .unwrap();
        assert_eq!(ed.view.focused_pane(), before);
        assert!(ed.state.status_msg.is_none());
    }
}
