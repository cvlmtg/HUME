//! The disk-change confirm layer — a reusable native yes/no confirmation
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
/// confirm use — see [`ConfirmLayer`]'s doc.
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
/// editor-internal yes/no questions — disk-change reload is the first one.
///
/// `choices[0]` is the accept choice — pressing its key runs `action`.
/// Every other listed choice dismisses without running `action`, doing
/// whatever else its own key implies (`decline_disk_change` for "keep").
/// There is currently never more than one non-accept outcome, so this
/// intentionally doesn't model per-choice actions beyond the first. `Esc`
/// and any listed choice's key are *consumed*; any other stray key also
/// dismisses without answering but is left to fall through to normal
/// dispatch ([`confirm_input`]) rather than being swallowed. No separate
/// view type: [`ConfirmLayer::render_line`] is painted directly by
/// `hume-editor`'s statusline — `pub(crate)`, not `pub(in crate::editor)`
/// like every other type in this module, since the statusline lives at
/// `crate::statusline`, a sibling of `crate::editor` rather than a
/// descendant of it.
pub(crate) struct ConfirmLayer {
    pub(in crate::editor) prompt: String,
    pub(in crate::editor) choices: Vec<ConfirmChoice>,
    pub(in crate::editor) action: ConfirmAction,
}

impl ConfirmLayer {
    /// Whether answering this confirm would act on `id` — i.e. whether `id`
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
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView) {}
}

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.confirm()`) stay as they are, and `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn confirm(&self) -> Option<&ConfirmLayer> {
        self.find()
    }
}

impl EditorState {
    /// The open disk-change confirm, if any — `crate::statusline`'s reader.
    /// A plain wrapper rather than exposing `input` itself at `pub(crate)`:
    /// `InputStack`'s own API stays `pub(in crate::editor)` (see its own
    /// doc for why), and the statusline is a sibling of `crate::editor`,
    /// not a descendant of it, so it needs a seam drawn somewhere — this is
    /// the narrowest one, mirroring `ConfirmLayer`'s own `pub(crate)`
    /// carve-out for the same reader. `EditorState::minibuf` (`mod.rs`) is
    /// the same carve-out for the field that was `pub(crate)` directly on
    /// `EditorState` before it moved into a mode layer's payload.
    pub(crate) fn confirm(&self) -> Option<&ConfirmLayer> {
        self.input.confirm()
    }
}

/// Handles one key while a native confirm overlay ([`ConfirmLayer`]) is
/// open. Always truncates itself first (there is exactly one way for this
/// layer to leave the stack — every key retires it, matched or not — so
/// "answer, then act" and "dismiss, then fall through" both start from
/// the same truncate), then answers the matched choice or falls through
/// for any other key.
///
/// `choices[0]`'s key runs `action`; `choices[1]`'s key (currently always
/// "keep", set by `open_disk_change_confirm`) records an explicit decline
/// (`Editor::decline_disk_change`) so `check_buffer_disk_state` doesn't
/// reopen the same question for the same on-disk signature. Both require
/// no modifiers — a modified key (`Ctrl-k`, the kitty one-shot extend for
/// `move-up`) was aimed at its own binding, not at this prompt, so it must
/// not be mistaken for a bare `k`. Every other key — `Esc`, or a stray
/// keystroke that happens to land here — is a plain dismissal: it answers
/// neither choice, leaving the question open for the next `BufferEnter`,
/// exactly as if the confirm had never opened. `Esc` is fully consumed;
/// any other unmatched key falls through instead, so a prompt the user
/// didn't notice never eats a keystroke meant for the editor (e.g. `/`
/// opening search).
///
/// A mouse event gets the same "stray input dismisses, then falls
/// through" treatment — but only a fresh press or wheel notch counts as
/// stray; a release, drag, or move is the tail of a gesture already in
/// flight (see [`is_fresh_gesture`]'s doc) and falls through untouched,
/// leaving the confirm open. Without that split the confirm would be
/// unreachable by its own most common trigger: clicking into another
/// pane opens it at the next `settle()`, and the click's matching `Up`
/// arrives one loop iteration later.
pub(in crate::editor) fn confirm_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A paste is not one of the choice keys and is never a stray
        // keystroke meant for whatever lies underneath — swallowed
        // outright, the confirm left open, matching this layer's
        // full-modal choice-key policy for anything else unmatched.
        InputEvent::Paste(_) => return,
        InputEvent::Mouse(mouse) => {
            if is_fresh_gesture(mouse.kind) {
                ed.state.input.truncate(r);
            }
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    let mut removed = ed.state.input.truncate(r);
    let Some(confirm) = removed.pop().and_then(|l| l.downcast::<ConfirmLayer>()) else {
        unreachable!("dispatch_at already checked kind(r) == ConfirmLayer");
    };
    let confirm = *confirm;

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
