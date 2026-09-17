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
//! [`InputStack::is_settled_for`], answers a different question — whether
//! an *async* opener's request has gone stale — not who outranks whom.
//! [`Layer::setup`] is a third, orthogonal concern: what a layer clears out
//! of its own way on entry (chiefly an open popup), never whether it may
//! land at all.
//!
//! A layer is a *purpose*, not a widget: today each kind wraps at most one
//! widget, but nothing in the stack's own API assumes that stays true.
//!
//! `stack.rs` is the mechanism: [`InputStack`], [`LayerRef`], [`InputEvent`],
//! and the [`Layer`] trait every layer implements. Its own API names no
//! concrete layer — a closed `enum` of every layer kind used to live here
//! instead, which is exactly the coupling this trait replaces.
//!
//! Each concrete layer — its state, its `Layer` impl, its own
//! key/paste/mouse handler, its per-frame render sync (where it has a view
//! to keep live — a mode layer like `Search`/`Sift` has none), and its named
//! lookup sugar over `InputStack` — lives in a file of its own, named by a
//! `mod` line below. Adding another means adding one file and one `mod` line
//! here, nothing else. `placement.rs` is the one exception: screen-placement
//! math shared by the three cursor/token-anchored overlays (`popup`, `menu`,
//! `completion`), not itself a layer.

mod placement;
mod stack;

pub(in crate::editor) mod base;
pub(in crate::editor) mod command;
pub(in crate::editor) mod completion;
pub(in crate::editor) mod confirm;
pub(in crate::editor) mod drawer;
pub(in crate::editor) mod insert;
pub(in crate::editor) mod menu;
pub(in crate::editor) mod picker;
pub(in crate::editor) mod popup;
pub(in crate::editor) mod prompt;
pub(in crate::editor) mod search;
pub(in crate::editor) mod sift;

pub(in crate::editor) use stack::{InputEvent, InputStack, Layer, LayerRef};

pub(in crate::editor) use base::BaseLayer;
pub(in crate::editor) use command::CommandLayer;
pub(in crate::editor) use completion::CompletionLayer;
pub(in crate::editor) use confirm::{ConfirmAction, ConfirmChoice, ConfirmLayer};
pub(in crate::editor) use drawer::DrawerLayer;
pub(in crate::editor) use insert::InsertLayer;
pub(in crate::editor) use menu::MenuLayer;
pub(in crate::editor) use picker::{PickerItem, PickerSession};
// `PickerLayer` itself (as opposed to `PickerSession`, the payload every
// production caller reaches through `open_picker`/`picker()`/`picker_mut()`)
// is only ever named directly by tests asserting on the stack's own shape
// (`is::<PickerLayer>(r)`, `ref_of::<PickerLayer>()`).
#[cfg(test)]
pub(in crate::editor) use picker::PickerLayer;
pub(in crate::editor) use popup::PopupLayer;
pub(in crate::editor) use prompt::PromptLayer;
pub(in crate::editor) use search::SearchLayer;
pub(in crate::editor) use sift::SiftLayer;
