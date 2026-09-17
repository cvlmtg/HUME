//! The mechanism: [`InputStack`], [`LayerRef`], [`InputEvent`], and the
//! [`Layer`] trait every concrete layer implements. Deliberately names no
//! layer in its own API — see this module's own doc for why.
//!
//! Storage is `Vec<(u64, Box<dyn Layer>)>` rather than `Vec<InputLayer>` for
//! a closed enum: every generic lookup below (`find`, `find_mut`, `ref_of`,
//! `is`) downcasts through `dyn Layer`'s own `is`/`downcast_ref`/
//! `downcast_mut`/`downcast` (below `Layer`'s own definition) rather than
//! matching a variant, so adding a layer never touches this file.
//!
//! Eleven of the twelve layers have already moved into files of their own
//! (`base.rs`, `insert.rs`, `command.rs`, `search.rs`, `sift.rs`,
//! `prompt.rs`, `popup.rs`, `menu.rs`, `drawer.rs`, `confirm.rs`,
//! `completion.rs`). `Picker` alone still lives in *this* file for now —
//! its own move (alongside `editor/picker.rs`) is the remaining step.
//! `dispatch_at` (`mappings/mod.rs`) still calls its handler directly by
//! name during that transition; `Layer` gains a `handler()` method (a
//! `LayerHandler` fn pointer, replacing the chain) only once `Picker`'s
//! `impl` is co-located with its own handler fn too — doing that before
//! then would need the not-yet-moved handler widened to
//! `pub(in crate::editor)`, reachable by any code in the crate rather than
//! only through `dispatch_at`'s liveness-checked call.

use std::any::Any;

use termina::event::{KeyEvent, MouseEvent};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::EditorState;
use super::super::minibuf::MiniBuffer;
use super::super::picker::PickerSession;
use super::base::BaseLayer;
use super::popup::PopupLayer;

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

/// What every concrete layer implements — its state, what mode (if any) it
/// presents, and what happens when it leaves the stack. `mode`/`tear_down`
/// are required, no default: a layer that hasn't stated both hasn't stated
/// its policy, mirroring the closed `enum` this trait replaces, whose every
/// variant forced a match arm everywhere `kind()`/`tear_down` touched it.
///
/// A `handler` method (a `LayerHandler` fn pointer) joins this trait once a
/// layer's own file exists — see this module's own doc for why it isn't
/// here yet.
pub(in crate::editor) trait Layer: Any {
    /// `Some` naming the [`EditorMode`] this layer presents when it is a
    /// *mode* layer (`Base`, and the five editing modes); `None` for a
    /// transient overlay. Single source of both `InputStack::mode()` and
    /// "is this a mode layer" — the closed `enum`'s separate
    /// `is_mode_layer()` is gone because nothing needs it apart from this.
    fn mode(&self) -> Option<EditorMode>;

    /// What happens when this layer leaves the stack, for any reason —
    /// teardown of a mode layer *is* its cancel (`SPEC.md` D9). Never fires
    /// a Steel callback: those are queued only from explicit accept/cancel
    /// arms, before the truncate that reaches here.
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView);

    /// The minibuffer this layer owns, if it's one of the four
    /// minibuf-backed mode layers (`Command`/`Search`/`Sift`/`Prompt`).
    /// [`InputStack::minibuf`] walks topmost-first over this — at most one
    /// layer ever returns `Some`, so "topmost" and "only" coincide.
    fn minibuf(&self) -> Option<&MiniBuffer> {
        None
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        None
    }

    /// The sticky-popup slot this layer owns, if it's `Base` or `Insert` —
    /// the home for a `Sticky` popup (signature help), which belongs to
    /// whichever mode owns it rather than to its own layer. See
    /// [`InputStack::popup`]'s doc for the two homes a popup can occupy.
    fn sticky_popup_slot(&self) -> Option<&Option<PopupLayer>> {
        None
    }
    fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        None
    }
}

// Type-erasing plumbing behind `InputStack::find`/`find_mut`/`is` and the
// owned extraction `downcast` performs — inherent methods on `dyn Layer`
// itself, not trait methods, so no layer writes any of this by hand. Each
// coerces `&dyn Layer`/`&mut dyn Layer`/`Box<dyn Layer>` to its `dyn Any`
// counterpart first (sound because `Layer: Any`) and delegates to `Any`'s
// own downcasting — a default trait *method* body can't do this coercion
// itself (its `self` has no known size until a concrete `Self` is plugged
// in), but a free function taking the already-unsized `dyn Layer` can.
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
    /// already knows `L` from `ref_of::<L>()`/dispatch having named it, and
    /// `.expect()`s/`unreachable!()`s on mismatch — there is no "give the
    /// box back on failure" path any caller needs.
    pub(in crate::editor) fn downcast<L: Layer>(self: Box<Self>) -> Option<Box<L>> {
        let any: Box<dyn Any> = self;
        any.downcast().ok()
    }
}

// ── Layer types (temporary home — see this file's own doc) ─────────────────

/// A fuzzy-picker layer. Wraps `Box<PickerSession>`: `PickerSession` alone is
/// several times the size of every other layer's own state (its own
/// fuzzy-match scoring buffers, picked items, `#:actions` table, …); the
/// closed `enum` this replaces boxed it for the same reason
/// (`clippy::large_enum_variant`), sizing every *other* variant to the
/// picker's own footprint otherwise. Unlike the enum, `Box<dyn Layer>`
/// already indirects every layer uniformly, so this inner box becomes
/// redundant once `PickerSession` moves into its own file — dropped there,
/// not here, to keep this step a pure mechanism change.
pub(in crate::editor) struct PickerLayer(pub(in crate::editor) Box<PickerSession>);

impl Layer for PickerLayer {
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView) {
        // `PickerSession`'s own `Drop` kills a streaming source's child
        // process — nothing further to do here.
    }
}

// ── The stack ────────────────────────────────────────────────────────────

/// The stack itself: `Base` at index 0, always, plus whatever overlay
/// layers are pushed above it.
///
/// No `pop()`, and no `get(r)`/`get_mut(r)` handing back a bare
/// `&mut dyn Layer`: either would let a caller overwrite a layer in place
/// or remove one that isn't its own, bypassing the ordering this type
/// exists to enforce. Structure changes only through [`Self::push`],
/// [`Self::truncate`], and [`Self::truncate_to_base`]; a payload is only
/// ever reached through a typed lookup (`find`, or a layer's own named
/// sugar over it).
pub(in crate::editor) struct InputStack {
    /// Paired with a monotonic id per entry (see `LayerRef`) rather than a
    /// bare `Vec<Box<dyn Layer>>` — the id is what lets `is_live` tell a
    /// stale `LayerRef` from a fresh layer that landed at the same index.
    layers: Vec<(u64, Box<dyn Layer>)>,
    next_id: u64,
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
        self.layers.get(r.depth).is_some_and(|(id, _)| *id == r.id)
    }

    /// Whether `r` is live *and* names a layer of concrete type `L` — the
    /// generic replacement for comparing a closed `enum`'s discriminant.
    pub(in crate::editor) fn is<L: Layer>(&self, r: LayerRef) -> bool {
        self.layers
            .get(r.depth)
            .filter(|(id, _)| *id == r.id)
            .is_some_and(|(_, layer)| layer.is::<L>())
    }

    /// The topmost layer of concrete type `L`, if one is open — the ref a
    /// `close-*!` builtin or a Rust-internal retirement needs to name a
    /// widget it didn't itself just push. `truncate` is the only removal
    /// op, so closing a widget that isn't on top would take everything
    /// above it with it — every `close-*!` builtin instead errors when
    /// `ref_of` finds the widget present but not `top()` (every
    /// `confirm`-retirement site on buffer close/focus change reads this
    /// the same way). Every type but `BaseLayer` occurs at most once on the
    /// stack today, so "topmost" and "only" coincide in practice.
    pub(in crate::editor) fn ref_of<L: Layer>(&self) -> Option<LayerRef> {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, layer))| layer.is::<L>())
            .map(|(depth, (id, _))| LayerRef { depth, id: *id })
    }

    /// The topmost layer of concrete type `L`, its payload — the read half
    /// of every named lookup (`InputStack::menu()`, `InputStack::picker()`,
    /// …), which is one-line sugar over this.
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
    pub(in crate::editor) fn push<L: Layer>(&mut self, layer: L) -> LayerRef {
        let id = self.next_id;
        self.next_id += 1;
        self.layers.push((id, Box::new(layer)));
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
    pub(in crate::editor) fn truncate(&mut self, r: LayerRef) -> Vec<Box<dyn Layer>> {
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
    /// [`Self::truncate`], and resets `Base` itself to a fresh, all-default
    /// [`BaseLayer`] — a reload must not leave Extend on, or a
    /// signature-help popup visible, for hooks that never saw either turned
    /// on.
    pub(in crate::editor) fn truncate_to_base(&mut self) -> Vec<Box<dyn Layer>> {
        let removed = self
            .layers
            .split_off(1)
            .into_iter()
            .rev()
            .map(|(_, layer)| layer)
            .collect();
        self.layers[0].1 = Box::new(BaseLayer {
            extend: false,
            sticky_popup: None,
        });
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

    /// Sets `Base`'s `extend` flag directly — `Base` is always at index 0,
    /// so this never needs a lookup. Does not gate on the current mode
    /// layer or clear it on push; `push_mode_layer` clears it on every push
    /// separately, and a toggle reads `mode()` first to decide the target
    /// value.
    pub(in crate::editor) fn set_extend(&mut self, extend: bool) {
        let base = self.layers[0]
            .1
            .downcast_mut::<BaseLayer>()
            .expect("index 0 is always Base");
        base.extend = extend;
    }

    /// The active minibuffer, topmost-wins across the four minibuf-backed
    /// mode layers (`Command`/`Search`/`Sift`/`Prompt`) — at most one of
    /// them is ever on the stack, so "topmost" and "only" coincide.
    pub(in crate::editor) fn minibuf(&self) -> Option<&MiniBuffer> {
        self.layers
            .iter()
            .rev()
            .find_map(|(_, layer)| layer.minibuf())
    }

    pub(in crate::editor) fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        self.layers
            .iter_mut()
            .rev()
            .find_map(|(_, layer)| layer.minibuf_mut())
    }

    // ── Named lookup sugar ───────────────────────────────────────────────
    //
    // One-line wrappers over `find`/`find_mut` — kept by name (rather than
    // rewriting every `input.picker()`/`input.confirm()`/… call site to
    // `input.find::<PickerLayer>()`) so the ~350 existing call sites are
    // untouched by this refactor. Each of these moves into its own layer's
    // file alongside that layer's own `impl Layer` block — `picker()`/
    // `picker_mut()` stay here until `Picker`'s own move.

    pub(in crate::editor) fn picker(&self) -> Option<&PickerSession> {
        self.find::<PickerLayer>().map(|l| l.0.as_ref())
    }

    pub(in crate::editor) fn picker_mut(&mut self) -> Option<&mut PickerSession> {
        self.find_mut::<PickerLayer>().map(|l| l.0.as_mut())
    }

    /// The active popup, whichever of its two homes holds it: a
    /// [`PopupLayer`] layer (checked first — see its own "never buried"
    /// doc, which is what makes checking it independently of
    /// `mode_layer()` sound) or the *current* mode layer's own
    /// sticky-popup slot. Reading only the current mode layer's slot — not
    /// any slot buried below it — matters when `Base`'s slot holds a value
    /// that a later mode-layer push left behind: `push_mode_layer` clears
    /// it before taking over as mode layer for exactly this reason, but
    /// this lookup would still be wrong to read past `mode_layer()` even if
    /// it didn't.
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
        let has_popup_layer = self
            .layers
            .iter()
            .any(|(_, layer)| layer.is::<PopupLayer>());
        if has_popup_layer {
            return self.find_mut::<PopupLayer>();
        }
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth]
            .1
            .sticky_popup_slot_mut()
            .and_then(|slot| slot.as_mut())
    }

    /// The current mode layer's sticky-popup slot, if that layer kind has
    /// one — `Base`/`Insert` only; the four minibuf mode layers do not. The
    /// SSOT `show_popup` gates a `Sticky` popup against, rather than
    /// re-listing which mode kinds may hold one.
    pub(in crate::editor) fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth].1.sticky_popup_slot_mut()
    }

    /// Clears every home a popup could occupy: a [`PopupLayer`] layer, if
    /// one is open (always `top()` — see its own doc), and the current
    /// mode layer's sticky slot. Shared by `show_popup` (so `(show-popup!
    /// …)` replaces any popup already showing, regardless of which of the
    /// two homes it used — the documented "no stacking" contract) and
    /// `close_popup`, and called by `EditorState::push_mode_layer` and
    /// `open_picker` before they take over the stack, which is what keeps
    /// the "never buried" invariant true.
    pub(in crate::editor) fn clear_popups(&mut self) {
        if let Some(r) = self.ref_of::<PopupLayer>() {
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
    use crate::editor::input_stack::command::CommandLayer;
    use crate::editor::input_stack::confirm::{ConfirmAction, ConfirmChoice, ConfirmLayer};
    use crate::editor::input_stack::insert::InsertLayer;
    use crate::editor::input_stack::menu::MenuLayer;
    use steel::rvals::SteelVal;

    fn menu(label: &str) -> MenuLayer {
        MenuLayer {
            rows: hume_ui::popup::MenuRows::measure(std::sync::Arc::new(vec![label.to_string()])),
            selected: 0,
            callback: SteelVal::BoolV(false),
        }
    }

    fn confirm() -> ConfirmLayer {
        ConfirmLayer {
            prompt: "test?".to_string(),
            choices: vec![ConfirmChoice {
                key: 'y',
                label: "yes",
            }],
            action: ConfirmAction::ReloadBuffer(hume_engine::pipeline::BufferId::default()),
        }
    }

    fn popup_model(text: &str) -> PopupLayer {
        PopupLayer {
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
        assert!(stack.is::<BaseLayer>(stack.top()));
    }

    #[test]
    fn truncate_to_base_keeps_index_zero() {
        let mut stack = InputStack::new();
        stack.push(menu("m"));
        stack.push(confirm());
        let removed = stack.truncate_to_base();
        assert_eq!(removed.len(), 2);
        assert_eq!(stack.top().depth, 0);
        assert!(stack.is::<BaseLayer>(stack.top()));
    }

    #[test]
    fn truncate_returns_top_first() {
        let mut stack = InputStack::new();
        stack.push(menu("bottom"));
        let r = stack.top();
        stack.push(confirm());
        let removed = stack.truncate(r);
        assert_eq!(removed.len(), 2);
        assert!(removed[0].is::<ConfirmLayer>());
        assert!(removed[1].is::<MenuLayer>());
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
        assert_eq!(stack.ref_of::<MenuLayer>(), None);
        let r = stack.push(menu("only"));
        assert_eq!(stack.ref_of::<MenuLayer>(), Some(r));
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
        stack.push(InsertLayer { sticky_popup: None });
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
        stack.push(popup_model("scrollable layer"));
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
        stack.push(InsertLayer { sticky_popup: None });
        assert!(stack.popup().is_none());
    }

    #[test]
    fn sticky_popup_slot_mut_is_none_under_a_minibuf_mode_layer() {
        let mut stack = InputStack::new();
        stack.push(CommandLayer {
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
        stack.push(popup_model("scrollable"));
        stack.clear_popups();
        assert!(stack.popup().is_none());
        assert!(stack.is::<BaseLayer>(stack.top()));
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
