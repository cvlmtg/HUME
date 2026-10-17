//! A paste: text landed at the top of the stack through each layer's
//! [`Layer::paste_handler`](super::Layer::paste_handler).
//! [`EditorState::paste`] is the one implementation; an entry point only
//! builds a [`PastedText`] and calls it.

use std::borrow::Cow;

use hume_editing::text::normalize_line_endings;
use hume_engine::pipeline::EngineView;

use super::super::EditorState;
use super::stack::LayerRef;

/// Pasted text with normalized line endings, never empty: the only form a
/// paste reaches a layer in.
pub(in crate::editor) struct PastedText(String);

impl PastedText {
    /// `None` for empty text, which pastes nothing.
    pub(in crate::editor) fn new(text: String) -> Option<Self> {
        // Normalized here rather than left to the changeset builder: this
        // text also reaches the minibuffer (via
        // `minibuf::flatten_single_line`, whose contract is
        // already-normalized input) and the insert-session replay log,
        // neither of which is behind that builder. Terminals commonly
        // transmit CR or CRLF for a newline in a bracketed paste regardless
        // of the source's own convention.
        //
        // `text` is already owned, and the common case (no bare CR) returns
        // `Cow::Borrowed`, so reuse `text` itself rather than re-copying it
        // via `into_owned()`, which would memcpy the whole paste a second
        // time.
        let text = match normalize_line_endings(&text) {
            Cow::Borrowed(_) => text,
            Cow::Owned(s) => s,
        };
        (!text.is_empty()).then_some(Self(text))
    }

    pub(in crate::editor) fn as_str(&self) -> &str {
        &self.0
    }

    pub(in crate::editor) fn into_string(self) -> String {
        self.0
    }
}

impl EditorState {
    /// Paste `text` into whatever layer is on top of the stack.
    pub(in crate::editor) fn paste(&mut self, view: &EngineView, text: PastedText) {
        let top = self.input.top();
        self.paste_at(view, top, text);
    }

    fn paste_at(&mut self, view: &EngineView, r: LayerRef, text: PastedText) {
        let f = self
            .input
            .paste_handler(r)
            .expect("paste target is live: top(), or below(r) under the index invariant");
        f(self, view, r, text);
    }

    /// Hand `text` to the layer directly below `r`, which stays addressable
    /// whether or not `r` was just truncated (see `Editor::fall_through`).
    pub(in crate::editor) fn fall_through_paste(
        &mut self,
        view: &EngineView,
        r: LayerRef,
        text: PastedText,
    ) {
        let below = self.input.below(r);
        self.paste_at(view, below, text);
    }
}

/// The paste handler of a layer that swallows a paste and stays open.
pub(in crate::editor) fn ignore_paste(
    _state: &mut EditorState,
    _view: &EngineView,
    _r: LayerRef,
    _text: PastedText,
) {
}
