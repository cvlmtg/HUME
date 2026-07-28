//! `picker-source-spawn!`'s Rust-side half: drives
//! `PickerSession::attach_source`/`Editor::drain_picker_source`
//! directly (no Steel involved — see `picker_source_steel.rs` for the
//! end-to-end builtin coverage), so these are unix-only (`sh`/`sleep`).

use super::*;

use crate::editor::picker::{self, PickerSession};
use hume_platform::process::line_source::spawn_line_source;
use std::sync::Arc;
use std::time::{Duration, Instant};
use steel::rvals::SteelVal;

fn open_bare_picker(ed: &mut Editor) {
    let session = PickerSession::new(SteelVal::BoolV(false), String::new());
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
}

fn no_op_wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

/// Drives `drain_async_sources` in a bounded loop until `until` returns
/// true, so CI scheduling jitter can't flake these tests.
fn drain_until(ed: &mut Editor, mut until: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.drain_async_sources();
        if until(ed) {
            return;
        }
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `kill -0` against the real OS as an independent liveness oracle — never
/// asks the handle itself whether it thinks the child is alive.
fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn kill -0")
        .success()
}

#[test]
fn end_to_end_drain_streams_lines_into_the_store() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    let args = vec!["-c".to_string(), "printf 'a\\nb\\nc\\n'".to_string()];
    let source = spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source);

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 3
    });

    let picker = ed.state.config.picker.as_ref().expect("picker still open");
    assert_eq!(picker.window(10).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    assert!(
        !picker.has_source(),
        "source must detach once the reader disconnects"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "a clean exit must not report anything"
    );
}

#[test]
fn coalesced_push_reranks_against_the_live_query() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);
    ed.state.config.picker.as_mut().unwrap().insert_char('z');

    let args = vec!["-c".to_string(), "printf 'abc\\nxyz\\n'".to_string()];
    let source = spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source);

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 2
    });

    let picker = ed.state.config.picker.as_ref().unwrap();
    assert_eq!(
        picker.matched_len(),
        1,
        "only the line matching the live query 'z' should rank"
    );
    assert_eq!(picker.window(10).collect::<Vec<_>>(), vec!["xyz"]);
}

#[test]
fn nonzero_exit_reports_a_status_message_with_stderr() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    let args = vec!["-c".to_string(), "echo boom >&2; exit 2".to_string()];
    let source = spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source);

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed.state.status_msg.as_ref().expect("status set");
    assert!(msg.contains("boom"), "got: {msg}");
    let error_entries = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .count();
    assert_eq!(error_entries, 1);
}

#[test]
fn close_picker_kills_the_source_child() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    let args = vec!["30".to_string()];
    let source = spawn_line_source("sleep", &args, None, b'\n', no_op_wake()).expect("spawn sleep");
    let pid = source.pid();
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source);

    picker::close_picker(&mut ed.state, SteelVal::BoolV(false));

    assert!(
        !process_is_alive(pid),
        "closing the picker must kill its source child"
    );
}

#[test]
fn replacing_the_session_kills_the_previous_source_child() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    let args = vec!["30".to_string()];
    let source = spawn_line_source("sleep", &args, None, b'\n', no_op_wake()).expect("spawn sleep");
    let pid = source.pid();
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source);

    // A fresh `open_picker` call replaces (and — via `close_picker` — drops)
    // whatever session was open, same as a second `picker!` from Steel.
    let replacement = PickerSession::new(SteelVal::BoolV(false), String::new());
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), replacement);

    assert!(
        !process_is_alive(pid),
        "replacing the session must kill the old session's source child"
    );
}

#[test]
fn a_second_attach_source_kills_the_first_source_child() {
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    let first_args = vec!["30".to_string()];
    let first =
        spawn_line_source("sleep", &first_args, None, b'\n', no_op_wake()).expect("spawn sleep");
    let first_pid = first.pid();
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(first);

    let second_args = vec!["30".to_string()];
    let second =
        spawn_line_source("sleep", &second_args, None, b'\n', no_op_wake()).expect("spawn sleep");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(second);

    assert!(
        !process_is_alive(first_pid),
        "attaching a second source must kill the first (re-spawn-replaces-source semantics)"
    );
}
