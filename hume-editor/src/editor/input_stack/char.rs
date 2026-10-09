//! The `Char` layer: the next key typed is read as a character for whatever
//! asked for one: a native command a key dispatched without its character
//! (`f`, `r`), a Steel `(on-next-char! …)` callback, or
//! the register name after `q`, `Q` or `"`. Reads made while one is pending
//! take the following keys in the order they were made. Enter and Tab read
//! as `'\n'` and `'\t'`. A register name must also be held with no modifier
//! but Shift and name a valid register. The scroll wheel passes through and
//! leaves the read pending. Any other key, a click or a paste ends the read
//! and is swallowed.
//!
//! During `.` a command's or a callback's read waits for no key: it takes
//! the next answer the replayed command recorded, and refuses when none is
//! left (see `replay::DotReplay`).

use std::collections::VecDeque;

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::register::{MACRO_REGISTER, is_valid_macro_register, is_valid_register_name};
use hume_scripting::host::CommandArg;
use steel::rvals::SteelVal;
use termina::event::{KeyCode, Modifiers, MouseEventKind};

use super::super::commands::{self, PendingDispatch};
use super::super::edit_session::DotWaiter;
use super::super::event::PendingWork;
use super::super::replay::{DispatchId, ReadScope, dot_refusal};
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PopupEviction, Removal};

/// What the character read is for.
enum CharThen {
    /// Run a native command dispatched without its character.
    Dispatch(PendingDispatch),
    /// Call a Steel `(on-next-char! …)` callback with the character, or with
    /// `#f` when none is read.
    Steel(SteelVal),
    /// The macro register `Q` records into or `q` replays.
    Macro(MacroRead),
    /// The register a `"` prefix names for the next yank/delete/change/paste.
    Register,
}

/// What a macro register name read after `Q` or `q` is for.
pub(in crate::editor) enum MacroRead {
    Record,
    /// Replay the macro `count` times.
    Replay {
        count: usize,
    },
}

/// One pending read. A command's or a callback's read is an interactive
/// input: only [`read_command_char`] and [`read_steel_char`] make one.
pub(in crate::editor) struct CharRead {
    then: CharThen,
    /// The read's claim on the capture of the Insert-key binding that made
    /// it. Only a command or a callback read holds one: a register or macro
    /// name edits no text.
    dot_waiter: Option<DotWaiter>,
    /// The Steel command dispatch whose recording the answer goes into.
    owner: Option<DispatchId>,
}

impl CharRead {
    /// The register a `"` prefix names.
    pub(in crate::editor) fn register() -> Self {
        Self {
            then: CharThen::Register,
            dot_waiter: None,
            owner: None,
        }
    }

    /// The macro register `Q` records into or `q` replays.
    pub(in crate::editor) fn macro_register(read: MacroRead) -> Self {
        Self {
            then: CharThen::Macro(read),
            dot_waiter: None,
            owner: None,
        }
    }
}

/// Reads the character the native command `pending` was dispatched
/// without, then runs it.
pub(in crate::editor) fn read_command_char(
    state: &mut EditorState,
    view: &EngineView,
    pending: PendingDispatch,
) -> Result<(), String> {
    let what = pending.name.to_string();
    read_interactive(state, view, CharThen::Dispatch(pending), &what)
}

/// `on-next-char!`: calls `callback` with the next character, or with `#f`.
pub(in crate::editor) fn read_steel_char(
    state: &mut EditorState,
    view: &EngineView,
    callback: SteelVal,
) -> Result<(), String> {
    read_interactive(state, view, CharThen::Steel(callback), "on-next-char!")
}

/// The one funnel a command's or a callback's read starts through. During
/// `.` it takes the next recorded answer, or refuses with none left;
/// otherwise it passes `EditorState::waiting_input` and waits for a key.
fn read_interactive(
    state: &mut EditorState,
    view: &EngineView,
    then: CharThen,
    what: &str,
) -> Result<(), String> {
    let owner = state.read_scope.as_ref().map(ReadScope::owner);
    if let Some(replay) = state.dot_replay.as_mut() {
        let answer = replay
            .answers
            .pop_front()
            .ok_or_else(|| dot_refusal(what))?;
        let read = CharRead {
            then,
            dot_waiter: None,
            owner,
        };
        replay.answered.push_back((read, answer));
        return Ok(());
    }
    let input = state.waiting_input(what)?;
    let read = CharRead {
        then,
        dot_waiter: input.into_waiter(),
        owner,
    };
    read_char(state, view, read);
    Ok(())
}

/// What a read leaves to run once it has its answer.
pub(in crate::editor) enum Resolved {
    Nothing,
    /// Run the native command with the character, under the read's claim.
    Command {
        pending: PendingDispatch,
        ch: char,
        waiter: Option<DotWaiter>,
    },
    /// Call the callback: a `PendingWork::Call` carrying the read's claim
    /// and owner.
    Call(PendingWork),
}

/// Ends `read` with `answer`, the character read or `None` when it ended
/// without one: records the answer for the read's owner and applies what
/// needs no `Editor`, handing back the rest. Every read ends here, live or
/// during `.`.
pub(in crate::editor) fn resolve(
    state: &mut EditorState,
    read: CharRead,
    answer: Option<char>,
) -> Resolved {
    let CharRead {
        then,
        dot_waiter,
        owner,
    } = read;
    if let Some(owner) = owner {
        state.record_read_answer(owner, answer);
    }
    match (then, answer) {
        (CharThen::Dispatch(pending), Some(ch)) => Resolved::Command {
            pending,
            ch,
            waiter: dot_waiter,
        },
        (CharThen::Dispatch(_), None) => {
            if let Some(waiter) = dot_waiter {
                state.release_dot_waiter(waiter);
            }
            Resolved::Nothing
        }
        (CharThen::Steel(callback), answer) => {
            let arg = answer.map_or(SteelVal::BoolV(false), SteelVal::CharV);
            Resolved::Call(PendingWork::Call {
                proc: callback,
                args: vec![arg],
                anchor: None,
                dot_waiter,
                read_owner: owner,
            })
        }
        (CharThen::Macro(MacroRead::Record), Some(ch)) => {
            let reg = if ch == 'Q' { MACRO_REGISTER } else { ch };
            state.macro_recording = Some((reg, Vec::new()));
            state.skip_macro_record = true;
            Resolved::Nothing
        }
        (CharThen::Macro(MacroRead::Replay { count }), Some(ch)) => {
            state.enqueue_macro_replay(ch, count);
            Resolved::Nothing
        }
        (CharThen::Register, Some(ch)) => {
            state.register_prefix = Some(ch);
            Resolved::Nothing
        }
        (CharThen::Macro(_) | CharThen::Register, None) => Resolved::Nothing,
    }
}

/// Runs a resolved command read, handing back a callback call for the
/// caller to queue (a key) or run at once (`.`).
pub(in crate::editor) fn run_resolved(ed: &mut Editor, resolved: Resolved) -> Option<PendingWork> {
    match resolved {
        Resolved::Nothing => None,
        Resolved::Command {
            pending,
            ch,
            waiter,
        } => {
            let run = |ed: &mut Editor| {
                commands::run_collected(&mut ed.state, &mut ed.view, pending, CommandArg::Char(ch));
            };
            ed.resolve_dot_waiter(waiter, run);
            None
        }
        Resolved::Call(call) => Some(call),
    }
}

/// The pending reads, oldest first: the next key goes to the front one.
/// Never empty while the layer is on the stack.
pub(in crate::editor) struct CharLayer {
    reads: VecDeque<CharRead>,
}

impl Layer for CharLayer {
    fn handler(&self) -> LayerHandler {
        char_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }
    /// Leaves a mode's sticky popup (signature help) in place, as reading a
    /// character doesn't leave the mode.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    /// Ends every read still pending without a character. A Steel callback
    /// hears `#f`. A count typed before `"` stays for the next command.
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        for read in self.reads.drain(..) {
            if let Resolved::Call(call) = resolve(state, read, None) {
                state.config.pending_work.push_back(call);
            }
        }
    }
}

/// Wait for a key to supply `read`'s character: the next one, or the one
/// after those the reads already pending take, wherever their layer sits
/// in the stack. A register or macro name read starts here directly.
pub(in crate::editor) fn read_char(state: &mut EditorState, view: &EngineView, read: CharRead) {
    match state.input.find_mut::<CharLayer>() {
        Some(layer) => layer.reads.push_back(read),
        None => {
            let reads = VecDeque::from([read]);
            state.push_layer(view, CharLayer { reads });
        }
    }
}

fn char_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let escape = matches!(&ev, InputEvent::Key(key) if key.code == KeyCode::Escape);
    let read = match ev {
        InputEvent::Key(key) => {
            let plain = (key.modifiers - Modifiers::SHIFT).is_empty();
            match key.code {
                KeyCode::Char(ch) => Some((ch, plain)),
                KeyCode::Enter => Some(('\n', plain)),
                KeyCode::Tab => Some(('\t', plain)),
                _ => None,
            }
        }
        InputEvent::Mouse(mouse)
            if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight
            ) =>
        {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
        InputEvent::Paste(_) | InputEvent::Mouse(_) => None,
    };
    let layer = ed
        .state
        .input
        .at_mut::<CharLayer>(r)
        .expect("char_input runs for its own CharLayer");
    let front = &layer.reads.front().expect("a CharLayer holds a read").then;
    let Some((ch, _)) = read.filter(|&(ch, plain)| fits(front, ch, plain)) else {
        // Esc resets a count typed before `"`, as it does in Normal mode.
        if escape && matches!(front, CharThen::Register) {
            ed.state.count = None;
        }
        ed.state.truncate_layers(&ed.view, r);
        return;
    };
    let read = layer.reads.pop_front().expect("a CharLayer holds a read");
    if layer.reads.is_empty() {
        ed.state.truncate_layers(&ed.view, r);
    }
    let resolved = resolve(&mut ed.state, read, Some(ch));
    if let Some(call) = run_resolved(ed, resolved) {
        ed.state.config.pending_work.push_back(call);
    }
}

/// Whether `ch` completes a read for `then`: any character for a command or a
/// callback, a plain register name for `q`, `Q` and `"`. `QQ` records into
/// the default macro register.
fn fits(then: &CharThen, ch: char, plain: bool) -> bool {
    match then {
        CharThen::Macro(MacroRead::Record) => plain && (ch == 'Q' || is_valid_macro_register(ch)),
        CharThen::Macro(MacroRead::Replay { .. }) => plain && is_valid_macro_register(ch),
        CharThen::Register => plain && is_valid_register_name(ch),
        CharThen::Dispatch(_) | CharThen::Steel(_) => true,
    }
}
