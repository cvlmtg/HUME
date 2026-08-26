//! Per-frame drain for a session-owned spawned line source. Kept out of
//! `picker.rs` so that module
//! stays a pure store; this is the one place `PickerItem`s get built from
//! streamed lines and the one place a spawned source's exit gets reported.
//! Also owns spawning/stopping a source (`spawn_source`/`stop_source`,
//! `EditorHostImpl`'s delegates for `picker-source-spawn!`/
//! `picker-source-stop!`), guarded by `picker::session_for_token` like every
//! other token-scoped picker mutation — the "report the outgoing source
//! before attaching a new one" ordering rule lives here, beside the
//! exit-reporting it feeds, not in the host-trait translation layer.

use std::sync::Arc;

use hume_platform::process::line_source::SpawnedLineSource;
use hume_scripting::host::PickerSourceOpts;
use steel::rvals::SteelVal;

use super::message_log::Severity;
use super::picker::{PickerItem, session_for_token};
use super::{Editor, EditorState};

/// `EditorHostImpl::picker_source_spawn`'s body: attaches a streaming
/// external-command source to the picker named by `token`. `Ok(false)` — a
/// stale token or no open picker — is the same expected-normal-race
/// contract `picker_push` uses; a genuine spawn failure raises.
///
/// Reports the outgoing source's exit (if it had already exited) *before*
/// attaching the new one — never after, or a source that already failed
/// would be silently dropped by the attach's own replace. Spawns the new
/// child before reaping the old one, though: the `?` below must return
/// before anything is torn down, so a failed re-spawn leaves the working
/// source in place rather than leaving the picker sourceless.
pub(super) fn spawn_source(
    state: &mut EditorState,
    token: u64,
    cmd: &str,
    args: Vec<String>,
    opts: PickerSourceOpts,
) -> Result<bool, String> {
    if session_for_token(state, token).is_none() {
        return Ok(false);
    }
    let delimiter = if opts.nul { b'\0' } else { b'\n' };
    let source = hume_platform::process::line_source::spawn_line_source(
        cmd,
        &args,
        opts.cwd.as_deref(),
        delimiter,
        Arc::clone(&state.wake),
    )
    .map_err(|e| format!("cannot run '{cmd}': {e}"))?;
    take_and_report_outgoing_source(state);
    session_for_token(state, token)
        .expect("checked Some above")
        .attach_source(source, opts.ok_exit_codes);
    Ok(true)
}

/// `EditorHostImpl::picker_source_stop`'s body: detaches (and reports, if
/// already exited) the picker's attached source, if any, without touching
/// the item list. Same expected-normal-race contract as `spawn_source`.
pub(super) fn stop_source(state: &mut EditorState, token: u64) -> bool {
    if session_for_token(state, token).is_none() {
        return false;
    }
    take_and_report_outgoing_source(state);
    true
}

impl Editor {
    /// Arrival-driven, like the parse worker — no `AsyncSource` impl (see
    /// `async_source.rs`'s module doc); the reader thread's `WakeCallback`
    /// wakes the loop directly the moment a batch lands, so there is no
    /// deadline to poll for here.
    ///
    /// Coalesces every batch queued since the last frame into at most ONE
    /// `PickerSession::push` — `push` reranks every call, and reranking once
    /// per queued batch instead of once per frame would multiply the
    /// rerank cost by however many batches arrived this frame.
    pub(super) fn drain_picker_source(&mut self) {
        let Some(session) = self.state.config.picker.as_mut() else {
            return;
        };
        let Some(source) = session.source_mut() else {
            return;
        };
        let (lines, disconnected) = source.try_recv_batches();

        // A blank line is unmatchable noise, not a real candidate — dropped
        // here rather than in the splitter, which stays a faithful,
        // pure transcription of the byte stream (see `line_source.rs`).
        let items: Vec<PickerItem> = lines
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| PickerItem {
                payload: SteelVal::StringV(line.clone().into()),
                display: line,
            })
            .collect();
        if !items.is_empty() {
            session.push(items);
        }

        let exit = disconnected.then(|| {
            session
                .take_source()
                .expect("source_mut returned Some above, and disconnect came from the same source")
        });
        // `session` (a borrow of `self.state.config.picker`) is not used past this
        // point, so `report_source_exit` below can take `&mut self.state` freely.

        if let Some((source, ok_exit_codes)) = exit {
            report_source_exit(&mut self.state, source, &ok_exit_codes);
        }
    }
}

/// Reports a spawned source's exit as a message-log error unless its status
/// code is in `ok_exit_codes` — shared by the natural end-of-stream drain
/// above and a source taken out early by [`take_and_report_outgoing_source`].
/// `ok_exit_codes` is the complete allowlist, not an addition to
/// `ExitStatus::success` — see `UiHost::picker_source_spawn`'s doc for why a
/// list omitting `0` reports a successful exit as a failure.
fn report_source_exit(state: &mut EditorState, source: SpawnedLineSource, ok_exit_codes: &[i32]) {
    let cmd = source.cmd().to_string();
    let exit = source.finish();
    let Some(status) = exit.status else {
        return;
    };
    if status.code().is_some_and(|c| ok_exit_codes.contains(&c)) {
        return;
    }
    state.report(
        Severity::Error,
        format!(
            "{cmd} failed ({}): {}",
            hume_platform::process::exit_code_str(status),
            exit.stderr.trim()
        ),
    );
}

/// Takes the picker's attached source (if any) out and, if it had already
/// exited before being superseded by a respawn or explicitly stopped,
/// reports its exit the same way the natural end-of-stream drain would have.
/// A source still running is dropped silently: `SpawnedLineSource::drop`
/// kills it, and the exit status of a deliberate kill is noise, not a
/// failure worth logging — this is the distinction `has_exited` exists to
/// draw. Shared by `spawn_source` (re-spawn on the same token) and
/// `stop_source`, so neither has to duplicate the "was it actually done?"
/// check. `close_picker` (`picker.rs`) is a third, deliberate path that
/// drops a source without going through here — a picker being closed has
/// nowhere left to report to, so its exit (if any) goes unreported.
fn take_and_report_outgoing_source(state: &mut EditorState) {
    let Some(session) = state.config.picker.as_mut() else {
        return;
    };
    let Some((source, ok_exit_codes)) = session.take_source() else {
        return;
    };
    if !source.has_exited() {
        return;
    }
    report_source_exit(state, source, &ok_exit_codes);
}
