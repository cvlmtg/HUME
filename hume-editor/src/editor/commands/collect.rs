//! Reading the operand a native command was dispatched without from the
//! user, then running the command with it: the character from the next key,
//! or the text typed at the command's prompt.

use hume_engine::pipeline::EngineView;
use hume_scripting::PaneHandle;
use hume_scripting::host::CommandArg;

use super::super::dispatch::CmdCtx;
use super::super::input_stack::{open_shell_prompt, read_command_char};
use super::super::registry::TextPrompt;
use super::super::replay::{Answer, AnswerTicket, InputOpener, PendingAnswer};
use super::super::{Editor, EditorState, Severity};
use super::search::{all_cursors, open_search_prompt, open_sift_prompt};
use super::{
    BoundCommand, FocusedPane, Missing, MissingOperand, PendingDispatch, report_arg_error, run,
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
        cmd,
        commits_paste,
        opener,
        operand,
    } = missing;
    if commits_paste {
        state.commit_paste_session();
    }
    let pending = PendingDispatch { cmd, pane, ctx };
    let opened = match operand {
        MissingOperand::Char => read_command_char(state, view, pending, opener),
        MissingOperand::Text { prompt, fp } => {
            open_text_prompt(state, view, fp, prompt, pending, opener)
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

/// Open `prompt` in `fp` to read the text `then` was dispatched without.
/// Selections that are all bare cursors have nothing to sift, so the sift
/// prompt doesn't open over them. The shell prompt is an overlay, so it
/// refuses to open while another minibuffer is open under it.
///
/// The prompt passes `EditorState::waiting_input` with `then`'s `opener` as
/// it opens. During `.` it opens nothing and takes the recorded text
/// instead, run after the re-run body; with none recorded it refuses with
/// the returned message.
fn open_text_prompt(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    prompt: TextPrompt,
    then: PendingDispatch,
    opener: InputOpener,
) -> Result<(), String> {
    match prompt {
        TextPrompt::Sift if all_cursors(state, view, fp.pane()) => return Ok(()),
        TextPrompt::Shell if state.input.minibuf().is_some() => {
            return Err(format!("{}: a minibuffer is already open", then.cmd.name()));
        }
        _ => {}
    }
    if let Some(replay) = state.dot_replay.as_mut() {
        let what = then.cmd.name().clone();
        return replay.answer_call(
            &what,
            |a| match a {
                Answer::Prompt(text) => Some(text.clone()),
                _ => None,
            },
            |text| Ok(PendingAnswer::Text(then, text)),
        );
    }
    let input = state.waiting_input(then.cmd.name(), opener)?;
    match prompt {
        TextPrompt::Search(direction) => {
            open_search_prompt(state, view, fp, direction, then, input);
        }
        TextPrompt::Sift => open_sift_prompt(state, view, fp, then, input),
        TextPrompt::Shell => open_shell_prompt(state, view, then, input),
    }
    Ok(())
}

/// End a text prompt `then` opened for its operand with the `text` it was
/// confirmed with, `None` when it was cancelled: record that as the
/// prompt's answer under `ticket`, if there is one, then run `then` with the
/// text in the prompt's scope. Every text prompt ends here, live or during
/// `.`. Returns whether the command ran.
pub(in crate::editor) fn finish_text_operand(
    ed: &mut Editor,
    then: PendingDispatch,
    ticket: Option<AnswerTicket>,
    text: Option<String>,
) -> bool {
    if let Some(ticket) = &ticket {
        ed.state.record_answer(ticket, Answer::Prompt(text.clone()));
    }
    let Some(text) = text else {
        return false;
    };
    ed.run_answered(ticket, |ed| {
        run_collected(&mut ed.state, &mut ed.view, then, CommandArg::Text(text))
    })
}

/// Run `pending` with the operand the user supplied for it, reporting any
/// refusal: by then no script caller is waiting to raise to. Returns
/// `false` when the operand does not fit or the command refused.
pub(in crate::editor) fn run_collected(
    state: &mut EditorState,
    view: &mut EngineView,
    pending: PendingDispatch,
    arg: CommandArg,
) -> bool {
    let PendingDispatch { cmd, pane, ctx } = pending;
    let name = cmd.name().clone();
    match BoundCommand::resolve(state, view, cmd, pane, Some(arg)) {
        Ok(bound) => run(state, view, bound, ctx),
        Err(e) => {
            report_arg_error(state, &name, &e);
            false
        }
    }
}
