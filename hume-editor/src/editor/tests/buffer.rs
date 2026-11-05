use super::*;
use pretty_assertions::assert_eq;

// ── :b with no argument ───────────────────────────────────────────────────────

#[test]
fn buffer_no_arg_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", None).unwrap_err();
    assert_eq!(err.message(), "Usage: :buffer <name|#|index>");
}

// ── :b by 1-based index ───────────────────────────────────────────────────────

#[test]
fn buffer_index_zero_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("0")).unwrap_err();
    assert!(
        err.to_string().contains("index"),
        "must mention 'index', got: {err}"
    );
}

#[test]
fn buffer_index_out_of_range_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("99")).unwrap_err();
    assert!(
        err.to_string().contains("99"),
        "must mention the bad index, got: {err}"
    );
}

// ── :b by full absolute path ──────────────────────────────────────────────────

// ── :b by exact basename ──────────────────────────────────────────────────────

// ── :b by basename prefix ─────────────────────────────────────────────────────

#[test]
fn buffer_no_match_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("b", Some("definitely_not_a_buffer_xyz"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "must say 'no buffer matching', got: {err}"
    );
}

// ── :b scratch buffer ─────────────────────────────────────────────────────────

// ── :b current buffer is a no-op ─────────────────────────────────────────────

#[test]
fn buffer_current_buffer_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    // The only buffer is the scratch buffer; :b *scratch* should be a no-op.
    let before_id = ed.focused_buffer_id();
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        before_id,
        ":b to current buffer must not change focus"
    );
}

// ── :b and :buffer aliases ────────────────────────────────────────────────────

#[test]
fn buffer_long_alias_accepted() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("buffer", Some("xyz_no_such_buf"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "canonical name 'buffer' must work too, got: {err}"
    );
}

#[test]
fn buffer_bang_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "other\n").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    let before_id = ed.focused_buffer_id();
    let err = ed.execute_typed("b!", Some("*scratch*")).unwrap_err();
    assert_eq!(err.message(), "`:buffer` takes no `!`");
    assert_eq!(ed.focused_buffer_id(), before_id);
}

// ── :b on a buffer whose backing file has been deleted ───────────────────────

// ── Ctrl-o restores position after :b ────────────────────────────────────────

// ── :new ─────────────────────────────────────────────────────────────────────

fn display_name(ed: &Editor, bid: BufferId) -> String {
    ed.state.buffers.get(bid).display_name()
}

#[test]
fn new_opens_an_empty_scratch_buffer_in_the_focused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    let first = ed.focused_buffer_id();

    ed.execute_typed("new", None).unwrap();
    let second = ed.focused_buffer_id();

    assert_ne!(second, first);
    assert_eq!(display_name(&ed, first), "*scratch*");
    assert_eq!(display_name(&ed, second), "*scratch-2*");
    assert_eq!(ed.state.buffers.get(second).text().to_string(), "\n");
    ed.execute_typed("b", Some("#")).unwrap();
    assert_eq!(ed.focused_buffer_id(), first);
}

#[test]
fn n_is_an_alias_of_new() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("n", None).unwrap();
    assert_eq!(display_name(&ed, ed.focused_buffer_id()), "*scratch-2*");
}

#[test]
fn scratch_numbers_are_never_reused() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("new", None).unwrap();
    ed.execute_typed("new", None).unwrap();
    ed.execute_typed("b", Some("*scratch-2*")).unwrap();
    ed.execute_typed("bd", None).unwrap();

    ed.execute_typed("new", None).unwrap();

    let names: Vec<String> = ed
        .state
        .buffers
        .iter()
        .map(|(_, buf)| buf.display_name())
        .collect();
    assert_eq!(names, ["*scratch*", "*scratch-3*", "*scratch-4*"]);
}

#[test]
fn buffer_new_opens_the_next_scratch_buffer_and_records_a_jump() {
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[h]>ello\n");
    let first = ed.focused_buffer_id();

    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("buffer-new", pane, NativeCall::default())
        .expect("buffer-new must not error");
    assert_eq!(display_name(&ed, ed.focused_buffer_id()), "*scratch-2*");

    ed.handle_key(key_ctrl('o'));
    assert_eq!(ed.focused_buffer_id(), first);
}

#[test]
fn new_records_a_jump_back_to_the_buffer_it_left() {
    let mut ed = editor_from("-[h]>ello\n");
    let first = ed.focused_buffer_id();
    ed.execute_typed("new", None).unwrap();
    assert_ne!(ed.focused_buffer_id(), first, "setup: :new switched away");

    ed.handle_key(key_ctrl('o'));

    assert_eq!(ed.focused_buffer_id(), first);
}

#[test]
fn closing_the_last_buffer_opens_the_first_scratch_when_none_was_made() {
    let mut ed = editor_with_path("a\n", std::path::Path::new("/tmp/only.txt"));
    ed.execute_typed("bd", None).unwrap();
    assert_eq!(display_name(&ed, ed.focused_buffer_id()), "*scratch*");
}

#[test]
fn closing_the_last_buffer_opens_the_next_scratch_number() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("file.txt");
    std::fs::write(&file, "file\n").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    ed.execute_typed("bd", None).unwrap();

    ed.execute_typed("bd", None).unwrap();

    assert_eq!(display_name(&ed, ed.focused_buffer_id()), "*scratch-2*");
}

#[test]
fn b_switches_to_a_numbered_scratch_buffer_by_name() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("new", None).unwrap();
    let second = ed.focused_buffer_id();
    ed.execute_typed("new", None).unwrap();

    ed.execute_typed("b", Some("*scratch-2*")).unwrap();

    assert_eq!(ed.focused_buffer_id(), second);
}

#[test]
fn an_ambiguous_scratch_prefix_names_each_scratch_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("new", None).unwrap();

    let err = ed.execute_typed("b", Some("*scr")).unwrap_err();

    let message = err.message();
    assert!(message.contains("*scratch*,"), "{message}");
    assert!(message.contains("*scratch-2*"), "{message}");
}
