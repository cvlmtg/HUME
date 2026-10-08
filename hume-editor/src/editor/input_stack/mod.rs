//! The stack of active input-handling layers, from the base editing surface
//! up through whatever transient overlay currently owns the keyboard.
//!
//! The stack is the only precedence mechanism: the top layer sees a key first
//! and its handler decides whether it handles, falls through, or discards it.
//! `push` never refuses. [`InputStack::is_settled_for`] only answers whether an
//! async opener's request has gone stale, and [`Layer::setup`] only clears what
//! a layer needs out of its way on entry (chiefly an open popup).
//!
//! `stack.rs` holds the mechanism ([`InputStack`], [`LayerRef`], [`InputEvent`],
//! the [`Layer`] trait) and names no concrete layer. Each concrete layer (state,
//! `Layer` impl, handlers, render sync, lookup helpers) lives in its own file
//! with a `mod` line below. `placement.rs` (overlay placement shared by `popup`,
//! `menu`, `completion`) and `snapshot.rs` (pre-entry selection capture shared
//! by `search`/`sift`) are helpers, not layers.

mod placement;
mod snapshot;
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
pub(in crate::editor) use completion::{BufferCompletionLayer, MinibufCompletionLayer};
pub(in crate::editor) use confirm::{ConfirmAction, ConfirmLayer, ConfirmPermit};
pub(in crate::editor) use drawer::{DrawerLayer, DrawerSelect};
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
pub(in crate::editor) use snapshot::PaneSnapshot;
