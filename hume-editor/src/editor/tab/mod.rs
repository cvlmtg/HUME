//! Tab pages, in Vim's sense: a tab is a saved window layout (its own
//! `FocusedLayout`: a tree and its focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! This module holds the one chokepoint every tab *switch* goes through,
//! [`switch_to_tab`]. Opening/closing a tab also has to create or tear down
//! panes (`commands::pane::open_pane_as_new_tab`/`drop_pane_state`), which
//! is a `commands`-layer concern. See `commands::tab::open_tab`/`close_tab`,
//! which call `TabStore`'s lower-level `open_after_current`/`close_current`
//! directly rather than through `switch_to_tab`, but still exchange the live
//! tab's layout through `focus::install_layout`.

pub(crate) mod store;
pub(crate) use store::TabId;
pub(in crate::editor) use store::TabStore;

use hume_engine::pipeline::EngineView;

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
    let incoming = state.tabs.take(target);
    let outgoing = super::focus::install_layout(state, view, incoming);
    state.tabs.switch(outgoing, target);
}
