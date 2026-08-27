//! `picker-source-spawn!`'s Rust-side half: drives
//! `PickerSession::attach_source`/`Editor::drain_picker_source`
//! directly (no Steel involved — see `picker_source_steel.rs` for the
//! end-to-end builtin coverage), so these are unix-only (`sh`/`sleep`).

use super::*;

use crate::editor::picker::{self, PickerSession, item};
use hume_platform::process::line_source::spawn_line_source;
use hume_scripting::host::{LivePickerOpts, PickerOpts, TruncateEnd};
use std::sync::Arc;
use steel::rvals::SteelVal;

fn open_bare_picker(ed: &mut Editor) {
    let session = PickerSession::new(SteelVal::BoolV(false), PickerOpts::default());
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
}

fn open_live_picker(ed: &mut Editor) {
    let session = PickerSession::new_live(
        SteelVal::BoolV(false),
        LivePickerOpts {
            prompt: String::new(),
            query: String::new(),
            on_query_change: SteelVal::BoolV(false),
            truncate: TruncateEnd::Head,
        },
    );
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
}

fn no_op_wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

/// Spawns `sh -c script` and attaches it to the already-open picker with
/// `ok_exit_codes` — the spawn-and-attach block every `sh`-based test below
/// needs before it can exercise `drain_picker_source`.
fn attach_sh(ed: &mut Editor, script: &str, ok_exit_codes: Vec<i32>) {
    let args = vec!["-c".to_string(), script.to_string()];
    let source = spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(source, ok_exit_codes);
}

#[test]
fn end_to_end_drain_streams_lines_into_the_store() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    attach_sh(&mut ed, "printf 'a\\nb\\nc\\n'", vec![0]);

    drain_sources_until(&mut ed, |ed| {
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
fn a_nul_inside_a_line_shows_as_a_colon_but_the_payload_keeps_it() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    attach_sh(&mut ed, "printf 'a\\000b\\nc\\n'", vec![0]);

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 2
    });

    let picker = ed.state.config.picker.as_ref().expect("picker still open");
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec!["a:b", "c"],
        "a NUL in the display must read as ':', the same field separator the tool prints without --null"
    );
    assert_eq!(
        picker.selected_payload(),
        Some(&SteelVal::StringV("a\0b".into())),
        "the payload must keep the raw NUL so a plugin's own split stays unambiguous"
    );
}

#[test]
fn coalesced_push_reranks_against_the_live_query() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);
    let _ = ed.state.config.picker.as_mut().unwrap().insert_char('z');

    attach_sh(&mut ed, "printf 'abc\\nxyz\\n'", vec![0]);

    drain_sources_until(&mut ed, |ed| {
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
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    attach_sh(&mut ed, "echo boom >&2; exit 2", vec![0]);

    drain_sources_until(&mut ed, |ed| ed.state.status_msg.is_some());

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
fn exit_code_in_the_allowlist_reports_nothing() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    // Exit 1 with nothing on stderr — the shape `rg` uses for "no matches".
    attach_sh(&mut ed, "exit 1", vec![0, 1]);

    drain_sources_until(&mut ed, source_detached);

    assert!(
        ed.state.status_msg.is_none(),
        "exit 1 is in the allowlist — must not report anything"
    );
    let error_entries = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .count();
    assert_eq!(error_entries, 0);
}

#[test]
fn exit_code_outside_the_allowlist_still_reports() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    // Exit 2 — `rg`'s "bad regex" — must still surface even with `1`
    // allowlisted for "no matches".
    attach_sh(&mut ed, "echo boom >&2; exit 2", vec![0, 1]);

    drain_sources_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed.state.status_msg.as_ref().expect("status set");
    assert!(msg.contains("boom"), "got: {msg}");
}

#[test]
fn allowlist_omitting_zero_reports_a_successful_exit() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);

    // `#:ok-exit-codes` is the complete allowlist, not an addition to
    // `ExitStatus::success` (see `UiHost::picker_source_spawn`'s doc) — a
    // list that omits `0` must report even a clean exit. Pinned here as a
    // characterization test so this reads as the documented contract, not
    // as a bug to "fix" later.
    attach_sh(&mut ed, "exit 0", vec![1]);

    drain_sources_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed.state.status_msg.as_ref().expect("status set");
    assert!(
        msg.contains("failed"),
        "an allowlist omitting 0 must report a successful exit, got: {msg}"
    );
}

#[test]
fn close_picker_kills_the_source_child() {
    // Spawns "sleep" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
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
        .attach_source(source, vec![0]);

    picker::close_picker(&mut ed.state, SteelVal::BoolV(false));

    assert!(
        !process_is_alive(pid),
        "closing the picker must kill its source child"
    );
}

#[test]
fn replacing_the_session_kills_the_previous_source_child() {
    // Spawns "sleep" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
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
        .attach_source(source, vec![0]);

    // A fresh `open_picker` call replaces (and — via `close_picker` — drops)
    // whatever session was open, same as a second `picker!` from Steel.
    let replacement = PickerSession::new(SteelVal::BoolV(false), PickerOpts::default());
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), replacement);

    assert!(
        !process_is_alive(pid),
        "replacing the session must kill the old session's source child"
    );
}

#[test]
fn a_second_attach_source_kills_the_first_source_child() {
    // Spawns "sleep" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
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
        .attach_source(first, vec![0]);

    let second_args = vec!["30".to_string()];
    let second =
        spawn_line_source("sleep", &second_args, None, b'\n', no_op_wake()).expect("spawn sleep");
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .attach_source(second, vec![0]);

    assert!(
        !process_is_alive(first_pid),
        "attaching a second source must kill the first (re-spawn-replaces-source semantics)"
    );
}

// ── `attach_source`'s `supersedes_rows`: a live session's own requery
// ── swaps its first batch in instead of appending — see `PickerSession::push`
// ── and `AttachedSource::supersedes_rows`'s docs in `picker.rs`.

#[test]
fn live_session_first_batch_after_attach_replaces_the_previous_rows() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_live_picker(&mut ed);
    {
        let session = ed.state.config.picker.as_mut().unwrap();
        session.push(vec![item("stale-a"), item("stale-b")]);
        session.move_selection(1, 10);
    }
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 1);

    attach_sh(&mut ed, "printf 'fresh\\n'", vec![0]);

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 1
    });

    let picker = ed.state.config.picker.as_ref().unwrap();
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec!["fresh"],
        "a live session's newly attached source must replace the previous \
         query's rows wholesale on its first batch, not append to them"
    );
    assert_eq!(
        picker.selected(),
        0,
        "the wholesale swap must reset the cursor"
    );
}

#[test]
fn live_session_second_batch_from_the_same_source_appends() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_live_picker(&mut ed);

    attach_sh(
        &mut ed,
        "printf 'first\\n'; sleep 0.3; printf 'second\\n'",
        vec![0],
    );

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            >= 1
    });
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["first"],
        "the first batch after attach must replace (wholesale, not append)"
    );

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 2
    });
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["first", "second"],
        "a later batch from the SAME source must append, not replace again"
    );
}

#[test]
fn filter_session_attach_source_still_appends_not_replaces() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_bare_picker(&mut ed);
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .push(vec![item("seeded")]);

    attach_sh(&mut ed, "printf 'fresh\\n'", vec![0]);

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 2
    });
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["seeded", "fresh"],
        "a non-live (`picker!`) session's attached source must still append \
         to whatever was already seeded, not replace it — `supersedes_rows` \
         is only set for a live session"
    );
}

#[test]
fn explicit_replace_while_a_source_is_attached_consumes_supersede_so_the_next_batch_appends() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    open_live_picker(&mut ed);

    attach_sh(&mut ed, "sleep 0.3; printf 'streamed\\n'", vec![0]);
    // Nothing has arrived yet (the child is still sleeping) — an explicit
    // replace must consume the still-armed `supersedes_rows` flag itself,
    // so the source's own later batch doesn't wholesale-replace this list
    // right back out.
    ed.state
        .config
        .picker
        .as_mut()
        .unwrap()
        .replace(vec![item("explicit")]);

    drain_sources_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == 2
    });
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["explicit", "streamed"],
        "the source's own batch must append to what `replace` just set, not \
         wholesale-replace it again"
    );
}
