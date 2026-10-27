//! `pipe`: running each selection through a shell command. The command
//! starts the run, which lives in `pipe_run`; this lands its output.

use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::edit::{fit_pipe_output, insert_values, replace_each_selection};
use hume_platform::process::batch::Captured;

use super::super::EditorState;
use super::super::doc_ops;
use super::super::error::CommandError;
use super::super::pipe_run::{Launch, PipeEnd, PipeKind};
use super::{CommandPane, ensure_writable};
use hume_core::{Count, Severity};

/// `pipe`: start `command` once per selection of `t`, the selection its
/// input and the output replacing it. Inside an Insert session on `t` each
/// cursor runs it with no input and the output is inserted there. The run
/// goes on past this dispatch: in the background, or waited for before the
/// next key when it inserts or a macro is recording.
pub(in crate::editor) fn cmd_pipe(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
    command: &str,
) -> Result<(), CommandError> {
    if is_blank_command(command) {
        return Err(CommandError::transient("pipe: no command to run"));
    }
    ensure_writable(state, view, t)?;
    let (pid, bid) = (t.pid(), t.bid(view));
    let kind = if state.input.inserting_into(pid, bid) {
        PipeKind::Insert
    } else {
        PipeKind::Replace
    };
    let settles = state.defer_change();
    state.start_pipe_run(pid, bid, command.to_string(), kind, settles);
    Ok(())
}

/// Whether `command` has nothing to run: [`cmd_pipe`] refuses it, and the
/// `|` prompt does not record it in its history.
pub(in crate::editor) fn is_blank_command(command: &str) -> bool {
    command.trim().is_empty()
}

/// `cancel-pipe`: cancel the pipe running in `t`'s buffer, changing
/// nothing. Refused when none is running there.
pub(in crate::editor) fn cmd_cancel_pipe(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if state.cancel_pipe_run(t.bid(view)) {
        Ok(())
    } else {
        Err(CommandError::transient("No pipe is running in this buffer"))
    }
}

/// Land a finished run's `outputs`, one per selection it started from, in
/// one edit through its pane: each fitted to whether its input ended a line,
/// then replacing its selection or inserted at its cursor as the run's kind
/// says. A process's stderr is logged at Trace. [`PipeEnd::Landed`] when the
/// edit applied, output identical to its input included; [`PipeEnd::Failed`],
/// reported, when it did not.
pub(in crate::editor) fn apply_pipe_output(
    state: &mut EditorState,
    launch: Launch,
    outputs: Vec<Captured>,
) -> PipeEnd {
    let Launch {
        buffer,
        pane,
        selections,
        input_ends_line,
        kind,
        ..
    } = launch;
    let mut values = Vec::with_capacity(outputs.len());
    for (&ends_line, Captured { stdout, stderr }) in input_ends_line.iter().zip(outputs) {
        if !stderr.is_empty() {
            state.report(Severity::Trace, format!("pipe: {}", stderr.trim_end()));
        }
        values.push(fit_pipe_output(ends_line, stdout));
    }
    let (buffers, mut stores) = state.edit_parts();
    let edited =
        doc_ops::apply_doc_edit_on(
            buffers,
            &mut stores,
            pane,
            buffer,
            selections,
            |s| match kind {
                PipeKind::Replace => replace_each_selection(s, &values),
                PipeKind::Insert => insert_values(s, &values),
            },
        );
    match edited {
        Ok(()) => PipeEnd::Landed,
        Err(e) => {
            state.report_error(&e);
            PipeEnd::Failed
        }
    }
}
