//! The disk-change confirm layer: a reusable native yes/no confirmation
//! overlay, rendered in the statusline row. See [`ConfirmLayer`]'s own doc
//! for why it's Rust-native rather than a Steel-facing primitive like the
//! other overlay layers.

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::types::EditorMode;

use super::super::mouse::is_fresh_gesture;
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

/// One key the user can press while a [`ConfirmLayer`] is open, and its
/// display label (e.g. `"reload"` for key `'r'`).
pub(in crate::editor) struct ConfirmChoice {
    pub(in crate::editor) key: char,
    pub(in crate::editor) label: &'static str,
}

/// What happens when the user accepts the confirm (presses `choices[0].key`).
///
/// A plain enum, not a boxed closure: the handler is one `match` arm, and it
/// can't accidentally capture stale editor state. Add a variant per new
/// confirm use; see [`ConfirmLayer`]'s doc.
pub(in crate::editor) enum ConfirmAction {
    /// Reload this buffer from disk, discarding any in-editor edits.
    /// Undoable: `reload_buffer_in_place` records the reload as a single
    /// revision on top of the existing undo tree.
    ReloadBuffer(BufferId),
}

/// A reusable native yes/no confirmation overlay, rendered in the
/// statusline row.
///
/// Unlike [`super::menu::MenuLayer`] (a Steel-facing `(show-menu! …)`
/// primitive with a `SteelVal` callback), a confirm is Rust-native: its
/// action is a plain enum matched inline, with no closure capturing `&mut
/// Editor` and no round-trip through the scripting VM. It exists for
/// editor-internal yes/no questions; disk-change reload is the first one.
///
/// `choices[0]` is the accept choice: pressing its key runs `action`.
/// Every other listed choice dismisses without running `action`, doing
/// whatever else its own key implies (`decline_disk_change` for "keep").
/// There is currently never more than one non-accept outcome, so this
/// intentionally doesn't model per-choice actions beyond the first. `Esc`
/// and any listed choice's key are *consumed*; any other stray key also
/// dismisses without answering but is left to fall through to normal
/// dispatch ([`confirm_input`]) rather than being swallowed. No separate
/// view type: [`ConfirmLayer::render_line`] is painted directly by
/// `hume-editor`'s statusline, so `pub(crate)`, not `pub(in crate::editor)`
/// like every other type in this module, since the statusline lives at
/// `crate::statusline`, a sibling of `crate::editor` rather than a
/// descendant of it.
pub(crate) struct ConfirmLayer {
    pub(in crate::editor) prompt: String,
    pub(in crate::editor) choices: Vec<ConfirmChoice>,
    pub(in crate::editor) action: ConfirmAction,
}

impl ConfirmLayer {
    /// Whether answering this confirm would act on `id`, i.e. whether `id`
    /// disappearing leaves the question unanswerable. Read by
    /// `buffer::lifecycle::close_buffer_and_notify`, which retires such a
    /// confirm rather than leaving one on screen whose only possible outcome
    /// is a silent no-op. A `match`, not a `matches!`, so the next `action`
    /// variant this module gains is forced to decide here rather than
    /// defaulting to "unaffected".
    pub(in crate::editor) fn targets_buffer(&self, id: BufferId) -> bool {
        match self.action {
            ConfirmAction::ReloadBuffer(bid) => bid == id,
        }
    }

    /// The line to paint in the statusline row: prompt text followed by
    /// each choice as `[key]label`.
    pub(crate) fn render_line(&self) -> String {
        let mut out = self.prompt.clone();
        for choice in &self.choices {
            out.push_str("  [");
            out.push(choice.key);
            out.push(']');
            out.push_str(choice.label);
        }
        out
    }
}

impl Layer for ConfirmLayer {
    fn handler(&self) -> LayerHandler {
        confirm_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    // No `setup`/`popup_eviction` override: the trait's own default
    // (`PopupEviction::Both`, evicted automatically by `push_layer`) is
    // exactly right here: `can_open_confirm` (`buffer/disk.rs`) deliberately
    // does *not* gate on a popup being open (a `Scrollable` one owns no keys
    // beyond Ctrl-u/d and dies on the next one anyway), so a confirm lands
    // directly above one and must evict it on the way in, same as every
    // other opener that can land above one.
}

/// Named sugar over the generic lookup; `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn confirm(&self) -> Option<&ConfirmLayer> {
        self.find()
    }
}

impl EditorState {
    /// The confirm that owns the statusline row right now, `crate::statusline`'s
    /// reader, and *not* the same query as `InputStack::confirm()`: a confirm
    /// can be buried (a timer's `prompt!` lands above it; `push_mode_layer`
    /// only truncates the outgoing *mode* layer, never an overlay sitting on
    /// top of `Base`), in which case it's still on the stack and still the
    /// right target for `buffer::disk`/`buffer::lifecycle`'s retire-on-close
    /// checks (`InputStack::confirm()`, used there), but it no longer owns
    /// the keyboard or this row. A buried confirm painted here would show
    /// the wrong prompt while the minibuffer above it reads the keys.
    /// `Some` only when the confirm is `top()`.
    ///
    /// A plain wrapper rather than exposing `input` itself at `pub(crate)`:
    /// `InputStack`'s own API stays `pub(in crate::editor)` (see its own
    /// doc for why), and the statusline is a sibling of `crate::editor`,
    /// not a descendant of it, so it needs a seam drawn somewhere, and this is
    /// the narrowest one, mirroring `ConfirmLayer`'s own `pub(crate)`
    /// carve-out for the same reader. `EditorState::minibuf` (`mod.rs`) is
    /// the same carve-out for the field that was `pub(crate)` directly on
    /// `EditorState` before it moved into a mode layer's payload.
    pub(crate) fn confirm(&self) -> Option<&ConfirmLayer> {
        self.input.at(self.input.top())
    }

    /// Retires the open confirm (if any) when `stale` says it no longer
    /// belongs: the shared body of `buffer::disk::enter_buffer_disk_check`
    /// (stale because focus moved off the buffer it targets) and
    /// `buffer::lifecycle::close_buffer_and_notify` (stale because its
    /// target buffer is being freed). Uses `excise_layer`, not
    /// `truncate_layers`: nothing guarantees the confirm is still `top()`:
    /// a `Prompt`/`Picker` opened above it since (which `push_mode_layer`'s
    /// own truncation never reaches, as an overlay on `Base` isn't a mode
    /// layer) has nothing to do with the question this confirm was
    /// answering and must survive the retirement untouched. A no-op when no
    /// confirm is open, or `stale` says the open one still applies.
    pub(in crate::editor) fn retire_stale_confirm(
        &mut self,
        view: &EngineView,
        stale: impl FnOnce(&ConfirmLayer) -> bool,
    ) {
        if self.input.confirm().is_some_and(stale)
            && let Some(r) = self.input.ref_of::<ConfirmLayer>()
        {
            self.excise_layer(view, r);
        }
    }
}

/// Handles one input event while a native confirm overlay ([`ConfirmLayer`])
/// is open. Every key retires the layer, matched or not.
///
/// An unmodified `choices[0]` key runs `action`; an unmodified `choices[1]` key
/// ("keep") records a decline via `Editor::decline_disk_change` so
/// `check_buffer_disk_state` doesn't ask again for the same on-disk signature.
/// Modified keys never match: `Ctrl-k` targets its own binding, not `k`. Any
/// other key dismisses without answering, leaving the question open for the
/// next `BufferEnter`. `Esc` is consumed; other keys fall through so an
/// unnoticed prompt never eats a keystroke.
///
/// A fresh mouse press or wheel notch also dismisses and falls through. A
/// release, drag, or move (see [`is_fresh_gesture`]) falls through and leaves
/// the confirm open, since a click into another pane opens it and the click's
/// `Up` arrives one iteration later.
pub(in crate::editor) fn confirm_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A paste is not one of the choice keys and is never a stray
        // keystroke meant for whatever lies underneath. Swallowed
        // outright, the confirm left open, matching this layer's
        // full-modal choice-key policy for anything else unmatched.
        InputEvent::Paste(_) => return,
        InputEvent::Mouse(mouse) => {
            if is_fresh_gesture(mouse.kind) {
                ed.state.truncate_layers(&ed.view, r);
            }
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    let confirm = *ed.state.take_layer::<ConfirmLayer>(&ed.view, r);

    let matched = (key.modifiers == Modifiers::NONE)
        .then(|| {
            confirm
                .choices
                .iter()
                .position(|c| key.code == KeyCode::Char(c.key))
        })
        .flatten();

    match confirm.action {
        ConfirmAction::ReloadBuffer(bid) => match matched {
            Some(0) => ed.reload_buffer_from_disk(bid),
            Some(1) => ed.decline_disk_change(bid),
            _ => {}
        },
    }

    if matched.is_none() && key.code != KeyCode::Escape {
        ed.fall_through(r, InputEvent::Key(key));
    }
}

#[cfg(test)]
mod tests;
