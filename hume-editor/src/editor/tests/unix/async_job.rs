//! `spawn-async!`'s Rust-side half: drives `EditorHostImpl::spawn_async`/
//! `cancel_async` and `Editor::drain_async_jobs` directly (no Steel
//! involved; see `async_job_steel.rs` for the end-to-end builtin
//! coverage), so these are unix-only (`sh`/`sleep`).

use super::*;

use std::time::{Duration, Instant};

use hume_platform::path::test_support::write_program;
use hume_scripting::host::{AsyncProcessHost, JobHandle};
use steel::rvals::SteelVal;

use crate::editor::async_job::PendingJob;
use crate::editor::host_impl::EditorHostImpl;

/// Whether job `id` has queued its last callback, which has not run yet.
fn awaits_last_callback(ed: &Editor, id: JobHandle) -> bool {
    ed.state
        .config
        .async_jobs
        .get(&id)
        .is_some_and(|pending| matches!(pending, PendingJob::Ended))
}

fn spawn_async(ed: &mut Editor, cmd: &str, args: Vec<String>, callback: SteelVal) -> JobHandle {
    let cwd = ed.state.cwd.clone();
    EditorHostImpl::new(&mut ed.state, &mut ed.view).spawn_async(
        cmd,
        args,
        cwd,
        hume_scripting::Rooted::new(callback),
    )
}

#[test]
fn end_to_end_drain_delivers_the_full_result_exactly_once() {
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["-c".to_string(), "printf 'hi'".to_string()];
    let id = spawn_async(&mut ed, "sh", args, SteelVal::BoolV(false));

    drain_sources_until(&mut ed, |ed| !ed.state.config.pending_work.is_empty());

    assert!(awaits_last_callback(&ed, id));
    assert_eq!(pending_calls(&ed).len(), 1);
    let (_, call_args) = pending_calls(&ed).remove(0);
    assert_eq!(
        call_args,
        vec![
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
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["-c".to_string(), "echo boom >&2; exit 3".to_string()];
    spawn_async(&mut ed, "sh", args, SteelVal::BoolV(false));

    drain_sources_until(&mut ed, |ed| !ed.state.config.pending_work.is_empty());

    let (_, call_args) = pending_calls(&ed).remove(0);
    assert_eq!(call_args[1], SteelVal::StringV("boom\n".into()));
    assert_eq!(call_args[2], SteelVal::IntV(3));
}

#[test]
fn a_child_killed_by_a_signal_has_no_exit_code() {
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["-c".to_string(), "kill -9 $$".to_string()];
    spawn_async(&mut ed, "sh", args, SteelVal::BoolV(false));

    drain_sources_until(&mut ed, |ed| !ed.state.config.pending_work.is_empty());

    let (_, call_args) = pending_calls(&ed).remove(0);
    assert_eq!(call_args[2], SteelVal::BoolV(false));
}

#[test]
fn missing_binary_fires_the_callback_synchronously_with_no_exit_code() {
    let mut ed = editor_from("-[a]>bc\n");
    let id = spawn_async(
        &mut ed,
        "definitely-not-a-real-binary-xyz",
        vec![],
        SteelVal::BoolV(false),
    );

    // No drain needed: a spawn failure queues its callback immediately,
    // inside `spawn_async` itself.
    assert!(awaits_last_callback(&ed, id));
    assert_eq!(pending_calls(&ed).len(), 1);
    let (_, call_args) = pending_calls(&ed).remove(0);
    assert_eq!(call_args[0], SteelVal::StringV("".into()));
    assert_eq!(call_args[2], SteelVal::BoolV(false));
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
    let mut ed = editor_from("-[a]>bc\n");
    let args = vec!["30".to_string()];
    let id = spawn_async(&mut ed, "sleep", args, SteelVal::BoolV(false));

    let pid = ed
        .state
        .config
        .async_jobs
        .get(&id)
        .expect("job registered")
        .process()
        .pid();

    let started = Instant::now();
    EditorHostImpl::new(&mut ed.state, &mut ed.view).cancel_async(id);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel must not block on the child's remaining lifetime, took {:?}",
        started.elapsed()
    );

    assert!(!is_alive(pid), "cancel-async! must kill its job's child");
    assert!(!ed.state.config.async_jobs.contains_key(&id));

    // Draining after cancel must never fire the dropped callback.
    ed.drain_async_sources();
    assert!(
        ed.state.config.pending_work.is_empty(),
        "a cancelled job's callback must never fire"
    );
}

#[test]
fn cancel_on_a_handle_no_job_holds_is_a_silent_no_op() {
    let mut ed = editor_from("-[a]>bc\n");
    EditorHostImpl::new(&mut ed.state, &mut ed.view).cancel_async(JobHandle::from_raw(999));
    assert!(ed.state.config.pending_work.is_empty());
}

// ── Program scans ────────────────────────────────────────────────────────────

fn scan_programs(ed: &mut Editor, prefix: &str) -> JobHandle {
    EditorHostImpl::new(&mut ed.state, &mut ed.view)
        .scan_programs(
            prefix.to_owned(),
            Vec::new(),
            hume_scripting::Rooted::new(SteelVal::BoolV(false)),
        )
        .expect("scan started")
}

/// The `(names done?)` of every queued callback, in order.
fn scan_reports(ed: &Editor) -> Vec<(Vec<String>, bool)> {
    pending_calls(ed)
        .into_iter()
        .map(|(_, args)| match args.as_slice() {
            [SteelVal::ListV(names), SteelVal::BoolV(done)] => (
                names
                    .iter()
                    .map(|name| match name {
                        SteelVal::StringV(s) => s.to_string(),
                        other => panic!("not a name: {other:?}"),
                    })
                    .collect(),
                *done,
            ),
            other => panic!("not a scan report: {other:?}"),
        })
        .collect()
}

#[test]
fn a_program_scan_reports_until_done_then_leaves_the_registry() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write_program(first.path(), "hume-scan-zeta", 0o755);
    write_program(second.path(), "hume-scan-alpha", 0o755);
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.dirs.search_path = Some(std::env::join_paths([first.path(), second.path()]).unwrap());
    let id = scan_programs(&mut ed, "hume-scan");

    drain_sources_until(&mut ed, |ed| {
        scan_reports(ed).last().is_some_and(|(_, done)| *done)
    });

    assert!(awaits_last_callback(&ed, id));
    let reports = scan_reports(&ed);
    let (last, earlier) = reports.split_last().expect("a report");
    assert_eq!(last.0, ["hume-scan-alpha", "hume-scan-zeta"]);
    assert!(earlier.iter().all(|(_, done)| !done));

    ed.drain_pending_work();
    assert!(!ed.state.config.async_jobs.contains_key(&id));
}

#[test]
fn a_last_callback_lost_to_the_drain_cap_frees_its_job() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.dirs.search_path = None;
    let id = scan_programs(&mut ed, "");
    drain_sources_until(&mut ed, |ed| awaits_last_callback(ed, id));

    // Past `MAX_EVENT_DRAIN` (1000, `event/pump.rs`), so the whole batch,
    // the scan's last callback included, is dropped unrun.
    for _ in 0..1100 {
        ed.state
            .config
            .pending_work
            .push_back(crate::editor::event::PendingWork::Event(
                crate::editor::event::EditorEvent::OnFocusGained,
            ));
    }
    assert!(!ed.drain_pending_work());

    assert!(!ed.state.config.async_jobs.contains_key(&id));
}

#[test]
fn cancel_async_stops_a_program_scan() {
    let bin = tempfile::tempdir().unwrap();
    write_program(bin.path(), "hume-scan-probe", 0o755);
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.dirs.search_path = Some(bin.path().into());
    let id = scan_programs(&mut ed, "hume-scan");

    EditorHostImpl::new(&mut ed.state, &mut ed.view).cancel_async(id);
    ed.drain_async_sources();

    assert!(!ed.state.config.async_jobs.contains_key(&id));
    assert!(pending_calls(&ed).is_empty());
}
