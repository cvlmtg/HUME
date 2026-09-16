//! The stack of active input-handling layers, from the base editing surface
//! up through whatever transient overlay currently owns the keyboard.
//!
//! Precedence used to be four independent `Option<T>` fields on
//! `ConfigState`, each checked by its own `is_some()` guard in a fixed `&&`
//! chain (`mappings/mod.rs`'s old `handle_key`) — the chain's *order*, not
//! any property of the widgets themselves, decided which one saw a key when
//! more than one happened to be open. A stack makes precedence a fact about
//! the data (whichever layer is on top runs first) rather than a fact about
//! the dispatcher's source order, and makes "can two of these ever be open
//! at once" a question [`InputStack::accepts_above`] answers once instead of
//! a property every opener has to re-derive from every other opener's own
//! guard.
//!
//! A layer is a *purpose*, not a widget: today each of the four kinds below
//! wraps exactly one widget, but nothing in the stack's own API assumes
//! that stays true.

use termina::event::KeyEvent;

use super::overlay_models::{ConfirmModel, DrawerModel, MenuModel};
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
/// [`InputStack::accepts_above`] and every opener's own gate switches on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum LayerKind {
    /// The always-present layer at index 0. Never removed.
    Base,
    Drawer,
    Menu,
    Picker,
    Confirm,
}

/// One entry on the stack. `Base` carries no payload today — sticky-popup
/// and extend-mode state land here once modes join the stack.
///
/// `Picker` is boxed: `PickerSession` alone is several times the size of
/// every other variant's payload (its own fuzzy-match scoring buffers,
/// picked items, `#:actions` table, …), so leaving it unboxed would size
/// the whole enum — every `Base`/`Drawer`/`Menu`/`Confirm` layer included —
/// to the picker's own footprint.
pub(in crate::editor) enum InputLayer {
    Base,
    Drawer(DrawerModel),
    Menu(MenuModel),
    Picker(Box<PickerSession>),
    Confirm(ConfirmModel),
}

impl InputLayer {
    fn kind(&self) -> LayerKind {
        match self {
            InputLayer::Base => LayerKind::Base,
            InputLayer::Drawer(_) => LayerKind::Drawer,
            InputLayer::Menu(_) => LayerKind::Menu,
            InputLayer::Picker(_) => LayerKind::Picker,
            InputLayer::Confirm(_) => LayerKind::Confirm,
        }
    }
}

/// One input event working its way down the stack. `Key` is the only
/// variant for now; a paste or a mouse event still takes its own dedicated
/// path outside this walk. Adding those variants later forces every layer's
/// handler to state its policy for them at compile time, the same way this
/// one variant already forces an exhaustive match today.
pub(in crate::editor) enum InputEvent {
    Key(KeyEvent),
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
            layers: vec![(0, InputLayer::Base)],
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
    /// widget it didn't itself just push (D4's "present but not top is an
    /// error", every `confirm`-retirement site on buffer close/focus
    /// change). Every kind but `Base` occurs at most once on the stack
    /// today, so "topmost" and "only" coincide in practice.
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

    /// Whether the current top layer allows anything to be pushed above it.
    /// `Base`/`Drawer` accept any overlay; `Picker`/`Confirm`/`Menu` accept
    /// none — each of those three is a modal owner for as long as it's
    /// open. `kind` is unused today (every current layer's policy depends
    /// only on what's already on top, not on what wants to land above it);
    /// kept in the signature since a future layer (a scrollable popup dying
    /// on any push regardless of policy, a Completion session accepting
    /// everything) may need to consult it directly.
    pub(in crate::editor) fn accepts_above(&self, _kind: LayerKind) -> bool {
        match self
            .layers
            .last()
            .expect("Base always occupies index 0")
            .1
            .kind()
        {
            LayerKind::Base | LayerKind::Drawer => true,
            LayerKind::Picker | LayerKind::Confirm | LayerKind::Menu => false,
        }
    }

    /// Pushes `layer` on top, unconditionally — callers consult
    /// [`Self::accepts_above`] (or a kind-specific replace rule, e.g. the
    /// picker's own "cancel and replace a live picker") *before* calling
    /// this; `push` itself enforces nothing about what's already open.
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
    /// [`Self::truncate`]. `Base` itself carries no payload today, so there
    /// is nothing on it to reset yet — that arrives once extend-mode and
    /// the sticky popup slot move onto it.
    pub(in crate::editor) fn truncate_to_base(&mut self) -> Vec<InputLayer> {
        self.layers
            .split_off(1)
            .into_iter()
            .rev()
            .map(|(_, layer)| layer)
            .collect()
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
    fn accepts_above_per_kind() {
        let mut stack = InputStack::new();
        assert!(stack.accepts_above(LayerKind::Menu));
        stack.push(menu("m"));
        assert!(!stack.accepts_above(LayerKind::Confirm));
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
}
