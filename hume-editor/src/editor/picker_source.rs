//! A picker session's spawned line source: spawning and stopping it, the
//! per-frame drain that builds `PickerItem`s from streamed lines, and
//! reporting its exit.

use std::sync::Arc;

use hume_platform::process::line_source::SpawnedLineSource;
use hume_scripting::host::PickerSourceOpts;
use steel::rvals::SteelVal;

use super::input_stack::PickerItem;
use super::input_stack::picker::{session_for_feed, session_for_token};
use super::{Editor, EditorState};
use hume_core::{HostToken, Severity};

/// Attaches a streaming external-command source to the picker named by
/// `token`. `Ok(false)` for a stale token or no open picker, an expected
/// race; a spawn failure is an `Err`.
///
/// Spawns the new child before touching the old source, so a failed spawn
/// leaves the working source in place. The outgoing source's exit is
/// reported before the attach replaces it.
pub(super) fn spawn_source(
    state: &mut EditorState,
    token: HostToken,
    cmd: &str,
    args: Vec<String>,
    opts: PickerSourceOpts,
) -> Result<bool, String> {
    if session_for_feed(state, token).is_none() {
        return Ok(false);
    }
    let delimiter = if opts.nul { b'\0' } else { b'\n' };
    let launch = hume_platform::process::Launch {
        cwd: &opts.cwd,
        search_path: state.dirs.search_path.as_deref(),
    };
    let source = hume_platform::process::line_source::spawn_line_source(
        cmd,
        &args,
        launch,
        delimiter,
        Arc::clone(&state.wake),
    )
    .map_err(|e| e.to_string())?;
    take_and_report_outgoing_source(state);
    session_for_feed(state, token)
        .expect("checked Some above")
        .attach_source(source, opts.ok_exit_codes);
    Ok(true)
}

/// Detaches the picker's source, if any, reporting its exit if it had
/// already exited, without touching the item list. `false` for a stale token
/// or no open picker.
pub(super) fn stop_source(state: &mut EditorState, token: HostToken) -> bool {
    if session_for_token(state, token).is_none() {
        return false;
    }
    take_and_report_outgoing_source(state);
    true
}

impl Editor {
    /// The reader thread's `WakeCallback` wakes the loop when a batch lands,
    /// so there is no deadline to poll for.
    ///
    /// Coalesces every batch queued since the last frame into one
    /// `PickerSession::source_batch`, which reranks on every call.
    pub(super) fn drain_picker_source(&mut self) {
        let Some(session) = self.state.input.picker_mut() else {
            return;
        };
        let Some(source) = session.source_mut() else {
            return;
        };
        let (lines, disconnected) = source.try_recv_batches();

        // A blank line is unmatchable noise. The splitter transcribes the
        // byte stream as-is, so it is dropped here.
        let items: Vec<PickerItem> = lines
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| PickerItem {
                // A source without `--nul` (`rg --vimgrep --null`) puts a
                // NUL inside a line as a field separator, which would paint
                // as a `<0>` placeholder; `:` is what the tool prints there
                // without that flag. `payload` keeps the raw byte so a
                // plugin can split a path that contains a colon.
                display: line.replace('\0', ":"),
                payload: hume_scripting::Rooted::new(SteelVal::StringV(line.into())),
            })
            .collect();
        session.source_batch(items, disconnected);

        let exit = disconnected.then(|| {
            session
                .take_source()
                .expect("source_mut returned Some above, and disconnect came from the same source")
        });

        if let Some((source, ok_exit_codes)) = exit {
            report_source_exit(&mut self.state, source, &ok_exit_codes);
        }
    }
}

/// Reports a spawned source's exit as a message-log error unless its status
/// code is in `ok_exit_codes`. That list is the complete allowlist, so one
/// without `0` reports a successful exit as a failure.
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
        hume_platform::process::failure_message(&cmd, Some(status), &exit.stderr),
    );
}

/// Takes the picker's attached source out, reporting its exit if it had
/// already exited. A source still running is dropped without a report:
/// `SpawnedLineSource::drop` kills it, and a deliberate kill's status is not
/// a failure. Closing a picker drops its source without coming here, since
/// there is nowhere left to report.
fn take_and_report_outgoing_source(state: &mut EditorState) {
    let Some(session) = state.input.picker_mut() else {
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
