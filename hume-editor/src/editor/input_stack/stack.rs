//! The mechanism: [`InputStack`], [`LayerRef`], [`InputEvent`], and the
//! [`Layer`] trait every concrete layer implements. It names no layer in its
//! own API: every lookup downcasts through `dyn Layer`, so adding a layer
//! never touches this file.

use std::any::Any;

use termina::event::{KeyEvent, MouseEvent};

use hume_core::{PaneId, Severity};
use hume_engine::pipeline::EngineView;
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::EditorMode;

use super::super::edit_session::InsertSession;
use super::super::error::CommandError;
use super::super::event::mode_name;
use super::super::minibuf::MiniBuffer;
use super::super::{Editor, EditorState};
use super::base::BaseLayer;
use super::completion::{BufferCompletionLayer, MinibufCompletionLayer};
use super::paste::PastedText;
use super::popup::PopupLayer;

/// Addresses one layer by position, minted only by [`InputStack`]'s
/// lookups. `depth` alone would alias, since truncating and pushing puts a
/// different layer at the same index; `id` tells them apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) struct LayerRef {
    depth: usize,
    id: u64,
}

/// One key or mouse event working its way down the stack. Adding a variant
/// forces every layer's handler to state its policy for it at compile time.
/// A paste is not one: it takes [`Layer::paste_handler`]'s route, which needs
/// only the editor state, so code holding no `Editor` can paste too.
pub(in crate::editor) enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

/// The function that handles one event at a layer. A fn pointer rather than
/// a method on [`Layer`]: the handler needs `&mut Editor`, which owns the
/// stack this layer sits in, so a receiver borrowing the layer would alias
/// it.
pub(in crate::editor) type LayerHandler = fn(&mut Editor, LayerRef, InputEvent);

/// The function that lands a paste at a layer: [`LayerHandler`]'s
/// counterpart for a paste, a fn pointer for the same reason. `Err` when
/// the paste was refused, for the entry point to report or return.
pub(in crate::editor) type PasteHandler =
    fn(&mut EditorState, &EngineView, LayerRef, PastedText) -> Result<(), CommandError>;

/// Which popup homes landing a layer evicts, read by
/// [`EditorState::push_layer`] after [`Layer::setup`]. A completion layer
/// evicts only the pushed-layer home, so a `Sticky` signature-help popup in
/// the mode layer's slot stays beside the completion menu.
pub(in crate::editor) enum PopupEviction {
    /// [`InputStack::clear_popups`]: a pushed `PopupLayer`, if any, and the
    /// current mode layer's sticky slot.
    Both,
    /// [`InputStack::clear_popup_layer`]: a pushed `PopupLayer` alone,
    /// leaving a `Sticky` popup in the mode layer's own slot untouched.
    LayerOnly,
}

/// Why a layer is leaving the stack, as its caller states it:
/// [`EditorState::truncate_layers`] and [`EditorState::take_layer`] tell the
/// layer they were asked to remove apart from the collateral above it.
pub(in crate::editor) enum Removal {
    /// The layer the caller asked to remove, or the outgoing mode layer
    /// [`EditorState::push_mode_layer`] replaces.
    Explicit,
    /// This layer merely sat above the one the caller asked to remove, and
    /// is being swept away as a side effect.
    Incidental,
}

/// What every concrete layer implements: its state, the mode it presents,
/// what happens when it enters and leaves the stack, and what handles one
/// event while it is the dispatch target. `handler`, `paste_handler` and
/// `mode` have no default, so no layer leaves its policy unstated.
pub(in crate::editor) trait Layer: Any {
    /// The function that handles one event while this layer is the dispatch
    /// target.
    fn handler(&self) -> LayerHandler;

    /// What a paste does while this layer is the dispatch target.
    fn paste_handler(&self) -> PasteHandler;

    /// The [`EditorMode`] this layer presents when it is a mode layer
    /// (`Base` and the five editing modes); `None` for a transient overlay.
    fn mode(&self) -> Option<EditorMode>;

    /// Runs before this layer lands on the stack, seeing what it lands on;
    /// [`EditorState::push_layer`] always calls it. `PickerLayer` dismisses
    /// an open completion session and replaces a live picker; `SearchLayer`
    /// and `SiftLayer` capture their pre-entry selection snapshot.
    fn setup(&mut self, _state: &mut EditorState, _view: &EngineView) {}

    /// Runs when this layer leaves the stack, for any reason, seeing what is
    /// left: teardown of a mode layer is its cancel. Most overrides fire no
    /// Steel callback, since an explicit accept or cancel takes the layer
    /// through `EditorState::take_layer` and fires its own. `MenuLayer` and
    /// `DrawerLayer` read `why` to fire their callback when swept up as
    /// collateral; `PickerLayer` and `PromptLayer` fire unconditionally,
    /// since nothing has fired theirs when they reach here.
    fn tear_down(self: Box<Self>, _state: &mut EditorState, _view: &EngineView, _why: Removal) {}

    /// The pane snapshot this layer restores on exit, so a text change can
    /// carry it. Every layer answers, so one that keeps a snapshot cannot
    /// leave it out of the carry and restore it against another text.
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot>;

    /// The minibuffer this layer owns, if it is a minibuf-backed mode layer
    /// (`Command`/`Search`/`Sift`/`Prompt`) or the `ShellPrompt` overlay. At
    /// most one layer on the stack returns `Some`.
    fn minibuf(&self) -> Option<&MiniBuffer> {
        None
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        None
    }

    /// Whether an async opener's staleness check
    /// ([`InputStack::is_settled_for`]) counts this layer as the stack having
    /// moved. `DrawerLayer` and `PopupLayer` answer `false`: a stray key
    /// falls through the drawer, and a popup owns nothing but Ctrl-u/d.
    fn is_modal(&self) -> bool {
        true
    }

    /// Which popup homes [`EditorState::push_layer`] evicts on this layer's
    /// behalf.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::Both
    }

    /// What [`EditorState::retire`] does to collateral above this layer.
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::Stack
    }

    /// The sticky-popup slot this layer owns, if it is `Base` or `Insert`:
    /// the home of a `Sticky` popup (signature help), which belongs to the
    /// mode rather than to a layer of its own.
    fn sticky_popup_slot(&self) -> Option<&Option<PopupLayer>> {
        None
    }
    fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        None
    }

    /// Rebuilds or drops whatever this layer built against the outgoing
    /// theme's colors, against `theme`. By default that is the popup in its
    /// sticky slot, if any.
    fn theme_changed(&mut self, theme: &mut Theme, registry: &ScopeRegistry) {
        if let Some(Some(popup)) = self.sticky_popup_slot_mut() {
            popup.theme_changed(theme, registry);
        }
    }
}

/// A layer whose self-replace path takes the outgoing layer by value and
/// fires its callback with `#f`. `take_layer` never tears down its own
/// target, so this cannot double the `Removal::Incidental` fire in
/// `tear_down`. Implemented by `MenuLayer` and `DrawerLayer`.
pub(in crate::editor) trait FiresFalseOnReplace: Layer {
    /// Queue the layer's closing call with `#f`, if it has a Steel owner to
    /// tell.
    fn close_firing_false(self: Box<Self>, state: &mut EditorState);
}

/// What retiring a layer ([`EditorState::retire`]) does to whatever sits
/// above it. [`Stack`](Self::Stack), the default, takes the collateral
/// along, for layers that depend on the target being open.
/// [`SelfOnly`](Self::SelfOnly) removes only the target, for a widget other
/// layers are routinely stacked over by coincidence: `DrawerLayer` (an
/// `Insert` session or a code-action `Menu` often sits above it) and
/// `MenuLayer` (a non-modal `Popup` can land above it).
pub(in crate::editor) enum RemovalScope {
    Stack,
    SelfOnly,
}

// Downcasting through `Layer: Any`, as inherent methods on `dyn Layer` so no
// layer writes it by hand.
impl dyn Layer {
    pub(in crate::editor) fn is<L: Layer>(&self) -> bool {
        let any: &dyn Any = self;
        any.is::<L>()
    }

    pub(in crate::editor) fn downcast_ref<L: Layer>(&self) -> Option<&L> {
        let any: &dyn Any = self;
        any.downcast_ref()
    }

    pub(in crate::editor) fn downcast_mut<L: Layer>(&mut self) -> Option<&mut L> {
        let any: &mut dyn Any = self;
        any.downcast_mut()
    }

    /// Downcast an owned trait object to its concrete type. Every caller
    /// already knows `L` and treats a mismatch as a bug.
    pub(in crate::editor) fn downcast<L: Layer>(self: Box<Self>) -> Option<Box<L>> {
        let any: Box<dyn Any> = self;
        any.downcast().ok()
    }
}

// ── The stack ────────────────────────────────────────────────────────────

/// The stack: `Base` at index 0, always, plus whatever layers are pushed
/// above it.
///
/// There is no `pop()`, and no `get`/`get_mut` returning a bare
/// `&mut dyn Layer`: a payload is reached only through a typed lookup
/// (`find`, `at`). Structure changes only through [`Self::push`],
/// [`Self::truncate`] and [`Self::excise`], narrowed to this module so the
/// `EditorState` methods that run `Layer::setup` and `Layer::tear_down`
/// ([`EditorState::push_layer`], [`EditorState::truncate_layers`], …) are
/// their only callers. `truncate_to_base` drops every callback without
/// teardown and is reached only through [`EditorState::reset_input`].
pub(in crate::editor) struct InputStack {
    /// Each entry's id lets `is_live` tell a stale `LayerRef` from a new
    /// layer at the same index.
    layers: Vec<(u64, Box<dyn Layer>)>,
    next_id: u64,
}

/// Every layer on the stack but the Insert layer, borrowed apart from the
/// Insert session that layer holds ([`InputStack::split_insert`]).
pub(in crate::editor) struct OtherLayers<'a> {
    below: &'a mut [(u64, Box<dyn Layer>)],
    above: &'a mut [(u64, Box<dyn Layer>)],
}

impl OtherLayers<'_> {
    fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Box<dyn Layer>> {
        self.below
            .iter_mut()
            .chain(self.above.iter_mut())
            .map(|(_, layer)| layer)
    }

    /// Every one of these layers' pane snapshots.
    pub(in crate::editor) fn snapshots_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut super::PaneSnapshot> {
        self.iter_mut().filter_map(|layer| layer.snapshot_mut())
    }

    /// The topmost of these layers of concrete type `L`.
    pub(in crate::editor) fn find_mut<L: Layer>(&mut self) -> Option<&mut L> {
        self.iter_mut().rev().find_map(|layer| layer.downcast_mut())
    }
}

impl InputStack {
    pub(in crate::editor) fn new() -> Self {
        Self {
            layers: vec![(
                0,
                Box::new(BaseLayer {
                    extend: false,
                    sticky_popup: None,
                }),
            )],
            next_id: 1,
        }
    }

    /// The open Insert session, if any, and every other layer, borrowed
    /// apart: what a text change carries positions through.
    pub(in crate::editor) fn split_insert(
        &mut self,
    ) -> (Option<&mut InsertSession>, OtherLayers<'_>) {
        let Some(at) = self
            .layers
            .iter()
            .position(|(_, layer)| layer.is::<super::InsertLayer>())
        else {
            return (
                None,
                OtherLayers {
                    below: &mut self.layers,
                    above: &mut [],
                },
            );
        };
        let (below, rest) = self.layers.split_at_mut(at);
        let ((_, insert), above) = rest.split_first_mut().expect("the Insert layer is at `at`");
        let session = &mut insert
            .downcast_mut::<super::InsertLayer>()
            .expect("the layer at `at` is the Insert layer")
            .session;
        (Some(session), OtherLayers { below, above })
    }

    /// The topmost layer. Every key dispatch starts here.
    pub(in crate::editor) fn top(&self) -> LayerRef {
        let depth = self.layers.len() - 1;
        LayerRef {
            depth,
            id: self.layers[depth].0,
        }
    }

    /// Whether `r` still names the layer it was minted against: `false`
    /// once that index has been truncated, even if a later `push` refilled
    /// it.
    pub(in crate::editor) fn is_live(&self, r: LayerRef) -> bool {
        self.live(r).is_some()
    }

    /// The layer `r` names, or `None` if `r` is stale.
    fn live(&self, r: LayerRef) -> Option<&(dyn Layer + 'static)> {
        self.layers
            .get(r.depth)
            .filter(|(id, _)| *id == r.id)
            .map(|(_, layer)| layer.as_ref())
    }

    /// Whether `r` is live and names a layer of concrete type `L`.
    pub(in crate::editor) fn is<L: Layer>(&self, r: LayerRef) -> bool {
        self.live(r).is_some_and(|layer| layer.is::<L>())
    }

    /// The layer at `r`, downcast to `L`: `None` if `r` is stale or names
    /// another type. A handler dispatched to `r` reaches its own payload
    /// this way instead of searching the stack by type.
    pub(in crate::editor) fn at<L: Layer>(&self, r: LayerRef) -> Option<&L> {
        self.live(r).and_then(|layer| layer.downcast_ref())
    }

    pub(in crate::editor) fn at_mut<L: Layer>(&mut self, r: LayerRef) -> Option<&mut L> {
        self.layers
            .get_mut(r.depth)
            .filter(|(id, _)| *id == r.id)
            .and_then(|(_, layer)| layer.downcast_mut())
    }

    /// `r`'s handler, or `None` if `r` is stale.
    pub(in crate::editor) fn handler(&self, r: LayerRef) -> Option<LayerHandler> {
        self.live(r).map(|layer| layer.handler())
    }

    /// `r`'s paste handler, or `None` if `r` is stale.
    pub(in crate::editor) fn paste_handler(&self, r: LayerRef) -> Option<PasteHandler> {
        self.live(r).map(|layer| layer.paste_handler())
    }

    /// The topmost layer of concrete type `L`, if one is open: how a
    /// `close-*!` builtin or an internal retirement names a widget it did
    /// not just push. Every type but `BaseLayer` occurs at most once on the
    /// stack.
    pub(in crate::editor) fn ref_of<L: Layer>(&self) -> Option<LayerRef> {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.is::<L>())
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
    }

    /// The topmost layer whose pane snapshot was taken in `pane`.
    pub(in crate::editor) fn topmost_bound_to(&mut self, pane: PaneId) -> Option<LayerRef> {
        let depth = self
            .layers
            .iter_mut()
            .rposition(|(_, layer)| layer.snapshot_mut().is_some_and(|snap| snap.pane() == pane))?;
        Some(LayerRef {
            depth,
            id: self.layers[depth].0,
        })
    }

    /// The topmost layer of concrete type `L`. The named lookups (`menu()`,
    /// `picker()`, …) are sugar over it.
    pub(in crate::editor) fn find<L: Layer>(&self) -> Option<&L> {
        self.layers
            .iter()
            .rev()
            .find_map(|(_, layer)| layer.downcast_ref())
    }

    pub(in crate::editor) fn find_mut<L: Layer>(&mut self) -> Option<&mut L> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| layer.downcast_mut())
    }

    /// The layer directly below `r`. Panics if `r` is `Base`; the caller
    /// checks `r.depth >= 1` first.
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

    /// Whether nothing modal ([`Layer::is_modal`]) other than a layer of
    /// type `L` sits above the current mode layer: the staleness check an
    /// async opener (`show-menu!`, `show-drawer-list!`) makes before
    /// landing, since the user may have opened something else while the
    /// request was in flight. A synchronous opener needs no check: dispatch
    /// order already proves where the stack is.
    ///
    /// `L` lets a self-replacing opener tolerate its own prior instance,
    /// even one buried under a later non-modal overlay such as a `Popup`.
    pub(in crate::editor) fn is_settled_for<L: Layer>(&self) -> bool {
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth + 1..]
            .iter()
            .all(|(_, layer)| !layer.is_modal() || layer.is::<L>())
    }

    /// Pushes `layer` on top, unconditionally: precedence is push order.
    /// [`EditorState::push_layer`], the only caller, runs `Layer::setup`
    /// first.
    pub(in crate::editor::input_stack) fn push<L: Layer>(&mut self, layer: L) -> LayerRef {
        let id = self.next_id;
        self.next_id += 1;
        self.layers.push((id, Box::new(layer)));
        LayerRef {
            depth: self.layers.len() - 1,
            id,
        }
    }

    /// Removes `r` and everything above it, returning the removed layers
    /// top-first, so `r`'s own layer is last. Empty when `r` is stale or is
    /// `Base`, which is never removed. Runs no `Layer::tear_down`: its
    /// callers do, except [`Self::clear_popup_layer`], whose one removed
    /// layer has an empty teardown.
    pub(in crate::editor::input_stack) fn truncate(&mut self, r: LayerRef) -> Vec<Box<dyn Layer>> {
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

    /// Removes only `r`, leaving every layer above it in place: for a
    /// `Confirm` that no longer targets anything live, or a drawer, with an
    /// unrelated layer stacked above it. Runs no `Layer::tear_down`, like
    /// [`Self::truncate`]. `None` when `r` is stale or is `Base`.
    pub(in crate::editor::input_stack) fn excise(&mut self, r: LayerRef) -> Option<Box<dyn Layer>> {
        if r.depth == 0 || !self.is_live(r) {
            return None;
        }
        Some(self.layers.remove(r.depth).1)
    }

    /// Removes every layer above `Base` without its teardown, discarding any
    /// Steel callback, and resets `Base`, so a reload leaves neither Extend
    /// on nor a signature-help popup up. Reached only through
    /// [`EditorState::reset_input`], which first ends the Insert session
    /// that dropping its layer would leave uncommitted.
    fn truncate_to_base(&mut self) {
        self.layers.truncate(1);
        self.layers[0].1 = Box::new(BaseLayer {
            extend: false,
            sticky_popup: None,
        });
    }

    /// The layer that determines `EditorMode`: the topmost mode layer,
    /// `Base` or the one mode layer pushed above it. An overlay never
    /// changes the mode beneath it.
    pub(in crate::editor) fn mode_layer(&self) -> LayerRef {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.mode().is_some())
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
            .expect("Base is always a mode layer and always on the stack")
    }

    /// The `EditorMode` the current mode layer maps to.
    pub(in crate::editor) fn mode(&self) -> EditorMode {
        let r = self.mode_layer();
        self.layers[r.depth]
            .1
            .mode()
            .expect("mode_layer() always names a layer whose mode() is Some")
    }

    /// Sets `Base`'s `extend` flag, without gating on the current mode
    /// layer.
    ///
    /// Clears any open popup when the flag flips, since the Normal/Extend
    /// switch does not go through `push_mode_layer`'s eviction. A same-value
    /// call leaves an unrelated popup alone.
    pub(in crate::editor) fn set_extend(&mut self, extend: bool) {
        let base = self.layers[0]
            .1
            .downcast_mut::<BaseLayer>()
            .expect("index 0 is always Base");
        if base.extend == extend {
            return;
        }
        base.extend = extend;
        self.clear_popups();
    }

    /// The active minibuffer, from a minibuf-backed mode layer or the
    /// `ShellPrompt` overlay. [`EditorState::push_layer`] retires an open
    /// one before pushing another, so at most one is on the stack.
    pub(in crate::editor) fn minibuf(&self) -> Option<&MiniBuffer> {
        self.layers
            .iter()
            .rev()
            .find_map(|(_, layer)| layer.minibuf())
    }

    /// The layer holding the open minibuffer, if any.
    fn minibuf_owner(&self) -> Option<LayerRef> {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.minibuf().is_some())
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
    }

    pub(in crate::editor) fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| layer.minibuf_mut())
    }

    /// The minibuffer owned by `top()`, skipping completion layers, which
    /// draw above the command line without replacing it. `None` when a
    /// minibuf-mode layer is buried under a picker or menu. A layer's own
    /// handler can use [`Self::minibuf`], since it runs only while on top;
    /// every other reader goes through [`EditorState::minibuf`].
    pub(in crate::editor) fn top_minibuf(&self) -> Option<&MiniBuffer> {
        self.layers
            .iter()
            .rev()
            .find(|(_, layer)| {
                !layer.is::<BufferCompletionLayer>() && !layer.is::<MinibufCompletionLayer>()
            })
            .and_then(|(_, layer)| layer.minibuf())
    }

    /// The active popup, in either home: a pushed [`PopupLayer`], checked
    /// first since it is never buried, or the current mode layer's sticky
    /// slot. Slots below the current mode layer are not read.
    pub(in crate::editor) fn popup(&self) -> Option<&PopupLayer> {
        if let Some(popup) = self.find::<PopupLayer>() {
            return Some(popup);
        }
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth]
            .1
            .sticky_popup_slot()
            .and_then(|slot| slot.as_ref())
    }

    pub(in crate::editor) fn popup_mut(&mut self) -> Option<&mut PopupLayer> {
        if let Some(r) = self.ref_of::<PopupLayer>() {
            return self.layers[r.depth].1.downcast_mut::<PopupLayer>();
        }
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth]
            .1
            .sticky_popup_slot_mut()
            .and_then(|slot| slot.as_mut())
    }

    /// The current mode layer's sticky-popup slot, if that layer has one
    /// (`Base`, `Insert`): what `show_popup` gates a `Sticky` popup on.
    pub(in crate::editor) fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth].1.sticky_popup_slot_mut()
    }

    /// Clears a pushed [`PopupLayer`], always `top()`, leaving the mode
    /// layer's sticky slot alone: a completion session evicts a `Scrollable`
    /// popup (hover, the diagnostic overlay) but keeps signature help.
    /// Truncates without teardown, which is sound because a `PopupLayer`'s
    /// teardown is empty.
    pub(in crate::editor) fn clear_popup_layer(&mut self) {
        if let Some(r) = self.ref_of::<PopupLayer>() {
            debug_assert_eq!(
                r,
                self.top(),
                "a Popup layer is never buried: every opener that could land \
                 above it retires it first"
            );
            self.truncate(r);
        }
    }

    /// Runs [`Layer::theme_changed`] on every layer, for a change to `theme`.
    pub(in crate::editor) fn theme_changed(&mut self, theme: &mut Theme, registry: &ScopeRegistry) {
        for (_, layer) in &mut self.layers {
            layer.theme_changed(theme, registry);
        }
    }

    /// Clears both popup homes. [`EditorState::push_layer`] calls it for
    /// every layer whose [`Layer::popup_eviction`] is
    /// [`PopupEviction::Both`]; the other callers are `show_popup`'s
    /// `Sticky` arm, so a new popup replaces one in either home, and
    /// `close_popup`.
    pub(in crate::editor) fn clear_popups(&mut self) {
        self.clear_popup_layer();
        if let Some(slot) = self.sticky_popup_slot_mut() {
            *slot = None;
        }
    }
}

impl EditorState {
    /// Pushes a mode layer: the single write path for mode transitions.
    /// `OnModeChange` fires later, from `Editor::detect_mode_change` at the
    /// next `settle()`.
    ///
    /// A same-kind re-entry replaces the layer, since its payload may
    /// differ. An Insert layer is never re-entered this way: one is built
    /// only from a freshly opened session, and `begin_insert_session` joins
    /// an open one instead.
    ///
    /// Tears down the current mode layer unless it is `Base`
    /// (teardown is cancel), clears Extend, and calls [`Self::push_layer`].
    /// Overlays below the outgoing mode layer stay, except one holding a
    /// minibuffer, which `push_layer` retires. `push_layer` evicts any popup
    /// in `Base`'s slot, so a popup is never buried.
    ///
    /// Never gated: a key only reaches `Base` after every overlay above it
    /// fell through, and a Steel-initiated push (a timer's `prompt!`) lands on
    /// top of an open picker, which stays suspended until the layer retires.
    pub(in crate::editor) fn push_mode_layer<L: Layer>(&mut self, view: &EngineView, layer: L) {
        let mode_layer = self.input.mode_layer();
        if !self.input.is::<super::BaseLayer>(mode_layer) {
            self.truncate_layers(view, mode_layer);
        }
        self.input.set_extend(false);
        self.push_layer(view, layer);
    }

    /// Ends the open Insert or paste session (`focus::end_focus_sessions`),
    /// then drops every layer above `Base` without its teardown and resets
    /// `Base` ([`InputStack::truncate_to_base`]): a config reload's reset of
    /// the input stack, while the outgoing config is still installed, since
    /// the Insert teardown's own edit reaches hooks and language servers.
    pub(in crate::editor) fn reset_input(&mut self, view: &EngineView) {
        crate::editor::focus::end_focus_sessions(self, view);
        self.input.truncate_to_base();
    }

    /// The only way to push a layer: [`Layer::setup`] runs first, seeing the
    /// stack it lands on, then popup eviction ([`Layer::popup_eviction`]),
    /// after `setup` so `PickerLayer`'s dismiss-and-replace runs first. A
    /// layer holding a minibuffer, and a mode layer, first retire the open
    /// minibuffer: keys never reach a buried one, and none stays drawn over
    /// a mode that does not read it.
    pub(in crate::editor) fn push_layer<L: Layer>(
        &mut self,
        view: &EngineView,
        mut layer: L,
    ) -> LayerRef {
        if (layer.minibuf().is_some() || layer.mode().is_some())
            && let Some(open) = self.input.minibuf_owner()
        {
            self.retire_at(view, open);
        }
        layer.setup(self, view);
        match layer.popup_eviction() {
            PopupEviction::Both => self.input.clear_popups(),
            PopupEviction::LayerOnly => self.input.clear_popup_layer(),
        }
        self.input.push(layer)
    }

    /// Removes `r` and everything above it, running each removed layer's
    /// [`Layer::tear_down`] top-first: the one teardown path for a
    /// mode-layer exit and an overlay retirement. `r`'s own layer gets
    /// [`Removal::Explicit`], the collateral above it
    /// [`Removal::Incidental`]. An accept arm does its own work before
    /// truncating, so every teardown here is the cancel case.
    pub(in crate::editor) fn truncate_layers(&mut self, view: &EngineView, r: LayerRef) {
        let mut removed = self.input.truncate(r);
        if let Some(target) = removed.pop() {
            for layer in removed {
                layer.tear_down(self, view, Removal::Incidental);
            }
            target.tear_down(self, view, Removal::Explicit);
        }
    }

    /// [`Self::truncate_layers`] for a caller that needs `r`'s own layer by
    /// value, to read its payload before acting on it (a menu's chosen
    /// index, a picker's selection). Layers above `r` get their ordinary
    /// teardown; `r`'s own layer gets none, so its accept work runs once.
    pub(in crate::editor) fn take_layer<L: Layer>(
        &mut self,
        view: &EngineView,
        r: LayerRef,
    ) -> Box<L> {
        let mut removed = self.input.truncate(r);
        let taken = removed
            .pop()
            .expect("r names a live layer, so truncate(r) removes at least one")
            .downcast::<L>()
            .expect("caller names r's own concrete type");
        for layer in removed {
            layer.tear_down(self, view, Removal::Incidental);
        }
        taken
    }

    /// Takes `r`'s layer by value and fires its callback with `#f`: the
    /// self-replace path of `show_menu`/`show_drawer_list`, so the outgoing
    /// owner learns its widget is gone, and the drawer's `Esc` arm. An
    /// explicit `close-*!` stays silent by going through [`Self::retire`].
    pub(in crate::editor) fn take_firing_false<L: FiresFalseOnReplace>(
        &mut self,
        view: &EngineView,
        r: LayerRef,
    ) {
        let old: Box<L> = self.take_layer(view, r);
        old.close_firing_false(self);
    }

    /// Removes exactly `r` via [`InputStack::excise`], running its own
    /// `tear_down` but leaving everything stacked above it untouched.
    /// [`Self::retire`] calls this itself for a [`RemovalScope::SelfOnly`]
    /// layer. A no-op when `r` is already stale.
    pub(in crate::editor) fn excise_layer(&mut self, view: &EngineView, r: LayerRef) {
        if let Some(layer) = self.input.excise(r) {
            layer.tear_down(self, view, Removal::Explicit);
        }
    }

    /// Retires the topmost layer of type `L`, if one is open, by its own
    /// [`Layer::removal_scope`]: [`Self::truncate_layers`] when what is above
    /// depends on it, [`Self::excise_layer`] when it is stacked over by
    /// coincidence.
    pub(in crate::editor) fn retire<L: Layer>(&mut self, view: &EngineView) {
        if let Some(r) = self.input.ref_of::<L>() {
            self.retire_at(view, r);
        }
    }

    /// Retires every layer whose pane snapshot was taken in `pane`, each by
    /// its own [`Layer::removal_scope`]: a closing pane takes its search
    /// or sift session with it. Runs while `pane`'s state still exists, so
    /// each layer's `tear_down` finds the pane it restores into.
    pub(in crate::editor) fn retire_bound_to(&mut self, view: &EngineView, pane: PaneId) {
        while let Some(r) = self.input.topmost_bound_to(pane) {
            self.retire_at(view, r);
        }
    }

    /// Retires the live layer at `r` by its own [`Layer::removal_scope`].
    fn retire_at(&mut self, view: &EngineView, r: LayerRef) {
        let scope = self
            .input
            .live(r)
            .map(|layer| layer.removal_scope())
            .expect("retire_at names a live layer");
        match scope {
            RemovalScope::Stack => self.truncate_layers(view, r),
            RemovalScope::SelfOnly => self.excise_layer(view, r),
        }
    }

    /// The staleness gate for an opener whose Steel callback fires after the
    /// key path that triggered it returned (`show-menu!`,
    /// `show-drawer-list!`). `M` is the mode layer the request requires, `L`
    /// the overlay it is about to push. `true` when the request should be
    /// dropped, for one of two reasons:
    ///
    /// - The mode layer is not `M`: the user left the mode, or a command ran
    ///   from the wrong one. Reported at `Severity::Warning`, since the
    ///   caller asked for something it cannot have.
    /// - A modal overlay landed above `M` since the request went out, an
    ///   ordinary race reported at `Severity::Trace`. A prior instance of
    ///   `L` does not count; the caller replaces it itself.
    pub(in crate::editor) fn async_opener_stale<M: Layer, L: Layer>(&mut self, what: &str) -> bool {
        if !self.input.is::<M>(self.input.mode_layer()) {
            self.report(
                Severity::Warning,
                format!(
                    "{what}: refused, not usable while in {} mode",
                    mode_name(self.mode())
                ),
            );
            return true;
        }
        let stale = !self.input.is_settled_for::<L>();
        if stale {
            self.report(
                Severity::Trace,
                format!("{what}: the stack moved before it could open; ignored"),
            );
        }
        stale
    }
}

impl Default for InputStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
