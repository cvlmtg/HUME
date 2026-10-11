//! Reading the operand a native command was dispatched without from the
//! user, then running the command with it: the character from the next key,
//! or the pattern typed at the command's prompt.

use hume_engine::pipeline::EngineView;
use hume_scripting::PaneHandle;
use hume_scripting::host::CommandArg;

use super::super::dispatch::CmdCtx;
use super::super::input_stack::read_command_char;
use super::super::{EditorState, Severity};
use super::{
    BoundCommand, Missing, MissingOperand, PendingDispatch, open_pattern_prompt, report_arg_error,
    run,
};

/// Read `missing` from the user, then run its command on `pane` as
/// dispatched with `ctx`. Starting the read commits the open paste session
/// as the command's keypress would, even if the read is then cancelled.
/// Returns `false` when the read was refused, which is reported.
pub(in crate::editor) fn collect_operand(
    state: &mut EditorState,
    view: &mut EngineView,
    missing: Missing,
    pane: PaneHandle,
    ctx: CmdCtx,
) -> bool {
    let Missing {
        name,
        commits_paste,
        opener,
        operand,
    } = missing;
    if commits_paste {
        state.commit_paste_session();
    }
    let pending = PendingDispatch { name, pane, ctx };
    let opened = match operand {
        MissingOperand::Char => read_command_char(state, view, pending, opener),
        MissingOperand::Pattern { prompt, fp } => {
            open_pattern_prompt(state, view, fp, prompt, pending)
        }
    };
    match opened {
        Ok(()) => true,
        Err(msg) => {
            state.report(Severity::Error, msg);
            false
        }
    }
}

/// Run `pending` with the operand the user supplied for it, reporting any
/// refusal: by then no script caller is waiting to raise to. Returns
/// `false` when the command is gone, the operand does not fit, or the
/// command refused.
pub(in crate::editor) fn run_collected(
    state: &mut EditorState,
    view: &mut EngineView,
    pending: PendingDispatch,
    arg: CommandArg,
) -> bool {
    let PendingDispatch { name, pane, ctx } = pending;
    let Some(cmd) = state.config.registry.get_mappable(&name).cloned() else {
        state.report(Severity::Error, format!("unknown command: {name}"));
        return false;
    };
    match BoundCommand::resolve(state, view, cmd, pane, Some(arg)) {
        Ok(bound) => run(state, view, bound, ctx),
        Err(e) => {
            report_arg_error(state, &name, &e);
            false
        }
    }
}
