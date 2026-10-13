//! Tab pages, in Vim's sense: a tab is a saved window layout (its own
//! `FocusedLayout`: a tree and its focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! Every change of the active tab goes through this module's three
//! operations, [`switch_to_tab`], [`open_tab`] and [`close_tab`]: each moves
//! a layout between `TabStore` and `EngineView` through
//! `focus::install_layout`, so no tab is ever left with a layout in neither
//! place. Creating and freeing the panes themselves
//! (`commands::pane::open_pane_as_new_tab`, `commands::tab::close_tab`) is a
//! `commands`-layer concern.

pub(crate) mod store;
pub(crate) use store::TabId;
pub(in crate::editor) use store::{SingleTab, TabStore};

use hume_engine::pipeline::{DetachedPane, EngineView, FocusedLayout};

use crate::editor::EditorState;

/// Switch the active tab to `target`, stashing the outgoing tab's layout.
/// No-op if `target` is already current: every caller can pass the current
/// tab's own id without a special case.
pub(in crate::editor) fn switch_to_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    target: TabId,
) {
    if target == state.tabs.current() {
        return;
    }
    let outgoing = enter(state, view, target);
    state.tabs.switch(outgoing, target);
}

/// Make `layout` the live layout of a new tab inserted right after the
/// current one (Vim's `:tabnew` placement), and switch to it.
pub(in crate::editor) fn open_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: FocusedLayout,
) {
    let outgoing = super::focus::install_layout(state, view, layout);
    state.tabs.open_after_current(outgoing);
}

/// Close the current tab, switching to its neighbour (see
/// `TabStore::adjacent`), and return the closed tab's panes, detached, for
/// the caller to free. Refused, with nothing changed, when it is the only
/// tab.
#[must_use = "the closed tab's panes must be freed"]
pub(in crate::editor) fn close_tab(
    state: &mut EditorState,
    view: &mut EngineView,
) -> Result<Vec<DetachedPane>, SingleTab> {
    let survivor = state.tabs.adjacent()?;
    let closing = enter(state, view, survivor);
    state.tabs.close_current(survivor);
    Ok(closing.into_detached())
}

/// Install inactive tab `target`'s layout as the live one, returning the
/// layout it displaces. The caller finishes the exchange in `TabStore`.
fn enter(state: &mut EditorState, view: &mut EngineView, target: TabId) -> FocusedLayout {
    let incoming = state.tabs.take(target);
    super::focus::install_layout(state, view, incoming)
}
