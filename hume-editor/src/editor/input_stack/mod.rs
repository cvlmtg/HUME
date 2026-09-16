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
//! A layer is a *purpose*, not a widget: today each kind wraps at most one
//! widget, but nothing in the stack's own API assumes that stays true.
//!
//! `stack.rs` is the mechanism: [`InputStack`], [`LayerRef`], [`InputEvent`],
//! and the [`Layer`] trait every layer implements. Its own API names no
//! concrete layer — a closed `enum` of every layer kind used to live here
//! instead, which is exactly the coupling this trait replaces.
//!
//! The concrete layer types themselves still live inside `stack.rs`, one
//! `impl Layer` block apiece — this is a transitional state, not the target
//! one. Each is moving out into a file of its own (state, `Layer` impl, and
//! key/paste/mouse handler together) over the commits that follow; once a
//! layer has its own file, adding another means adding one file and one
//! `mod` line here, nothing else.

mod stack;

pub(in crate::editor) use stack::{InputEvent, InputStack, Layer, LayerRef};

// The concrete layer types living inside `stack.rs` for now (see this
// module's own doc) — re-exported flat, matching how the closed `enum`'s
// variants used to read minus the `InputLayer::` prefix. `DrawerModel`,
// `MenuModel`, `ConfirmModel`, and `PopupModel` are the four layers whose
// state predates this refactor's payload/handler split (`overlay_models.rs`)
// and are named from there instead.
pub(in crate::editor) use stack::{
    BaseLayer, CommandLayer, CompletionLayer, InsertLayer, PickerLayer, PromptLayer, SearchLayer,
    SiftLayer,
};
