//! Per-frame drain for a session-owned spawned line source. Kept out of
//! `picker.rs` so that module
//! stays a pure store; this is the one place `PickerItem`s get built from
//! streamed lines and the one place a spawned source's exit gets reported.

use hume_platform::process::line_source::SourceExit;
use steel::rvals::SteelVal;

use super::message_log::Severity;
use super::picker::PickerItem;
use super::{Editor, EditorState};

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
            let token = session.token();
            session.push(token, items);
        }

        let exit = disconnected.then(|| {
            let (source, ok_exit_codes) = session
                .take_source()
                .expect("source_mut returned Some above, and disconnect came from the same source");
            let cmd = source.cmd().to_string();
            (cmd, source.finish(), ok_exit_codes)
        });
        // `session` (a borrow of `self.state.config.picker`) is not used past this
        // point, so `report_source_exit` below can take `&mut self.state` freely.

        if let Some((cmd, exit, ok_exit_codes)) = exit {
            report_source_exit(&mut self.state, &cmd, exit, &ok_exit_codes);
        }
    }
}

/// Reports a spawned source's exit as a message-log error unless its status
/// code is in `ok_exit_codes` — shared by the natural end-of-stream drain
/// above and a source taken out early by
/// [`take_and_report_outgoing_source`], so an exit is reported exactly once
/// no matter which path notices it.
fn report_source_exit(state: &mut EditorState, cmd: &str, exit: SourceExit, ok_exit_codes: &[i32]) {
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
/// draw. Shared by `EditorHostImpl::picker_source_spawn` (re-spawn on the
/// same token) and `picker_source_stop` (`picker-source-stop!`), so neither
/// has to duplicate the "was it actually done?" check.
pub(super) fn take_and_report_outgoing_source(state: &mut EditorState) {
    let Some(session) = state.config.picker.as_mut() else {
        return;
    };
    let Some((source, ok_exit_codes)) = session.take_source() else {
        return;
    };
    if !source.has_exited() {
        return;
    }
    let cmd = source.cmd().to_string();
    report_source_exit(state, &cmd, source.finish(), &ok_exit_codes);
}
