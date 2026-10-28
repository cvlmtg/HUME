use super::*;

/// Open a second real (file-backed) buffer with content `"world\n"` and
/// return its id — a second file buffer distinct from `editor_with_file`'s.
fn open_second_file_buffer(ed: &mut Editor) -> BufferId {
    let (buf, _tmp_path) = file_buffer("world\n");
    ed.open_buffer(buf)
}

/// Helper: submit a typed command through the minibuffer.
pub(super) fn submit(ed: &mut Editor, cmd: &str) {
    ed.handle_key(key(':'));
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
}

/// Evaluates `source` against a fresh `ScriptingHost` and returns the error
/// it raised.
fn eval_err(ed: &mut Editor, source: &str) -> String {
    let mut host = hume_scripting::ScriptingHost::new(&ed.state.dirs);
    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(source, &mut ih)
    };
    result.expect_err("the source must raise")
}

mod earlier_later;
mod entry_and_editing;
mod goto_and_sort;
mod history_and_names;
mod quit_and_write;
mod typed_refusals;
