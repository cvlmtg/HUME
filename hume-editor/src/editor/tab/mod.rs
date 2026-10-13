//! Tab pages, in Vim's sense: a tab is a saved window layout (its own
//! `FocusedLayout`: a tree and its focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! This module holds the one chokepoint every tab *switch* goes through,
//! [`switch_to_tab`], plus [`install_live`]: the exchange of the live tab's
//! layout in `EngineView` for another. Opening/closing a tab also has
//! to create or tear down panes (`commands::pane::open_pane_as_new_tab`/
//! `drop_pane_state`), which is a `commands`-layer concern. See
//! `commands::tab::open_tab`/`close_tab`, which call `TabStore`'s
//! lower-level `open_after_current`/`close_current` directly rather than
//! through `switch_to_tab`, but still route the live-tab exchange itself
//! through `install_live`.

pub(crate) mod store;
pub(crate) use store::TabId;
pub(in crate::editor) use store::TabStore;

use hume_engine::pipeline::{EngineView, FocusedLayout};

use crate::editor::EditorState;

/// Install `layout` as the live tab, returning the layout it displaces for
/// the caller to stash or free. Goes through `focus::install_layout`, since
/// replacing the layout moves focus.
///
/// `resync_viewport_dims`/`begin_frame` run here, not just at the next
/// frame's own `sync_viewport_dims`/`prepare_frame`: a command dispatch that
/// switches tabs and then reads pane geometry in the same call (a scroll
/// bound after a tab-switch key, a Steel body chaining `(call! "goto-next-tab" pane)`
/// onto a motion) would otherwise see the outgoing tab's stale viewport and
/// an un-rewound line store for the incoming one. This does not touch the
/// *hidden* tab's staleness model: a backgrounded pane is still untouched
/// until its own tab is next focused; this only closes the window for the
/// tab landing here, right now.
#[must_use]
pub(in crate::editor) fn install_live(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: FocusedLayout,
) -> FocusedLayout {
    let outgoing = super::focus::install_layout(state, view, layout);
    view.resync_viewport_dims();
    view.begin_frame();
    outgoing
}

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
    let outgoing = install_live(state, view, incoming);
    state.tabs.switch(outgoing, target);
}
