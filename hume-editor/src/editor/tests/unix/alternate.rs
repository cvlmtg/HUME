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
fn goto_alternate_file_switches_to_alternate_and_is_involutive() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    live_host!(ed)
        .run_command_sync("goto-alternate-file", Some(1), false, None)
        .expect("goto-alternate-file must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        "goto-alternate-file must switch to alternate"
    );

    live_host!(ed)
        .run_command_sync("goto-alternate-file", Some(1), false, None)
        .expect("goto-alternate-file must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_b,
        "goto-alternate-file again returns to starting buffer"
    );
}

#[test]
fn goto_alternate_file_pushes_jump_entry() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();

    live_host!(ed)
        .run_command_sync("goto-alternate-file", Some(1), false, None)
        .expect("goto-alternate-file must not error");
    assert_ne!(
        ed.focused_buffer_id(),
        id_before,
        "goto-alternate-file changes focus"
    );
    live_host!(ed)
        .run_command_sync("jump-backward", Some(1), false, None)
        .expect("jump-backward must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "jump-backward retraces goto-alternate-file"
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
