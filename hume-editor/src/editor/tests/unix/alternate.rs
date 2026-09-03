use super::*;
use hume_scripting::host::CommandHost;

#[test]
fn alternate_buffer_is_previous_focused() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    assert_ne!(id_a, id_b, "A and B must be distinct");
    assert_eq!(ed.alternate_buffer(), Some(id_a));
}

#[test]
fn goto_alternate_buffer_switches_to_alternate_and_is_involutive() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        "goto-alternate-buffer must switch to alternate"
    );

    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_b,
        "goto-alternate-buffer again returns to starting buffer"
    );
}

#[test]
fn goto_alternate_buffer_pushes_jump_entry() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();

    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    assert_ne!(
        ed.focused_buffer_id(),
        id_before,
        "goto-alternate-buffer changes focus"
    );
    live_host!(ed)
        .run_command_sync("jump-backward", Some(1), false, None)
        .expect("jump-backward must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "jump-backward retraces goto-alternate-buffer"
    );
}

#[test]
fn colon_e_hash_opens_alternate() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let buf_count = ed.state.buffers.len();

    type_cmd(&mut ed, ":e #");
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        ":e # must switch to alternate"
    );
    assert_eq!(
        ed.state.buffers.len(),
        buf_count,
        ":e # must not open a duplicate"
    );
}

#[test]
fn alternate_follows_focus_order_not_open_order() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let (p3, _t3) = temp_file("file3\n");
    let c1 = std::fs::canonicalize(&p1).unwrap();
    let c2 = std::fs::canonicalize(&p2).unwrap();
    let c3 = std::fs::canonicalize(&p3).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    type_cmd_event(&mut ed, &format!(":e {}", c1.display()));
    let id_a = ed.focused_buffer_id();
    type_cmd_event(&mut ed, &format!(":e {}", c2.display()));
    type_cmd_event(&mut ed, &format!(":e {}", c3.display()));
    let id_c = ed.focused_buffer_id();
    let buf_count = ed.state.buffers.len();

    // Stand-in for a file-picker jump back to A (both reach the same
    // `switch-to-buffer!` chokepoint): re-opening an already-open path dedups
    // rather than reopening — assert the count is unchanged so a dedup miss
    // fails here instead of silently invalidating the assertions below.
    type_cmd_event(&mut ed, &format!(":e {}", c1.display()));
    assert_eq!(ed.focused_buffer_id(), id_a, "re-open of A must dedup");
    assert_eq!(
        ed.state.buffers.len(),
        buf_count,
        ":e on an already-open path must not duplicate"
    );

    assert_eq!(
        ed.alternate_buffer(),
        Some(id_c),
        "alternate right after focusing A must be C, the buffer just left"
    );

    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    ed.settle();
    assert_eq!(ed.focused_buffer_id(), id_c);

    assert_eq!(
        ed.alternate_buffer(),
        Some(id_a),
        "alternate must follow focus order (A, just left) not open order (B)"
    );

    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    ed.settle();
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        "goto-alternate-buffer must toggle A <-> C, not collapse into B"
    );
}

#[test]
fn alternate_follows_pane_focus_moves_alone() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let (p3, _t3) = temp_file("file3\n");
    let mut ed = editor_from("-[h]>ello\n");

    type_cmd_event(&mut ed, &format!(":e {}", p1.to_str().unwrap()));
    let id_a = ed.focused_buffer_id();
    type_cmd_event(&mut ed, &format!(":e {}", p2.to_str().unwrap()));
    let id_b = ed.focused_buffer_id();
    type_cmd_event(&mut ed, &format!(":e {}", p3.to_str().unwrap()));

    // Two extra panes, each pinned to an older buffer — from here on, moving
    // focus between them (never `:e`, which is already covered above) is the
    // only thing that can reorder A or B.
    let pid_a = crate::editor::commands::open_pane(&mut ed.state, &mut ed.view, id_a);
    let pid_b = crate::editor::commands::open_pane(&mut ed.state, &mut ed.view, id_b);

    // Visit A, then B — both by pane focus alone — then revisit A. If focus
    // moves promote MRU, A's alternate is now B (visited in between); an
    // open-order-only implementation would still say C (never revisited).
    ed.switch_focused_pane(pid_a);
    ed.settle();
    ed.switch_focused_pane(pid_b);
    ed.settle();
    ed.switch_focused_pane(pid_a);
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), id_a);
    assert_eq!(
        ed.alternate_buffer(),
        Some(id_b),
        "pane-focus moves alone must reorder the alternate"
    );
}

#[test]
fn colon_e_percent_is_noop_reload() {
    let (p1, _t1) = temp_file("file1\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();
    let count_before = ed.state.buffers.len();

    type_cmd(&mut ed, ":e %");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        ":e % stays on same buffer"
    );
    assert_eq!(
        ed.state.buffers.len(),
        count_before,
        ":e % does not duplicate"
    );
}
