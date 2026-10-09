//! The `Char` layer: the next key typed is read as a character for whatever
//! asked for one: a native command dispatched without its character (`f`,
//! `r`, `(call! "replace" pane)`), a Steel `(on-next-char! …)` callback, or
//! the register name after `q`, `Q` or `"`. Reads made while one is pending
//! take the following keys in the order they were made. Enter and Tab read
//! as `'\n'` and `'\t'`. A register name must also be held with no modifier
//! but Shift and name a valid register. The scroll wheel passes through and
//! leaves the read pending. Any other key, a click or a paste ends the read
//! and is swallowed.

use std::collections::VecDeque;

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::register::{MACRO_REGISTER, is_valid_macro_register, is_valid_register_name};
use hume_scripting::host::CommandArg;
use steel::rvals::SteelVal;
use termina::event::{KeyCode, Modifiers, MouseEventKind};

use super::super::commands::{self, PendingDispatch};
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PopupEviction, Removal};

/// What the character read is for.
pub(in crate::editor) enum CharThen {
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

/// The pending reads, oldest first: the next key goes to the front one.
/// Never empty while the layer is on the stack.
pub(in crate::editor) struct CharLayer {
    reads: VecDeque<CharThen>,
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
        for then in self.reads.drain(..) {
            match then {
                CharThen::Steel(callback) => {
                    state.queue_steel_call(callback, vec![SteelVal::BoolV(false)]);
                }
                CharThen::Register | CharThen::Dispatch(_) | CharThen::Macro(_) => {}
            }
        }
    }
}

/// Wait for a key to supply `then`'s character: the next one, or the one
/// after those the reads already pending take, wherever their layer sits
/// in the stack.
pub(in crate::editor) fn read_char(state: &mut EditorState, view: &EngineView, then: CharThen) {
    match state.input.find_mut::<CharLayer>() {
        Some(layer) => layer.reads.push_back(then),
        None => {
            let reads = VecDeque::from([then]);
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
    let front = layer.reads.front().expect("a CharLayer holds a read");
    let Some((ch, _)) = read.filter(|&(ch, plain)| fits(front, ch, plain)) else {
        // Esc resets a count typed before `"`, as it does in Normal mode.
        if escape && matches!(front, CharThen::Register) {
            ed.state.count = None;
        }
        ed.state.truncate_layers(&ed.view, r);
        return;
    };
    let then = layer.reads.pop_front().expect("a CharLayer holds a read");
    if layer.reads.is_empty() {
        ed.state.truncate_layers(&ed.view, r);
    }
    let state = &mut ed.state;
    match then {
        CharThen::Dispatch(pending) => {
            commands::run_collected(state, &mut ed.view, pending, CommandArg::Char(ch));
        }
        CharThen::Steel(callback) => state.queue_steel_call(callback, vec![SteelVal::CharV(ch)]),
        CharThen::Macro(MacroRead::Record) => {
            let reg = if ch == 'Q' { MACRO_REGISTER } else { ch };
            state.macro_recording = Some((reg, Vec::new()));
            state.skip_macro_record = true;
        }
        CharThen::Macro(MacroRead::Replay { count }) => state.enqueue_macro_replay(ch, count),
        CharThen::Register => state.register_prefix = Some(ch),
    }
}

/// Whether `ch` completes the read at `r`: any character for a command or a
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
