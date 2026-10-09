//! The `Char` layer: the next key typed is read as a character for whatever
//! asked for one, a native command dispatched without its character (`f`,
//! `r`, `(call! "replace" pane)`) or a Steel `(on-next-char! …)` callback.
//! Enter and Tab read as `'\n'` and `'\t'`. Any other key or event ends the
//! read and is swallowed.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_scripting::host::CommandArg;
use steel::rvals::SteelVal;
use termina::event::KeyCode;

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
}

pub(in crate::editor) struct CharLayer {
    pub(in crate::editor) then: CharThen,
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
    /// A Steel callback hears `#f` however the read ends without a
    /// character; the accept path takes the layer instead of tearing it down.
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        if let CharThen::Steel(callback) = &self.then {
            state.queue_steel_call(callback.clone(), vec![SteelVal::BoolV(false)]);
        }
    }
}

/// Wait for the next key to supply `then`'s character.
pub(in crate::editor) fn read_char(state: &mut EditorState, view: &EngineView, then: CharThen) {
    state.push_layer(view, CharLayer { then });
}

fn char_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let ch = match ev {
        InputEvent::Key(key) => match key.code {
            KeyCode::Char(ch) => Some(ch),
            KeyCode::Enter => Some('\n'),
            KeyCode::Tab => Some('\t'),
            _ => None,
        },
        InputEvent::Paste(_) | InputEvent::Mouse(_) => None,
    };
    let Some(ch) = ch else {
        ed.state.truncate_layers(&ed.view, r);
        return;
    };
    let layer = ed.state.take_layer::<CharLayer>(&ed.view, r);
    match layer.then {
        CharThen::Dispatch(pending) => {
            commands::run_collected(&mut ed.state, &mut ed.view, pending, CommandArg::Char(ch));
        }
        CharThen::Steel(callback) => ed
            .state
            .queue_steel_call(callback, vec![SteelVal::CharV(ch)]),
    }
}
