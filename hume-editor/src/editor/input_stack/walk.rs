//! The input stack walk: handing an event to the topmost layer, and from a
//! layer to the one below it.

use super::{InputEvent, LayerRef};
use crate::editor::Editor;

impl Editor {
    /// Bare stack walk, no cross-cutting bookkeeping: the single entry
    /// point every key and mouse event reaches the stack through, whatever
    /// its source. `handle_key` wraps it with the status/summary
    /// bookkeeping, macro recording, and dot-repeat replay above and below,
    /// for a key from the terminal; `handle_input`'s mouse arm calls it
    /// directly, carrying none of that key-only bookkeeping. A paste takes
    /// `EditorState::paste` instead.
    pub(in crate::editor) fn dispatch_input(&mut self, ev: InputEvent) {
        let top = self.state.input.top();
        self.dispatch_at(top, ev);
    }

    /// Routes to whichever handler owns `r`'s concrete type: a vtable
    /// read (`Layer::handler`), ending the borrow on `self.state.input`
    /// before the handler itself runs, since every handler needs the whole
    /// `&mut Editor` (which transitively owns the stack `r` names a layer
    /// on). Every layer states its own policy for the event it's handed
    /// (handle / fall through / discard). This dispatcher has no
    /// per-layer knowledge at all, which is the point: adding a layer
    /// never touches this function.
    fn dispatch_at(&mut self, r: LayerRef, ev: InputEvent) {
        let f = self
            .state
            .input
            .handler(r)
            .expect("dispatch target is live: top(), or below(r) under the index invariant");
        f(self, r, ev);
    }

    /// Hand `ev` to the layer directly below `r`. The caller never inspects
    /// what is there: every index below `r.depth` is stable for the whole
    /// duration of the handler running at `r` (the index invariant), so the
    /// target is live whether `r` itself is still on the
    /// stack or was just truncated by the caller (self-removal is always
    /// `truncate(r)`, never "pop the top", so `below(r)` works either way).
    /// `LayerRef`'s fields are private to `InputStack`'s own module, so the
    /// "never fall through from `Base`" invariant is enforced there instead:
    /// `InputStack::below` panics unconditionally (not just in debug
    /// builds) if `r` is already `Base`, which is strictly louder than a
    /// `debug_assert!` here could be.
    pub(in crate::editor) fn fall_through(&mut self, r: LayerRef, ev: InputEvent) {
        let below = self.state.input.below(r);
        self.dispatch_at(below, ev);
    }
}
