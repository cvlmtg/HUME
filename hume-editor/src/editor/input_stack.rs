//! The stack of active input-handling layers, from the base editing surface
//! up through whatever transient overlay currently owns the keyboard.
//!
//! Precedence used to be four independent `Option<T>` fields on
//! `ConfigState`, each checked by its own `is_some()` guard in a fixed `&&`
//! chain (`mappings/mod.rs`'s old `handle_key`) — the chain's *order*, not
//! any property of the widgets themselves, decided which one saw a key when
//! more than one happened to be open. A stack makes precedence a fact about
//! the data (whichever layer is on top runs first) rather than a fact about
//! the dispatcher's source order — and it is the *only* precedence
//! mechanism: which layer sees a key, and whether the next one below ever
//! does, is entirely decided by each layer's own handler (handle / fall
//! through / discard). There is no separate rule for which layer may open
//! above which; `push` never refuses. The one gate that exists,
//! [`InputStack::is_stack_settled`], answers a different question — whether
//! an *async* opener's request has gone stale — not who outranks whom.
//!
//! A layer is a *purpose*, not a widget: today each of the four kinds below
//! wraps exactly one widget, but nothing in the stack's own API assumes
//! that stays true.

use termina::event::{KeyEvent, MouseEvent};

use hume_engine::types::EditorMode;
use steel::rvals::SteelVal;

use super::completion::MinibufCompletionState;
use super::lsp::completion::{CompletionMenuUi, CompletionSession};
use super::minibuf::MiniBuffer;
use super::overlay_models::{ConfirmModel, DrawerModel, MenuModel, PopupModel};
use super::picker::PickerSession;

/// Addresses one layer by position — minted only by [`InputStack::push`] and
/// [`InputStack::top`], read back by every other method. `depth` alone would
/// alias: truncating at `depth` and pushing a new layer puts a *different*
/// layer at the same index. `id` is the tiebreaker — see
/// [`InputStack::is_live`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) struct LayerRef {
    depth: usize,
    id: u64,
}

/// Which kind of layer occupies a slot, without its payload — what
/// [`InputStack::kind`]/[`InputStack::mode_layer`] hand back, and what every
/// opener's own mode-layer gate switches on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum LayerKind {
    /// The always-present layer at index 0. Never removed.
    Base,
    Insert,
    Command,
    Search,
    Sift,
    Prompt,
    Drawer,
    Menu,
    Picker,
    Confirm,
    /// An LSP completion session — an overlay, not a mode layer (only ever
    /// pushed above `Insert`; `mode()` skips it, reading `Insert` from the
    /// layer beneath).
    Completion,
    /// A `Scrollable` popup (hover, `gn`/`gp`'s diagnostic overlay) — not a
    /// mode layer. A `Sticky` popup (signature help) uses a different home,
    /// the `sticky_popup` slot on `Base`/`Insert`, and so never appears as
    /// this kind — see `PopupKind`'s two homes, `overlay_models.rs`.
    Popup,
}

/// One entry on the stack.
///
/// `Base` and `Insert` carry `sticky_popup` — the home for a `Sticky` popup
/// (signature help), which belongs to whichever mode owns it rather than to
/// its own layer: it must survive underneath a completion session or an LSP
/// menu without gating either (`is_stack_settled()` never sees a slot), and
/// it dies when its mode layer does, not on a separate Steel hook. A
/// `Scrollable` popup (hover, `gn`/`gp`) instead gets its own `Popup` layer
/// below — the two kinds' dismiss rules differ enough (a slot is invisible
/// to input dispatch; a layer intercepts it) that one shape can't serve
/// both. `Base` also carries `extend` (Extend is a flag on `Base`, never its
/// own layer).
///
/// `Picker` is boxed: `PickerSession` alone is several times the size of
/// every other variant's payload (its own fuzzy-match scoring buffers,
/// picked items, `#:actions` table, …), so leaving it unboxed would size
/// the whole enum — every other layer included — to the picker's own
/// footprint.
pub(in crate::editor) enum InputLayer {
    Base {
        extend: bool,
        sticky_popup: Option<PopupModel>,
    },
    Insert {
        sticky_popup: Option<PopupModel>,
    },
    Command {
        minibuf: MiniBuffer,
        completion: Option<MinibufCompletionState>,
    },
    Search {
        minibuf: MiniBuffer,
    },
    Sift {
        minibuf: MiniBuffer,
    },
    Prompt {
        minibuf: MiniBuffer,
        callback: SteelVal,
    },
    Drawer(DrawerModel),
    Menu(MenuModel),
    Picker(Box<PickerSession>),
    Confirm(ConfirmModel),
    /// An open LSP completion session, pushed above `Insert` — see
    /// `LayerKind::Completion`'s own doc. `ui` is `None` until the first
    /// Tab/Down/BackTab/Up moves the selection off its implicit default of 0.
    Completion {
        session: CompletionSession,
        ui: Option<CompletionMenuUi>,
    },
    /// An open `Scrollable` popup — see `LayerKind::Popup`'s doc. Never
    /// buried: every opener that could otherwise land above it retires it
    /// first (`show_popup`'s self-replace, `open_picker`,
    /// `EditorState::push_mode_layer`) or is itself gated on the stack being
    /// settled, so `close-popup!`/`popup()` never need to look past `top()`.
    Popup(PopupModel),
}

impl InputLayer {
    pub(in crate::editor) fn kind(&self) -> LayerKind {
        match self {
            InputLayer::Base { .. } => LayerKind::Base,
            InputLayer::Insert { .. } => LayerKind::Insert,
            InputLayer::Command { .. } => LayerKind::Command,
            InputLayer::Search { .. } => LayerKind::Search,
            InputLayer::Sift { .. } => LayerKind::Sift,
            InputLayer::Prompt { .. } => LayerKind::Prompt,
            InputLayer::Drawer(_) => LayerKind::Drawer,
            InputLayer::Menu(_) => LayerKind::Menu,
            InputLayer::Picker(_) => LayerKind::Picker,
            InputLayer::Confirm(_) => LayerKind::Confirm,
            InputLayer::Completion { .. } => LayerKind::Completion,
            InputLayer::Popup(_) => LayerKind::Popup,
        }
    }

    /// Whether this kind is a mode layer — the always-present `Base` plus
    /// the five editing modes, as opposed to a transient overlay
    /// (`Drawer`/`Menu`/`Picker`/`Confirm`). Exactly one mode layer is ever
    /// on the stack, and it is always either the top layer or has only
    /// overlays above it — see [`InputStack::mode_layer`].
    fn is_mode_layer(&self) -> bool {
        matches!(
            self.kind(),
            LayerKind::Base
                | LayerKind::Insert
                | LayerKind::Command
                | LayerKind::Search
                | LayerKind::Sift
                | LayerKind::Prompt
        )
    }
}

/// One input event working its way down the stack. Adding a variant forces
/// every layer's handler to state its policy for it at compile time —
/// `Paste` broke all eleven handlers' irrefutable `let InputEvent::Key(key)
/// = ev;` when it arrived, and `Mouse` broke every resulting `Key`/`Paste`
/// match the same way.
pub(in crate::editor) enum InputEvent {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
}

/// The stack itself: `Base` at index 0, always, plus whatever overlay
/// layers are pushed above it.
///
/// No `pop()`, and no `get(r)`/`get_mut(r)` handing back a bare
/// `&mut InputLayer`: either would let a caller overwrite a layer in place
/// or remove one that isn't its own, bypassing the ordering this type
/// exists to enforce. Structure changes only through [`Self::push`],
/// [`Self::truncate`], and [`Self::truncate_to_base`]; a payload is only
/// ever reached through a typed lookup (`picker()`, `confirm()`, …).
pub(in crate::editor) struct InputStack {
    /// Paired with a monotonic id per entry (see `LayerRef`) rather than a
    /// bare `Vec<InputLayer>` — the id is what lets `is_live` tell a stale
    /// `LayerRef` from a fresh layer that landed at the same index.
    layers: Vec<(u64, InputLayer)>,
    next_id: u64,
}

impl InputStack {
    pub(in crate::editor) fn new() -> Self {
        Self {
            layers: vec![(
                0,
                InputLayer::Base {
                    extend: false,
                    sticky_popup: None,
                },
            )],
            next_id: 1,
        }
    }

    /// The topmost layer — every key dispatch starts here.
    pub(in crate::editor) fn top(&self) -> LayerRef {
        let depth = self.layers.len() - 1;
        LayerRef {
            depth,
            id: self.layers[depth].0,
        }
    }

    /// Whether `r` still names the layer it was minted against — `false`
    /// once that index has been truncated and (possibly) refilled by a
    /// later `push`. The only aliasing hole `depth` alone leaves open.
    pub(in crate::editor) fn is_live(&self, r: LayerRef) -> bool {
        self.kind(r).is_some()
    }

    /// `r`'s layer kind, or `None` if `r` is stale (see [`Self::is_live`]).
    pub(in crate::editor) fn kind(&self, r: LayerRef) -> Option<LayerKind> {
        self.layers
            .get(r.depth)
            .filter(|(id, _)| *id == r.id)
            .map(|(_, layer)| layer.kind())
    }

    /// The topmost layer of kind `kind`, if one is open — the ref a
    /// `close-*!` builtin or a Rust-internal retirement needs to name a
    /// widget it didn't itself just push. `truncate` is the only removal
    /// op, so closing a widget that isn't on top would take everything
    /// above it with it — every `close-*!` builtin instead errors when
    /// `ref_of` finds the widget present but not `top()` (every
    /// `confirm`-retirement site on buffer close/focus change reads this
    /// the same way). Every kind but `Base` occurs at most once on the
    /// stack today, so "topmost" and "only" coincide in practice.
    pub(in crate::editor) fn ref_of(&self, kind: LayerKind) -> Option<LayerRef> {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.kind() == kind)
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
    }

    /// The layer directly below `r` — the only way a handler addresses
    /// "whatever is underneath me". Panics if `r` is already `Base`; every
    /// caller ([`Self::below`]'s one caller, `fall_through`) is expected to
    /// have already asserted `r.depth >= 1` itself, so reaching this panic
    /// means that invariant broke, not that a caller made an ordinary
    /// mistake worth a softer `Option`.
    pub(in crate::editor) fn below(&self, r: LayerRef) -> LayerRef {
        let depth = r
            .depth
            .checked_sub(1)
            .expect("below(r) called with r already at Base");
        LayerRef {
            depth,
            id: self.layers[depth].0,
        }
    }

    /// Whether nothing sits above the current mode layer — the staleness
    /// check an *async* opener (a Steel callback answering a request fired
    /// earlier: `show-menu!`, `show-drawer-list!`, `completion-begin!`)
    /// makes before landing, alongside its own mode-layer requirement. It
    /// is not a precedence rule: a synchronous, key- or command-triggered
    /// opener (`picker!`, `prompt!`) never calls this, because dispatch
    /// order already proves the stack is exactly where the key path left
    /// it — there is nothing left to check. An async response has no such
    /// guarantee: the user may have opened a picker, a menu, or moved to a
    /// different mode between the request going out and the response
    /// landing, and this is what tells the two apart. `top() ==
    /// mode_layer()` — an overlay already open (of any kind, including one
    /// this same opener is mid-refreshing) makes this `false`; a caller
    /// replacing its own prior instance checks for that case separately
    /// rather than through this.
    pub(in crate::editor) fn is_stack_settled(&self) -> bool {
        self.top() == self.mode_layer()
    }

    /// Pushes `layer` on top, unconditionally — nothing is refused, since
    /// precedence is push order and push order is only ever decided by
    /// whoever's calling this. An async opener consults
    /// [`Self::is_stack_settled`] itself before calling this (a staleness
    /// check, not a permission check); a kind-specific replace rule (the
    /// picker's own "cancel and replace a live picker") runs first for the
    /// same reason. `push` itself enforces nothing about what's already
    /// open.
    pub(in crate::editor) fn push(&mut self, layer: InputLayer) -> LayerRef {
        let id = self.next_id;
        self.next_id += 1;
        self.layers.push((id, layer));
        LayerRef {
            depth: self.layers.len() - 1,
            id,
        }
    }

    /// Removes `r` and everything above it, returning the removed layers
    /// top-first (the former top layer is `removed[0]`; the layer that was
    /// at `r.depth` itself is `removed`'s last element). A no-op returning
    /// `vec![]` when `r` is already stale, or when `r` is `Base` — `Base`
    /// is never removed.
    pub(in crate::editor) fn truncate(&mut self, r: LayerRef) -> Vec<InputLayer> {
        if r.depth == 0 || !self.is_live(r) {
            return Vec::new();
        }
        self.layers
            .split_off(r.depth)
            .into_iter()
            .rev()
            .map(|(_, layer)| layer)
            .collect()
    }

    /// Removes every layer above `Base`, top-first, same return contract as
    /// [`Self::truncate`], and resets `Base` itself to `extend: false,
    /// sticky_popup: None` — a reload must not leave Extend on, or a
    /// signature-help popup visible, for hooks that never saw either turned
    /// on.
    pub(in crate::editor) fn truncate_to_base(&mut self) -> Vec<InputLayer> {
        let removed = self
            .layers
            .split_off(1)
            .into_iter()
            .rev()
            .map(|(_, layer)| layer)
            .collect();
        self.layers[0].1 = InputLayer::Base {
            extend: false,
            sticky_popup: None,
        };
        removed
    }

    /// The layer that determines `EditorMode` — the first mode layer found
    /// walking down from the top. Exactly one mode layer is ever on the
    /// stack (`Base`, or the one mode layer a `push_mode_layer` call
    /// replaced it with), and an overlay never changes what mode is
    /// underneath it, so this always terminates at either the top itself or
    /// `Base`.
    pub(in crate::editor) fn mode_layer(&self) -> LayerRef {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.is_mode_layer())
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
            .expect("Base is always a mode layer and always on the stack")
    }

    /// The `EditorMode` the current mode layer maps to. `Prompt` reads as
    /// `Command` — the engine has no `Prompt` variant, and today's Steel
    /// `(prompt! …)` session already runs as `Command` for every consumer
    /// outside this crate.
    pub(in crate::editor) fn mode(&self) -> EditorMode {
        match &self.layers[self.mode_layer().depth].1 {
            InputLayer::Base { extend: false, .. } => EditorMode::Normal,
            InputLayer::Base { extend: true, .. } => EditorMode::Extend,
            InputLayer::Insert { .. } => EditorMode::Insert,
            InputLayer::Command { .. } => EditorMode::Command,
            InputLayer::Search { .. } => EditorMode::Search,
            InputLayer::Sift { .. } => EditorMode::Sift,
            InputLayer::Prompt { .. } => EditorMode::Command,
            _ => unreachable!("mode_layer() only ever returns a mode layer"),
        }
    }

    /// Sets `Base`'s `extend` flag directly — `Base` is always at index 0,
    /// so this never needs a lookup. Does not gate on the current mode
    /// layer or clear it on push; `push_mode_layer` clears it on every push
    /// separately, and a toggle reads `mode()` first to decide the target
    /// value.
    pub(in crate::editor) fn set_extend(&mut self, extend: bool) {
        let InputLayer::Base { extend: slot, .. } = &mut self.layers[0].1 else {
            unreachable!("index 0 is always Base");
        };
        *slot = extend;
    }

    /// The active minibuffer, topmost-wins across the four minibuf-backed
    /// mode layers (`Command`/`Search`/`Sift`/`Prompt`) — at most one of
    /// them is ever on the stack, so "topmost" and "only" coincide.
    pub(in crate::editor) fn minibuf(&self) -> Option<&MiniBuffer> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Command { minibuf, .. }
            | InputLayer::Search { minibuf }
            | InputLayer::Sift { minibuf }
            | InputLayer::Prompt { minibuf, .. } => Some(minibuf),
            _ => None,
        })
    }

    pub(in crate::editor) fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Command { minibuf, .. }
                | InputLayer::Search { minibuf }
                | InputLayer::Sift { minibuf }
                | InputLayer::Prompt { minibuf, .. } => Some(minibuf),
                _ => None,
            })
    }

    /// The active completion session, flattened — `None` both when no
    /// `Command` layer is open and when one is open with no session. Reads
    /// only; see [`Self::minibuf_completion_mut`] to replace or clear it.
    pub(in crate::editor) fn minibuf_completion(&self) -> Option<&MinibufCompletionState> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Command { completion, .. } => completion.as_ref(),
            _ => None,
        })
    }

    /// The `Command` layer's completion slot itself (not its content) —
    /// `Some(&mut Option<..>)` whenever a `Command` layer is open, letting a
    /// caller assign a fresh session or clear one (`*slot = None`), as
    /// opposed to [`Self::minibuf_completion`]'s flattened read.
    pub(in crate::editor) fn minibuf_completion_mut(
        &mut self,
    ) -> Option<&mut Option<MinibufCompletionState>> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Command { completion, .. } => Some(completion),
                _ => None,
            })
    }

    /// The open `(prompt! …)` session's callback, if `Prompt` is on the
    /// stack. Read-only — a caller finishing the prompt clones this out
    /// (cheap: `SteelVal` is reference-counted) before truncating the
    /// `Prompt` layer away, rather than taking ownership through this
    /// lookup.
    pub(in crate::editor) fn prompt_callback(&self) -> Option<&SteelVal> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Prompt { callback, .. } => Some(callback),
            _ => None,
        })
    }

    pub(in crate::editor) fn picker(&self) -> Option<&PickerSession> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Picker(session) => Some(session.as_ref()),
            _ => None,
        })
    }

    pub(in crate::editor) fn picker_mut(&mut self) -> Option<&mut PickerSession> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Picker(session) => Some(session.as_mut()),
                _ => None,
            })
    }

    pub(in crate::editor) fn confirm(&self) -> Option<&ConfirmModel> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Confirm(model) => Some(model),
            _ => None,
        })
    }

    pub(in crate::editor) fn menu(&self) -> Option<&MenuModel> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Menu(model) => Some(model),
            _ => None,
        })
    }

    pub(in crate::editor) fn menu_mut(&mut self) -> Option<&mut MenuModel> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Menu(model) => Some(model),
                _ => None,
            })
    }

    pub(in crate::editor) fn drawer(&self) -> Option<&DrawerModel> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Drawer(model) => Some(model),
            _ => None,
        })
    }

    pub(in crate::editor) fn drawer_mut(&mut self) -> Option<&mut DrawerModel> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Drawer(model) => Some(model),
                _ => None,
            })
    }

    pub(in crate::editor) fn completion(&self) -> Option<&CompletionSession> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Completion { session, .. } => Some(session),
            _ => None,
        })
    }

    pub(in crate::editor) fn completion_mut(&mut self) -> Option<&mut CompletionSession> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Completion { session, .. } => Some(session),
                _ => None,
            })
    }

    /// The completion session's UI selection, flattened — same shape as
    /// [`Self::minibuf_completion`]. Reads only; see
    /// [`Self::completion_ui_mut`] to assign or clear it.
    pub(in crate::editor) fn completion_ui(&self) -> Option<&CompletionMenuUi> {
        self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Completion { ui, .. } => ui.as_ref(),
            _ => None,
        })
    }

    /// The `Completion` layer's UI slot itself (not its content) — same
    /// shape as [`Self::minibuf_completion_mut`], for
    /// `move_completion_selection`'s `get_or_insert`.
    pub(in crate::editor) fn completion_ui_mut(&mut self) -> Option<&mut Option<CompletionMenuUi>> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| match layer {
                InputLayer::Completion { ui, .. } => Some(ui),
                _ => None,
            })
    }

    /// The active popup, whichever of its two homes holds it: a `Popup`
    /// layer (checked first — see `LayerKind::Popup`'s "never buried" doc,
    /// which is what makes checking it independently of `mode_layer()`
    /// sound) or the *current* mode layer's own `sticky_popup` slot. Reading
    /// only the current mode layer's slot — not any slot buried below it —
    /// matters when `Base`'s slot holds a value that a later mode-layer push
    /// left behind: `push_mode_layer` clears it before taking over as mode
    /// layer for exactly this reason, but this lookup would still be wrong
    /// to read past `mode_layer()` even if it didn't.
    pub(in crate::editor) fn popup(&self) -> Option<&PopupModel> {
        if let Some(model) = self.layers.iter().rev().find_map(|(_, layer)| match layer {
            InputLayer::Popup(model) => Some(model),
            _ => None,
        }) {
            return Some(model);
        }
        match &self.layers[self.mode_layer().depth].1 {
            InputLayer::Base { sticky_popup, .. } | InputLayer::Insert { sticky_popup } => {
                sticky_popup.as_ref()
            }
            _ => None,
        }
    }

    pub(in crate::editor) fn popup_mut(&mut self) -> Option<&mut PopupModel> {
        let has_popup_layer = self
            .layers
            .iter()
            .any(|(_, layer)| matches!(layer, InputLayer::Popup(_)));
        if has_popup_layer {
            return self
                .layers
                .iter_mut()
                .rev()
                .find_map(|(_, layer)| match layer {
                    InputLayer::Popup(model) => Some(model),
                    _ => None,
                });
        }
        let mode_depth = self.mode_layer().depth;
        match &mut self.layers[mode_depth].1 {
            InputLayer::Base { sticky_popup, .. } | InputLayer::Insert { sticky_popup } => {
                sticky_popup.as_mut()
            }
            _ => None,
        }
    }

    /// The current mode layer's sticky-popup slot, if that layer kind has
    /// one — `Base`/`Insert` only; the four minibuf mode layers do not. The
    /// SSOT `show_popup` gates a `Sticky` popup against, rather than
    /// re-listing which mode kinds may hold one.
    pub(in crate::editor) fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupModel>> {
        let mode_depth = self.mode_layer().depth;
        match &mut self.layers[mode_depth].1 {
            InputLayer::Base { sticky_popup, .. } | InputLayer::Insert { sticky_popup } => {
                Some(sticky_popup)
            }
            _ => None,
        }
    }

    /// Clears every home a popup could occupy: a `Popup` layer, if one is
    /// open (always `top()` — see `LayerKind::Popup`'s doc), and the current
    /// mode layer's sticky slot. Shared by `show_popup` (so `(show-popup!
    /// …)` replaces any popup already showing, regardless of which of the
    /// two homes it used — the documented "no stacking" contract) and
    /// `close_popup`, and called by `EditorState::push_mode_layer` and
    /// `open_picker` before they take over the stack, which is what keeps
    /// the "never buried" invariant true.
    pub(in crate::editor) fn clear_popups(&mut self) {
        if let Some(r) = self.ref_of(LayerKind::Popup) {
            debug_assert_eq!(
                r,
                self.top(),
                "a Popup layer is never buried: every opener that could land \
                 above it retires it first"
            );
            self.truncate(r);
        }
        if let Some(slot) = self.sticky_popup_slot_mut() {
            *slot = None;
        }
    }
}

impl Default for InputStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::overlay_models::{ConfirmAction, ConfirmChoice};
    use steel::rvals::SteelVal;

    fn menu(label: &str) -> InputLayer {
        InputLayer::Menu(MenuModel {
            rows: hume_ui::popup::MenuRows::measure(std::sync::Arc::new(vec![label.to_string()])),
            selected: 0,
            callback: SteelVal::BoolV(false),
        })
    }

    fn confirm() -> InputLayer {
        InputLayer::Confirm(ConfirmModel {
            prompt: "test?".to_string(),
            choices: vec![ConfirmChoice {
                key: 'y',
                label: "yes",
            }],
            action: ConfirmAction::ReloadBuffer(hume_engine::pipeline::BufferId::default()),
        })
    }

    fn popup_model(text: &str) -> PopupModel {
        PopupModel {
            text: text.to_string(),
            kind: hume_scripting::host::PopupKind::Scrollable,
            scroll: 0,
            syntax: None,
            layout: hume_ui::popup::PopupLayout::Cursor,
            content: None,
        }
    }

    #[test]
    fn truncate_refuses_base_and_returns_empty() {
        let mut stack = InputStack::new();
        let base = stack.top();
        let removed = stack.truncate(base);
        assert!(removed.is_empty());
        assert_eq!(stack.kind(stack.top()), Some(LayerKind::Base));
    }

    #[test]
    fn truncate_to_base_keeps_index_zero() {
        let mut stack = InputStack::new();
        stack.push(menu("m"));
        stack.push(confirm());
        let removed = stack.truncate_to_base();
        assert_eq!(removed.len(), 2);
        assert_eq!(stack.top().depth, 0);
        assert_eq!(stack.kind(stack.top()), Some(LayerKind::Base));
    }

    #[test]
    fn truncate_returns_top_first() {
        let mut stack = InputStack::new();
        stack.push(menu("bottom"));
        let r = stack.top();
        stack.push(confirm());
        let removed = stack.truncate(r);
        assert_eq!(removed.len(), 2);
        assert!(matches!(removed[0], InputLayer::Confirm(_)));
        assert!(matches!(removed[1], InputLayer::Menu(_)));
    }

    #[test]
    fn layer_ref_is_stale_after_truncate_then_push_at_same_depth() {
        let mut stack = InputStack::new();
        let first = stack.push(menu("first"));
        stack.truncate(first);
        let second = stack.push(menu("second"));

        assert_eq!(first.depth, second.depth, "same index, different id");
        assert!(!stack.is_live(first), "stale ref must not read as live");
        assert!(stack.is_live(second));
    }

    #[test]
    fn ref_of_finds_topmost_of_kind() {
        let mut stack = InputStack::new();
        assert_eq!(stack.ref_of(LayerKind::Menu), None);
        let r = stack.push(menu("only"));
        assert_eq!(stack.ref_of(LayerKind::Menu), Some(r));
    }

    #[test]
    fn is_stack_settled_true_on_a_fresh_stack() {
        let stack = InputStack::new();
        assert!(stack.is_stack_settled());
    }

    #[test]
    fn is_stack_settled_false_with_an_overlay_above_the_mode_layer() {
        let mut stack = InputStack::new();
        stack.push(menu("m"));
        assert!(!stack.is_stack_settled());
    }

    #[test]
    fn is_stack_settled_true_again_once_the_overlay_is_truncated() {
        let mut stack = InputStack::new();
        let r = stack.push(menu("m"));
        stack.truncate(r);
        assert!(stack.is_stack_settled());
    }

    #[test]
    fn is_stack_settled_true_with_a_mode_layer_alone_on_top() {
        let mut stack = InputStack::new();
        stack.push(InputLayer::Insert { sticky_popup: None });
        assert!(stack.is_stack_settled());
    }

    #[test]
    fn fall_through_after_self_truncate_reaches_the_layer_below() {
        // `fall_through` itself lives on `Editor` (mappings/mod.rs) and
        // needs a whole editor to exercise end to end — this test pins the
        // `InputStack` half of its contract: `below(r)` stays addressable
        // by depth alone even after `r` itself was just removed, since
        // every index below `r.depth` never moved.
        let mut stack = InputStack::new();
        let base = stack.top();
        let r = stack.push(menu("m"));
        stack.truncate(r);
        assert_eq!(stack.below(r), base);
    }

    #[test]
    fn popup_prefers_a_popup_layer_over_a_lower_sticky_slot() {
        let mut stack = InputStack::new();
        *stack.sticky_popup_slot_mut().expect("Base has a slot") =
            Some(popup_model("sticky on base"));
        stack.push(InputLayer::Popup(popup_model("scrollable layer")));
        assert_eq!(
            stack.popup().map(|p| p.text.as_str()),
            Some("scrollable layer")
        );
    }

    #[test]
    fn popup_reads_only_the_current_mode_layers_slot_not_a_buried_one() {
        // Base's own slot holding a value must not leak through once a
        // different mode layer is on top — `push_mode_layer` clears it
        // before taking over for exactly this reason; this pins the read
        // side independent of that write-time behavior.
        let mut stack = InputStack::new();
        *stack.sticky_popup_slot_mut().expect("Base has a slot") =
            Some(popup_model("buried on base"));
        stack.push(InputLayer::Insert { sticky_popup: None });
        assert!(stack.popup().is_none());
    }

    #[test]
    fn sticky_popup_slot_mut_is_none_under_a_minibuf_mode_layer() {
        let mut stack = InputStack::new();
        stack.push(InputLayer::Command {
            minibuf: MiniBuffer {
                prompt: String::new(),
                input: String::new(),
                cursor: 0,
            },
            completion: None,
        });
        assert!(stack.sticky_popup_slot_mut().is_none());
    }

    #[test]
    fn clear_popups_clears_both_a_popup_layer_and_the_current_slot() {
        let mut stack = InputStack::new();
        *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
        stack.push(InputLayer::Popup(popup_model("scrollable")));
        stack.clear_popups();
        assert!(stack.popup().is_none());
        assert_eq!(stack.kind(stack.top()), Some(LayerKind::Base));
    }

    #[test]
    fn truncate_to_base_clears_the_sticky_slot() {
        let mut stack = InputStack::new();
        *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
        stack.push(menu("m"));
        stack.truncate_to_base();
        assert!(stack.popup().is_none());
    }
}
