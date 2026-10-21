//! A paste: text landed at the top of the stack through each layer's
//! [`Layer::paste_handler`](super::Layer::paste_handler).
//! [`EditorState::paste`] is the one implementation; an entry point only
//! builds a [`PastedText`] and calls it.

use std::borrow::Cow;

use hume_editing::text::normalize_line_endings_owned;
use hume_engine::pipeline::EngineView;

use super::super::EditorState;
use super::super::error::CommandError;
use super::super::minibuf::flatten_single_line;
use super::stack::LayerRef;

/// Pasted values with normalized line endings, not all empty: the only form a
/// paste reaches a layer in. A terminal paste is one value; a register holds
/// one per yanked selection, which a buffer places as `p` does and a
/// single-line input receives joined ([`Self::single_line`]).
///
/// Line endings are normalized on entry because a paste also reaches the
/// minibuffer (`flatten_single_line` takes normalized input) and the
/// insert-session replay log, which no changeset builder normalizes.
/// Terminals commonly send CR or CRLF for a newline in a bracketed paste.
pub(in crate::editor) struct PastedText(Vec<String>);

impl PastedText {
    /// `None` when every value is empty, which pastes nothing.
    pub(in crate::editor) fn new(values: Vec<String>) -> Option<Self> {
        let values: Vec<String> = values
            .into_iter()
            .map(normalize_line_endings_owned)
            .collect();
        values.iter().any(|v| !v.is_empty()).then_some(Self(values))
    }

    pub(in crate::editor) fn values(&self) -> &[String] {
        &self.0
    }

    /// Every value in order, joined and flattened to one line.
    pub(in crate::editor) fn single_line(&self) -> String {
        let joined = match self.0.as_slice() {
            [one] => Cow::Borrowed(one.as_str()),
            values => Cow::Owned(values.concat()),
        };
        flatten_single_line(&joined)
    }

    pub(in crate::editor) fn into_values(self) -> Vec<String> {
        self.0
    }
}

impl EditorState {
    /// Paste `text` into whatever layer is on top of the stack. `Err` when
    /// the layer refused it.
    pub(in crate::editor) fn paste(
        &mut self,
        view: &EngineView,
        text: PastedText,
    ) -> Result<(), CommandError> {
        let top = self.input.top();
        self.paste_at(view, top, text)
    }

    fn paste_at(
        &mut self,
        view: &EngineView,
        r: LayerRef,
        text: PastedText,
    ) -> Result<(), CommandError> {
        let f = self
            .input
            .paste_handler(r)
            .expect("paste target is live: top(), or below(r) under the index invariant");
        f(self, view, r, text)
    }

    /// Hand `text` to the layer directly below `r`, which stays addressable
    /// whether or not `r` was just truncated (see `Editor::fall_through`).
    pub(in crate::editor) fn fall_through_paste(
        &mut self,
        view: &EngineView,
        r: LayerRef,
        text: PastedText,
    ) -> Result<(), CommandError> {
        let below = self.input.below(r);
        self.paste_at(view, below, text)
    }
}

/// The paste handler of a layer that swallows a paste and stays open.
pub(in crate::editor) fn ignore_paste(
    _state: &mut EditorState,
    _view: &EngineView,
    _r: LayerRef,
    _text: PastedText,
) -> Result<(), CommandError> {
    Ok(())
}
