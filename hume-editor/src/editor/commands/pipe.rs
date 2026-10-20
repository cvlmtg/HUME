//! `pipe`: running each selection through a shell command. The command
//! queues the run, which lives in `pipe_run`; this lands its output.

use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::edit::{fit_pipe_output, insert_values, replace_each_selection};

use super::super::error::CommandError;
use super::super::pipe_run::{PipeEnd, PipeKind, QueuedPipe};
use super::super::registry::Count;
use super::super::shell_batch::Captured;
use super::super::{EditorState, Severity};
use super::{CommandPane, apply_pane_edit, ensure_writable};

/// `pipe`: queue `command` to run once per selection of `t`, the selection
/// its input and the output replacing it. Inside an Insert session on `t`
/// each cursor runs it with no input and the output is inserted there. The
/// main loop, or a replay before its next key, runs what this queues.
pub(in crate::editor) fn cmd_pipe(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
    command: &str,
) -> Result<(), CommandError> {
    if command.trim().is_empty() {
        return Err(CommandError::transient("pipe: no command to run"));
    }
    ensure_writable(state, view, t)?;
    debug_assert!(
        state.queued_pipe.is_none(),
        "a queued pipe runs before the next key"
    );
    let kind = if state.input.inserting_into(t.pid(), t.bid(view)) {
        PipeKind::Insert
    } else {
        PipeKind::Replace
    };
    state.queued_pipe = Some(QueuedPipe {
        pane: t.handle(view),
        command: command.to_string(),
        kind,
        settles: state.defer_change(),
    });
    Ok(())
}

/// Land a finished run's `outputs`, one per selection, on `t` in one edit:
/// each fitted to whether its input ended a line (`input_ends_line`), then
/// replacing its selection or inserted at its cursor as `kind` says. A
/// process's stderr is logged at Trace. [`PipeEnd::Landed`] when the edit
/// applied, output identical to its input included; [`PipeEnd::Failed`],
/// reported, when it did not.
pub(in crate::editor) fn apply_pipe_output(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    kind: PipeKind,
    input_ends_line: &[bool],
    outputs: Vec<Captured>,
) -> PipeEnd {
    let mut values = Vec::with_capacity(outputs.len());
    for (&ends_line, Captured { stdout, stderr }) in input_ends_line.iter().zip(outputs) {
        if !stderr.is_empty() {
            state.report(Severity::Trace, format!("pipe: {}", stderr.trim_end()));
        }
        values.push(fit_pipe_output(ends_line, stdout));
    }
    let edited = apply_pane_edit(state, view, t, |s| match kind {
        PipeKind::Replace => replace_each_selection(s, &values),
        PipeKind::Insert => insert_values(s, &values),
    });
    match edited {
        Ok(()) => PipeEnd::Landed,
        Err(e) => {
            state.report_error(&e);
            PipeEnd::Failed
        }
    }
}
