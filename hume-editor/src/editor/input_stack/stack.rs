//! The mechanism: [`InputStack`], [`LayerRef`], [`InputEvent`], and the
//! [`Layer`] trait every concrete layer implements. Deliberately names no
//! layer in its own API — see this module's own doc for why.
//!
//! Storage is `Vec<(u64, Box<dyn Layer>)>` rather than `Vec<InputLayer>` for
//! a closed enum: every generic lookup below (`find`, `find_mut`, `ref_of`,
//! `is`, `at`, `at_mut`) downcasts through `dyn Layer`'s own `is`/
//! `downcast_ref`/`downcast_mut`/`downcast` (below `Layer`'s own definition)
//! rather than matching a variant, so adding a layer never touches this
//! file.
//!
//! Every layer now has a file of its own — `Picker`'s move
//! (`input_stack/picker/`) was the last one — so `Layer::handler` is a
//! required method like `mode`/`tear_down`, and `dispatch_at`
//! (`mappings/mod.rs`) is the one-line vtable call the whole split was
//! building toward: `let f = input.handler(r); f(editor, r, ev);`.

use std::any::Any;

use termina::event::{KeyEvent, MouseEvent};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::minibuf::MiniBuffer;
use super::super::{Editor, EditorState, Severity};
use super::base::BaseLayer;
use super::completion::CompletionLayer;
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

/// The function that handles one event at a layer — a fn pointer, not a
/// `&self`/`&mut self` method on [`Layer`]: the handler needs `&mut Editor`,
/// which transitively owns the very stack this layer sits in, so a receiver
/// borrowing the layer itself would alias it. [`Layer::handler`] reads the
/// pointer out of the layer's vtable and hands it back by value, ending
/// that borrow before the caller invokes it.
pub(in crate::editor) type LayerHandler = fn(&mut Editor, LayerRef, InputEvent);

/// Which popup home(s) landing here evicts — read by
/// [`EditorState::push_layer`] right after [`Layer::setup`] runs, so no
/// layer calls [`InputStack::clear_popups`]/[`InputStack::clear_popup_layer`]
/// by hand. [`Both`](Self::Both) (the default) matches every layer but
/// `CompletionLayer`: a completion session must coexist with a `Sticky`
/// signature-help popup sitting in the current mode layer's own slot, so it
/// evicts only the pushed-layer home.
pub(in crate::editor) enum PopupEviction {
    /// [`InputStack::clear_popups`] — a pushed `PopupLayer`, if any, and the
    /// current mode layer's sticky slot.
    Both,
    /// [`InputStack::clear_popup_layer`] — a pushed `PopupLayer` alone,
    /// leaving a `Sticky` popup in the mode layer's own slot untouched.
    LayerOnly,
}

/// Why this layer is leaving the stack — the caller's own answer, not
/// something the layer infers. [`EditorState::truncate_layers`]/
/// [`EditorState::take_layer`] are the only two callers of
/// [`Layer::tear_down`] that ever see collateral (anything stacked above
/// the layer the caller actually named); both already separate "the named
/// target" from "the collateral above it" internally
/// ([`InputStack::truncate`]'s own top-first/target-last split) — this
/// reason is that same split, handed to the layer instead of silently
/// discarded.
pub(in crate::editor) enum Removal {
    /// This is the layer the caller asked to remove: [`EditorState::retire`]'s
    /// own `L`, [`EditorState::excise_layer`]'s own `r`, or the outgoing mode
    /// layer [`EditorState::push_mode_layer`] is replacing.
    Explicit,
    /// This layer merely sat above the one the caller asked to remove, and
    /// is being swept away as a side effect.
    Incidental,
}

/// What every concrete layer implements — its state, what mode (if any) it
/// presents, what happens when it enters and leaves the stack, and what
/// handles one event while it's the dispatch target. `handler`/`mode` are
/// required, no default: a layer that hasn't stated either hasn't stated its
/// policy, mirroring the closed `enum` this trait replaces, whose every
/// variant forced a match arm everywhere `kind()`/`dispatch_at` touched it.
pub(in crate::editor) trait Layer: Any {
    /// The function that handles one event while this layer is the dispatch
    /// target — see [`LayerHandler`].
    fn handler(&self) -> LayerHandler;

    /// `Some` naming the [`EditorMode`] this layer presents when it is a
    /// *mode* layer (`Base`, and the five editing modes); `None` for a
    /// transient overlay. Single source of both `InputStack::mode()` and
    /// "is this a mode layer" — the closed `enum`'s separate
    /// `is_mode_layer()` is gone because nothing needs it apart from this.
    fn mode(&self) -> Option<EditorMode>;

    /// What happens before this layer lands on top of the stack — the
    /// mirror of `tear_down`: `setup` sees what it is landing on (called
    /// before `push`), `tear_down` sees what is left (called after
    /// `truncate`). [`EditorState::push_layer`] is the only caller — it is
    /// what makes this impossible to skip, the same way `truncate_layers`
    /// makes `tear_down` impossible to skip. Empty by default: popup
    /// eviction is [`Self::popup_eviction`]'s job, not this one's, so most
    /// layers have nothing left to say here. `PickerLayer` dismisses any
    /// open completion session and replaces a live picker; `SearchLayer`/
    /// `SiftLayer` capture their own pre-entry selection snapshot here (see
    /// each one's own doc for why that must happen at this point rather
    /// than at construction).
    fn setup(&mut self, _state: &mut EditorState, _view: &EngineView) {}

    /// What happens when this layer leaves the stack, for any reason —
    /// teardown of a mode layer *is* its cancel. Empty by default. Most
    /// overrides never fire a Steel callback from here regardless of `why`:
    /// an explicit accept/cancel arm already queues one, before the
    /// truncate that reaches this, via `EditorState::take_layer` rather
    /// than `truncate_layers`/`retire` — so by the time `tear_down` runs on
    /// *that* layer's own removal, the callback question is already
    /// answered. `why` exists for the two (`MenuLayer`/`DrawerLayer`) whose
    /// explicit path answers it themselves but whose *incidental* removal
    /// (swept up as collateral above some other target) would otherwise
    /// drop their callback forever — see [`Removal`]'s own doc.
    /// `PickerLayer`/`PromptLayer` fire unconditionally, ignoring `why`:
    /// both are only ever reached here when nothing has fired their
    /// callback yet, regardless of which reason applies.
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView, _why: Removal) {}

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

    /// Whether an *async* opener's staleness check
    /// ([`InputStack::is_settled_for`]) should treat this layer as
    /// something intervened, versus a widget the request landing underneath
    /// it can simply ignore. `true` (the default) for every ordinary
    /// overlay and mode layer; `DrawerLayer`/`PopupLayer` override to
    /// `false` — the drawer is built to be worked over (a stray key falls
    /// through and it stays open) and a popup owns nothing but Ctrl-u/d, so
    /// neither should make a `show-menu!`/`completion-begin!` response read
    /// the stack as moved.
    fn is_modal(&self) -> bool {
        true
    }

    /// Which popup home(s) [`EditorState::push_layer`] evicts on this
    /// layer's behalf — see [`PopupEviction`]'s own doc for the default and
    /// its one override.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::Both
    }

    /// What [`EditorState::retire`] does to collateral above this layer when
    /// retiring it — see [`RemovalScope`]'s own doc for the default and its
    /// two overrides (`DrawerLayer`, `MenuLayer`).
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::Stack
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

/// A layer whose self-replace path takes the outgoing layer by value and
/// fires its callback with `#f` explicitly — `take_layer` never runs
/// `tear_down` on its own target, so the fire can't double against the
/// `Removal::Incidental`-only fire in `tear_down`. Implemented by
/// `MenuLayer`/`DrawerLayer` (the two self-replacing widgets); read by
/// [`EditorState::take_firing_false`](super::super::EditorState::take_firing_false).
pub(in crate::editor) trait FiresFalseOnReplace: Layer {
    fn into_callback(self: Box<Self>) -> steel::rvals::SteelVal;
}

/// What a `close-*!`/Rust-internal retirement of this layer does to whatever
/// sits above it — read by [`EditorState::retire`]. [`Stack`](Self::Stack)
/// (the default) takes any collateral above the target with it, the right
/// shape when what's above genuinely depends on the target being open.
/// [`SelfOnly`](Self::SelfOnly) removes exactly the target, leaving anything
/// above in place, for a widget something else is routinely stacked over by
/// coincidence rather than by dependency — `DrawerLayer` (browse-while-editing
/// means an `Insert` session or a code-action `Menu` often sits above it) and
/// `MenuLayer` (a non-modal `Popup` can land above it by design) both declare
/// this. A layer states its own policy here instead of the caller picking
/// per call site, so the answer can't drift between `retire`'s callers.
pub(in crate::editor) enum RemovalScope {
    Stack,
    SelfOnly,
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

// ── The stack ────────────────────────────────────────────────────────────

/// The stack itself: `Base` at index 0, always, plus whatever overlay
/// layers are pushed above it.
///
/// No `pop()`, and no `get(r)`/`get_mut(r)` handing back a bare
/// `&mut dyn Layer`: either would let a caller overwrite a layer in place
/// or remove one that isn't its own, bypassing the ordering this type
/// exists to enforce. Structure changes only through [`Self::push`],
/// [`Self::truncate`], and [`Self::truncate_to_base`] — the first two are
/// `pub(in crate::editor::input_stack)`, one level narrower than everything
/// else in this file, because each has a policy hook (`Layer::setup`,
/// `Layer::tear_down`) that must run every time it's called: the named
/// doors, [`EditorState::push_layer`]/[`EditorState::truncate_layers`]/
/// [`EditorState::take_layer`] (below, in this same module), are what run
/// it, and narrowing the raw ops keeps a caller elsewhere in `crate::editor`
/// from reaching around them. `truncate_to_base` stays at the wider
/// visibility: its one caller (`reload.rs`) deliberately drops every
/// layer's callback rather than running teardown — a stated exception, not
/// a hole a door could close. A payload is only ever reached through a
/// typed lookup — `find` (topmost of a type), `at` (the layer at a specific
/// `LayerRef`), or a layer's own named sugar over either.
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

    /// The layer at `r`, downcast to `L` — `None` if `r` is stale (see
    /// [`Self::is_live`]) or doesn't name a layer of type `L`. The
    /// address-based counterpart to [`Self::find`]/[`Self::find_mut`]
    /// ("topmost of type `L`, wherever it is"): a handler dispatched to `r`
    /// uses this to reach its own payload directly, instead of re-searching
    /// the stack by type and trusting that `L` occurs only once. Still
    /// returns a typed `&L`/`&mut L`, never a bare `&mut dyn Layer` — the
    /// same "payload only through a typed lookup" contract `find` already
    /// has, not the structure-mutating handle this type's own doc explains
    /// why there's no `get`/`get_mut` for.
    pub(in crate::editor) fn at<L: Layer>(&self, r: LayerRef) -> Option<&L> {
        self.layers
            .get(r.depth)
            .filter(|(id, _)| *id == r.id)
            .and_then(|(_, layer)| layer.downcast_ref())
    }

    pub(in crate::editor) fn at_mut<L: Layer>(&mut self, r: LayerRef) -> Option<&mut L> {
        self.layers
            .get_mut(r.depth)
            .filter(|(id, _)| *id == r.id)
            .and_then(|(_, layer)| layer.downcast_mut())
    }

    /// `r`'s handler, or `None` if `r` is stale (see [`Self::is_live`]) —
    /// what `dispatch_at` calls through to reach whichever layer `r` names,
    /// without needing to know its concrete type.
    pub(in crate::editor) fn handler(&self, r: LayerRef) -> Option<LayerHandler> {
        self.layers
            .get(r.depth)
            .filter(|(id, _)| *id == r.id)
            .map(|(_, layer)| layer.handler())
    }

    /// The topmost layer of concrete type `L`, if one is open — the ref a
    /// `close-*!` builtin or a Rust-internal retirement needs to name a
    /// widget it didn't itself just push. [`EditorState::retire`] reads each
    /// layer's own [`Layer::removal_scope`] to decide whether removing it
    /// takes collateral above it along — see that type's own doc.
    /// `enter_buffer_disk_check`/`close_buffer_and_notify` (via
    /// [`EditorState::retire_stale_confirm`]) excise a stale `Confirm`
    /// directly rather than through `retire`, since they're naming a specific
    /// `LayerRef`, not "topmost of type `L`". Every type but `BaseLayer`
    /// occurs at most once on the stack today, so "topmost" and "only"
    /// coincide in practice.
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

    /// Whether nothing *modal* (see [`Layer::is_modal`]) sits above the
    /// current mode layer, tolerating a layer of type `L` specifically —
    /// the staleness check an *async* opener (a Steel callback answering a
    /// request fired earlier: `show-menu!`, `show-drawer-list!`,
    /// `completion-begin!`) makes before landing, alongside its own
    /// mode-layer requirement. It is not a precedence rule: a synchronous,
    /// key- or command-triggered opener (`picker!`, `prompt!`) never calls
    /// this, because dispatch order already proves the stack is exactly
    /// where the key path left it — there is nothing left to check. An
    /// async response has no such guarantee: the user may have opened a
    /// picker, a menu, or moved to a different mode between the request
    /// going out and the response landing, and this is what tells the two
    /// apart. A non-modal overlay above the mode layer (a `Drawer`, built
    /// to be worked over; a `Popup`, which owns nothing but Ctrl-u/d) does
    /// not itself count as "the stack moved" regardless of `L` — only a
    /// modal one does, or a mode-layer change.
    ///
    /// The `L` parameter is what lets a self-replacing opener
    /// (`show_menu`, `show_drawer_list`, `completion_begin`) tolerate its
    /// own prior instance too, wherever it landed relative to a later
    /// non-modal overlay (e.g. a `Popup` that opened once the first
    /// instance was already up) — a plain `top() == L` check misses exactly
    /// that case, since `L` buried under a later non-modal overlay would
    /// still read as unsettled. A caller with no such instance to tolerate
    /// (there is none today) would pass a type nothing on the stack can
    /// ever be.
    pub(in crate::editor) fn is_settled_for<L: Layer>(&self) -> bool {
        let mode_depth = self.mode_layer().depth;
        self.layers[mode_depth + 1..]
            .iter()
            .all(|(_, layer)| !layer.is_modal() || layer.is::<L>())
    }

    /// Pushes `layer` on top, unconditionally — nothing is refused by
    /// *precedence*, since precedence is push order and push order is only
    /// ever decided by whoever's calling this. An async opener consults
    /// [`Self::is_settled_for`] itself before calling this (a staleness
    /// check, not a permission check); a kind-specific replace rule (the
    /// picker's own "cancel and replace a live picker") runs first for the
    /// same reason. `push` itself enforces nothing about what's already
    /// open — that's `Layer::setup`'s job, run by this method's only
    /// caller, [`EditorState::push_layer`]; the narrower-than-usual
    /// visibility here is what makes that the *only* caller.
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
    /// top-first (the former top layer is `removed[0]`; the layer that was
    /// at `r.depth` itself is `removed`'s last element). A no-op returning
    /// `vec![]` when `r` is already stale, or when `r` is `Base` — `Base`
    /// is never removed. Runs no `Layer::tear_down` — that is
    /// [`EditorState::truncate_layers`]/[`EditorState::take_layer`]'s job,
    /// this method's only two callers (besides [`Self::clear_popups`],
    /// sound only because the one layer it ever removes has an empty
    /// `tear_down` by construction — see its own doc).
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

    /// Removes exactly `r`, leaving every layer above it in place
    /// (re-indexed down by one) — the removal that must not take collateral
    /// with it: a Rust-internal retirement of a `Confirm` that no longer
    /// targets anything live, while an unrelated session (a `Prompt`, a
    /// `Picker`) may have landed above it since; or an explicit
    /// `close-drawer!`, whose browse-while-editing design means something
    /// unrelated (an `Insert` session, a code-action `Menu`) is routinely
    /// stacked above it. Runs no `Layer::tear_down` itself, same contract as
    /// [`Self::truncate`] — that is [`EditorState::excise_layer`]'s job. A
    /// no-op returning `None` when `r` is already stale, or is `Base` —
    /// `Base` is never removed.
    pub(in crate::editor::input_stack) fn excise(&mut self, r: LayerRef) -> Option<Box<dyn Layer>> {
        if r.depth == 0 || !self.is_live(r) {
            return None;
        }
        Some(self.layers.remove(r.depth).1)
    }

    /// Removes every layer above `Base`, dropping each one — no
    /// `Layer::tear_down` runs (same as [`Self::truncate`]'s own contract),
    /// so a still-open mode layer or overlay's Steel callback is discarded,
    /// not fired — and resets `Base` itself to a fresh, all-default
    /// [`BaseLayer`] — a reload must not leave Extend on, or a
    /// signature-help popup visible, for hooks that never saw either turned
    /// on. Returns nothing: its one caller (`reload.rs`) never reads what
    /// was removed.
    pub(in crate::editor) fn truncate_to_base(&mut self) {
        self.layers.truncate(1);
        self.layers[0].1 = Box::new(BaseLayer {
            extend: false,
            sticky_popup: None,
        });
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
    /// layer; a toggle reads `mode()` first to decide the target value.
    ///
    /// Clears any open popup when the flag actually flips — the deleted
    /// `lib.scm` hook (`on-mode-change → close-popup!`) covered every mode
    /// transition; `push_mode_layer`'s own `clear_popups()` call replaced it
    /// for every transition that goes through `push_mode_layer`, except
    /// Normal↔Extend, which never does (this is the one write site for
    /// that transition). Gated on an observed diff, not unconditional, so a
    /// same-value call (the two collapse-and-exit-extend commands call this
    /// with `false` even when already Normal) doesn't spuriously kill an
    /// unrelated popup — mirrors `detect_mode_change`'s own "only on a
    /// diff" contract for the same hook this replaces.
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

    /// The minibuf owned by `top()` specifically — `None` if `top()` isn't
    /// one of the four minibuf-backed mode layers, even if one is open
    /// buried beneath a picker or menu (a mouse click always falls through
    /// under a minibuf-mode layer, so focus and the picker/menu it opens can
    /// land above one without ever closing it). A `Minibuf`-target
    /// `CompletionLayer` is not such a takeover — it's a dropdown drawn
    /// above the command line, not a replacement for it — so it's skipped
    /// here the same way `mode_layer()` skips it (its own `mode()` is
    /// `None`) rather than counting as "something else is on top now".
    /// Distinct from [`Self::minibuf`] (topmost-of-any-depth), which every
    /// minibuf-mode layer's *own* handler uses safely — dispatch only ever
    /// reaches it while it's already `top()`. [`EditorState::minibuf`] is
    /// the gated reader every external (non-owning-layer) consumer — the
    /// statusline, the hardware-cursor placement — must use instead.
    pub(in crate::editor) fn top_minibuf(&self) -> Option<&MiniBuffer> {
        self.layers
            .iter()
            .rev()
            .find(|(_, layer)| !layer.is::<CompletionLayer>())
            .and_then(|(_, layer)| layer.minibuf())
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
        if let Some(r) = self.ref_of::<PopupLayer>() {
            return self.layers[r.depth].1.downcast_mut::<PopupLayer>();
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

    /// Clears the pushed-layer popup home alone — a [`PopupLayer`] layer, if
    /// one is open (always `top()`, enforced below), leaving the current
    /// mode layer's sticky slot untouched. `CompletionLayer`'s
    /// [`PopupEviction::LayerOnly`] override reaches this instead of
    /// [`Self::clear_popups`]: a completion session must evict a
    /// `Scrollable` popup (hover, the `gn`/`gp` diagnostic overlay — both
    /// pushed layers) the same as any other opener, but must *not* dismiss
    /// a `Sticky` signature-help popup, which lives in the slot and is meant
    /// to coexist with an open completion menu. Takes no `EditorState`/`view`
    /// (unlike `EditorState::truncate_layers`), so this truncates `self`
    /// directly rather than running `tear_down` — sound because the one
    /// layer this ever removes is a `PopupLayer`, whose `tear_down` is empty
    /// by construction.
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

    /// Clears every home a popup could occupy: [`Self::clear_popup_layer`]
    /// plus the current mode layer's sticky slot. Called automatically by
    /// [`EditorState::push_layer`] for every layer whose
    /// [`Layer::popup_eviction`] is [`PopupEviction::Both`] (the default —
    /// every layer but `CompletionLayer`) — no layer calls this by hand.
    /// The two callers that reach it *without* going through `push_layer`
    /// are `show_popup`'s `Sticky` arm (which writes straight into a mode
    /// layer's slot rather than pushing anything, so `(show-popup! …)`
    /// still replaces any popup already showing, regardless of which of the
    /// two homes it used — the documented "no stacking" contract) and
    /// `close_popup` (a direct clear with nothing to push).
    pub(in crate::editor) fn clear_popups(&mut self) {
        self.clear_popup_layer();
        if let Some(slot) = self.sticky_popup_slot_mut() {
            *slot = None;
        }
    }
}

impl EditorState {
    /// The only way to push a layer — [`Layer::setup`] runs first, always,
    /// because [`InputStack::push`] itself is narrowed to this module and
    /// unreachable from anywhere else. Mirrors [`Self::truncate_layers`]:
    /// `setup` sees what it is landing on (stack unchanged so far),
    /// `tear_down` sees what is left (already removed). Popup eviction
    /// ([`Layer::popup_eviction`]) runs after `setup`, not before — a
    /// no-op ordering difference for every layer but `PickerLayer`, whose
    /// own `setup` needs to run its dismiss-and-replace work first; eviction
    /// only ever touches the two popup homes, never selections, completion,
    /// or picker state, so nothing else can observe the difference.
    pub(in crate::editor) fn push_layer<L: Layer>(
        &mut self,
        view: &EngineView,
        mut layer: L,
    ) -> LayerRef {
        layer.setup(self, view);
        match layer.popup_eviction() {
            PopupEviction::Both => self.input.clear_popups(),
            PopupEviction::LayerOnly => self.input.clear_popup_layer(),
        }
        self.input.push(layer)
    }

    /// Removes `r` and everything above it, running each removed layer's own
    /// [`Layer::tear_down`] top-first — the single teardown path for both a
    /// mode-layer exit and a `close-*!`/Rust-internal overlay retirement.
    /// What each layer's teardown does, for any reason it leaves the stack,
    /// lives on that layer's own `Layer` impl — a `Confirm` arm does its own
    /// accept work (recording history, restoring/clearing a stash) *before*
    /// truncating, so by the time teardown runs, every removal is already
    /// the "cancel" case; there is no separate "cancel-specific work" split
    /// to make. `r`'s own layer — the one this call was actually asked to
    /// remove — gets [`Removal::Explicit`]; anything stacked above it, swept
    /// up as collateral, gets [`Removal::Incidental`] — `InputStack::truncate`
    /// already returns them in target-last order, so splitting them here is
    /// just reading that order rather than folding it into one loop.
    pub(in crate::editor) fn truncate_layers(&mut self, view: &EngineView, r: LayerRef) {
        let mut removed = self.input.truncate(r);
        if let Some(mut target) = removed.pop() {
            for mut layer in removed {
                layer.tear_down(self, view, Removal::Incidental);
            }
            target.tear_down(self, view, Removal::Explicit);
        }
    }

    /// [`Self::truncate_layers`]'s variant for a caller that needs `r`'s own
    /// layer *by value* rather than merely retired — every accept/cancel arm
    /// that reads a widget's payload before acting on it (a menu's chosen
    /// index, a picker's selected payload, a completion session to hand to
    /// the LSP client). Truncates the same way (top-first — anything stacked
    /// above `r` gets ordinary teardown, since none of it asked to be
    /// taken), but pulls `r`'s own layer out of the batch instead of tearing
    /// it down, so its own accept-specific work runs once instead of racing
    /// whatever `tear_down` would have done to the same payload.
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
        for mut layer in removed {
            layer.tear_down(self, view, Removal::Incidental);
        }
        taken
    }

    /// Takes `r`'s own layer by value and fires its callback with `#f`
    /// explicitly — the self-replace path shared by `show_menu`/
    /// `show_drawer_list` (a second open while one is still showing replaces
    /// it, so the outgoing owner learns its widget is gone) and the drawer's
    /// own `Esc` arm. `take_layer` never runs `tear_down` on its own target,
    /// so this can't double-fire against the `Removal::Incidental`-only fire
    /// there; an explicit `close-*!` stays silent by routing through
    /// [`Self::retire`] instead, which is exactly why that path must not
    /// come here.
    pub(in crate::editor) fn take_firing_false<L: FiresFalseOnReplace>(
        &mut self,
        view: &EngineView,
        r: LayerRef,
    ) {
        let old: Box<L> = self.take_layer(view, r);
        self.queue_steel_call(
            old.into_callback(),
            vec![steel::rvals::SteelVal::BoolV(false)],
        );
    }

    /// Removes exactly `r` via [`InputStack::excise`], running its own
    /// `tear_down` but leaving everything stacked above it untouched.
    /// [`Self::retire`] calls this itself for a [`RemovalScope::SelfOnly`]
    /// layer; its other two callers name a specific `LayerRef` they already
    /// hold rather than going through `retire`'s own `ref_of::<L>()` lookup:
    /// a stale `Confirm` retirement ([`Self::retire_stale_confirm`]), where
    /// an unrelated session landing above it since has nothing to do with
    /// the question the confirm was answering (`ConfirmLayer::tear_down` is
    /// empty, so this can never double-fire a callback there); and
    /// `close_drawer`, which already has the token-matched ref
    /// `drawer_ref_with_token` gave it. A no-op when `r` is already stale.
    pub(in crate::editor) fn excise_layer(&mut self, view: &EngineView, r: LayerRef) {
        if let Some(mut layer) = self.input.excise(r) {
            layer.tear_down(self, view, Removal::Explicit);
        }
    }

    /// Retires the topmost layer of type `L`, if one is open — the
    /// `ref_of::<L>()` lookup shared by every `close-*!` builtin and internal
    /// dismissal that names its target by type rather than a `LayerRef` it
    /// already holds (`close_menu`, `dismiss_completion`; `close_drawer`
    /// excises directly instead, since it holds a token-matched `LayerRef`
    /// already — see its own doc). `show_menu`/`show_drawer_list`'s
    /// self-replace paths take by value instead, via
    /// [`Self::take_firing_false`], since they must fire the outgoing
    /// callback themselves. What happens to anything stacked above `L` is
    /// `L`'s own [`Layer::removal_scope`] — collateral
    /// removal ([`Self::truncate_layers`]) when it genuinely depends on `L`
    /// being open, in-place removal ([`Self::excise_layer`]) when it's merely
    /// stacked over `L` by coincidence (`DrawerLayer`, `MenuLayer`).
    pub(in crate::editor) fn retire<L: Layer>(&mut self, view: &EngineView) {
        let Some(r) = self.input.ref_of::<L>() else {
            return;
        };
        match self
            .input
            .at::<L>(r)
            .expect("ref_of found L at r")
            .removal_scope()
        {
            RemovalScope::Stack => self.truncate_layers(view, r),
            RemovalScope::SelfOnly => self.excise_layer(view, r),
        }
    }

    /// The async-staleness gate every opener whose Steel callback fires
    /// after the key path that triggered it has already returned must
    /// check before landing — `show-menu!`, `show-drawer-list!`,
    /// `completion-begin!`. `M` is the mode layer the request requires
    /// (`BaseLayer` for the first two, `InsertLayer` for completion); `L`
    /// is the overlay it's about to push. `true` when the request should be
    /// dropped: the mode layer changed, or a *modal* overlay landed on top
    /// of it, since the request went out — the user left the required mode,
    /// or opened something else, while the response was in flight. A prior
    /// instance of `L` itself — buried or not — is not "the stack moved": a
    /// second call while the first is still open is the normal refresh
    /// path (`is_settled_for`'s own doc has the full reasoning), which each
    /// caller still has to retire/replace itself.
    ///
    /// Reports the drop itself (`Severity::Trace`, since this is timing —
    /// the user moved on — never a plugin bug) so a caller only needs
    /// `if self.async_opener_stale::<M, L>(what) { return Ok(()); }`.
    pub(in crate::editor) fn async_opener_stale<M: Layer, L: Layer>(&mut self, what: &str) -> bool {
        let stale =
            !self.input.is::<M>(self.input.mode_layer()) || !self.input.is_settled_for::<L>();
        if stale {
            self.report(
                Severity::Trace,
                format!("{what}: the stack moved before it could open — ignored"),
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
mod tests {
    use super::*;
    use crate::editor::input_stack::PickerLayer;
    use crate::editor::input_stack::command::CommandLayer;
    use crate::editor::input_stack::confirm::{ConfirmAction, ConfirmChoice, ConfirmLayer};
    use crate::editor::input_stack::drawer::DrawerLayer;
    use crate::editor::input_stack::insert::InsertLayer;
    use crate::editor::input_stack::menu::MenuLayer;
    use steel::rvals::SteelVal;

    fn menu(label: &str) -> MenuLayer {
        MenuLayer {
            rows: hume_ui::popup::MenuRows::plain(vec![label.to_string()]),
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
        stack.truncate_to_base();
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
    fn is_settled_for_true_on_a_fresh_stack() {
        // `PickerLayer` is never pushed in this group — an arbitrary witness
        // type, standing in for `is_settled_for`'s base case (nothing modal
        // open at all), same contract the deleted `is_stack_settled` tested
        // directly.
        let stack = InputStack::new();
        assert!(stack.is_settled_for::<PickerLayer>());
    }

    #[test]
    fn is_settled_for_false_with_an_unrelated_modal_overlay_above_the_mode_layer() {
        let mut stack = InputStack::new();
        stack.push(menu("m"));
        assert!(!stack.is_settled_for::<PickerLayer>());
    }

    #[test]
    fn is_settled_for_true_again_once_the_overlay_is_truncated() {
        let mut stack = InputStack::new();
        let r = stack.push(menu("m"));
        stack.truncate(r);
        assert!(stack.is_settled_for::<PickerLayer>());
    }

    #[test]
    fn is_settled_for_true_with_a_mode_layer_alone_on_top() {
        let mut stack = InputStack::new();
        stack.push(InsertLayer { sticky_popup: None });
        assert!(stack.is_settled_for::<PickerLayer>());
    }

    #[test]
    fn is_settled_for_tolerates_its_own_type_buried_under_a_non_modal_overlay() {
        // The case `is_settled_for` exists for, beyond `is_stack_settled`'s
        // old all-or-nothing contract: a prior `Menu` instance still counts
        // as settled for `Menu` even with a later non-modal `Drawer` on top
        // of it.
        let mut stack = InputStack::new();
        stack.push(menu("m"));
        stack.push(DrawerLayer::new(
            vec!["d".to_string()],
            SteelVal::BoolV(false),
        ));
        assert!(stack.is_settled_for::<MenuLayer>());
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
            minibuf: MiniBuffer::new(""),
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
    fn clear_popup_layer_leaves_the_sticky_slot_intact() {
        // `CompletionLayer::setup`'s narrower need than `clear_popups`: evict
        // a pushed `Popup` layer without dismissing a `Sticky` popup sitting
        // in the current mode layer's own slot.
        let mut stack = InputStack::new();
        *stack.sticky_popup_slot_mut().expect("Base has a slot") = Some(popup_model("sticky"));
        stack.push(popup_model("scrollable"));
        stack.clear_popup_layer();
        assert!(stack.is::<BaseLayer>(stack.top()), "the layer home is gone");
        assert_eq!(
            stack.popup().map(|p| p.text.as_str()),
            Some("sticky"),
            "the sticky slot must survive"
        );
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
