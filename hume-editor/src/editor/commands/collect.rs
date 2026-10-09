//! Reading the operand a native command was dispatched without from the
//! user, then running the command with it: the character from the next key,
//! or the pattern typed at the command's prompt.

use hume_engine::pipeline::EngineView;
use hume_scripting::PaneHandle;
use hume_scripting::host::CommandArg;

use super::super::dispatch::CmdCtx;
use super::super::input_stack::{CharThen, read_char};
use super::super::{EditorState, Severity};
use super::{BoundCommand, Missing, MissingOperand, PendingDispatch, open_pattern_prompt, run};

/// Read `missing` from the user, then run its command on `pane` as
/// dispatched with `ctx`. Starting the read commits the open paste session
/// as the command's keypress would, even if the read is then cancelled.
pub(in crate::editor) fn collect_operand(
    state: &mut EditorState,
    view: &mut EngineView,
    missing: Missing,
    pane: PaneHandle,
    ctx: CmdCtx,
) {
    let Missing {
        name,
        commits_paste,
        operand,
    } = missing;
    if commits_paste {
        state.commit_paste_session();
    }
    match operand {
        MissingOperand::Char => {
            let then = CharThen::Dispatch(PendingDispatch { name, pane, ctx });
            read_char(state, view, then);
        }
        MissingOperand::Pattern { prompt, fp } => {
            open_pattern_prompt(state, view, fp, prompt, PendingDispatch { name, pane, ctx });
        }
    }
}

/// Run `pending` with the operand the user supplied for it, reporting any
/// refusal: by then no script caller is waiting to raise to.
pub(in crate::editor) fn run_collected(
    state: &mut EditorState,
    view: &mut EngineView,
    pending: PendingDispatch,
    arg: CommandArg,
) {
    let PendingDispatch { name, pane, ctx } = pending;
    let Some(cmd) = state.config.registry.get_mappable(&name).cloned() else {
        state.report(Severity::Error, format!("unknown command: {name}"));
        return;
    };
    match BoundCommand::resolve(state, view, cmd, pane, Some(arg)) {
        Ok(bound) => {
            run(state, view, bound, ctx);
        }
        Err(e) => state.report(Severity::Error, format!("'{name}': {e}")),
    }
}
