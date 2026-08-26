//! `spawn-async!`'s Rust-side half: drives `EditorHostImpl::spawn_async`/
//! `cancel_async` and `Editor::drain_async_jobs` directly (no Steel
//! involved — see `async_job_steel.rs` for the end-to-end builtin
//! coverage), so these are unix-only (`sh`/`sleep`).

use super::*;

use std::time::{Duration, Instant};

use hume_scripting::host::AsyncProcessHost;
use steel::rvals::SteelVal;

use crate::editor::host_impl::EditorHostImpl;

fn spawn_async(ed: &mut Editor, cmd: &str, args: Vec<String>, callback: SteelVal) -> u64 {
    EditorHostImpl::new(&mut ed.state, &mut ed.view).spawn_async(cmd, args, None, callback)
}

#[test]
fn end_to_end_drain_delivers_the_full_result_exactly_once() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["-c".to_string(), "printf 'hi'".to_string()];
    let id = spawn_async(&mut ed, "sh", args, SteelVal::BoolV(false));

    drain_sources_until(&mut ed, |ed| !ed.state.config.pending_work.is_empty());

    assert!(
        !ed.state.config.async_jobs.contains_key(&id),
        "a completed job must be removed from the registry"
    );
    assert_eq!(pending_calls(&ed).len(), 1);
    let (_, call_args) = pending_calls(&ed)[0];
    assert_eq!(
        call_args,
        &vec![
            SteelVal::StringV("hi".into()),
            SteelVal::StringV("".into()),
            SteelVal::IntV(0),
        ]
    );

    // Draining again must not re-queue the same completion.
    ed.state.config.pending_work.clear();
    ed.drain_async_sources();
    assert!(ed.state.config.pending_work.is_empty());
}

#[test]
fn nonzero_exit_and_stderr_reach_the_callback() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["-c".to_string(), "echo boom >&2; exit 3".to_string()];
    spawn_async(&mut ed, "sh", args, SteelVal::BoolV(false));

    drain_sources_until(&mut ed, |ed| !ed.state.config.pending_work.is_empty());

    let (_, call_args) = pending_calls(&ed)[0];
    assert_eq!(call_args[1], SteelVal::StringV("boom\n".into()));
    assert_eq!(call_args[2], SteelVal::IntV(3));
}

#[test]
fn missing_binary_fires_the_callback_synchronously_with_code_negative_one() {
    let mut ed = editor_from("-[a]>bc\n");
    let id = spawn_async(
        &mut ed,
        "definitely-not-a-real-binary-xyz",
        vec![],
        SteelVal::BoolV(false),
    );

    // No drain needed — a spawn failure fires its callback immediately,
    // inside `spawn_async` itself.
    assert!(!ed.state.config.async_jobs.contains_key(&id));
    assert_eq!(pending_calls(&ed).len(), 1);
    let (_, call_args) = pending_calls(&ed)[0];
    assert_eq!(call_args[0], SteelVal::StringV("".into()));
    assert_eq!(call_args[2], SteelVal::IntV(-1));
    let SteelVal::StringV(stderr) = &call_args[1] else {
        panic!("expected a string stderr arg");
    };
    assert!(
        stderr.contains("definitely-not-a-real-binary-xyz"),
        "got: {stderr:?}"
    );
}

#[test]
fn cancel_kills_the_child_and_drops_the_callback_without_firing_it() {
    // Spawns "sleep" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["30".to_string()];
    let id = spawn_async(&mut ed, "sleep", args, SteelVal::BoolV(false));

    let pid = ed
        .state
        .config
        .async_jobs
        .get(&id)
        .expect("job registered")
        .job
        .pid();

    let started = Instant::now();
    EditorHostImpl::new(&mut ed.state, &mut ed.view).cancel_async(id);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel must not block on the child's remaining lifetime, took {:?}",
        started.elapsed()
    );

    assert!(
        !process_is_alive(pid),
        "cancel-async! must kill its job's child"
    );
    assert!(!ed.state.config.async_jobs.contains_key(&id));

    // Draining after cancel must never fire the dropped callback.
    ed.drain_async_sources();
    assert!(
        ed.state.config.pending_work.is_empty(),
        "a cancelled job's callback must never fire"
    );
}

#[test]
fn cancel_on_an_unknown_id_is_a_silent_no_op() {
    let mut ed = editor_from("-[a]>bc\n");
    EditorHostImpl::new(&mut ed.state, &mut ed.view).cancel_async(999);
    assert!(ed.state.config.pending_work.is_empty());
}
