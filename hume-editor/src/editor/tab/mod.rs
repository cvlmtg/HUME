//! Tab pages, in Vim's sense: a tab is a saved window layout (its own
//! `FocusedLayout`: a tree and its focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! This module holds the one chokepoint every tab *switch* goes through,
//! [`switch_to_tab`], plus [`take_live`]/[`install_live`]: the shared
//! exchange between the live tab's layout in `EngineView` and a `TabStore`
//! call's own stashed layout. Opening/closing a tab also has
//! to create or tear down panes (`commands::pane::open_pane_as_new_tab`/
//! `drop_pane_state`), which is a `commands`-layer concern. See
//! `commands::tab::open_tab`/`close_tab`, which call `TabStore`'s
//! lower-level `open_after_current`/`close_current` directly rather than
//! through `switch_to_tab`, but still route the live-tab exchange itself
//! through `take_live`/`install_live`.

pub(crate) mod store;
pub(crate) use store::TabId;
pub(in crate::editor) use store::TabStore;

use hume_engine::pipeline::{EngineView, FocusedLayout};

use crate::editor::EditorState;

/// Read the live layout out of `view`, leaving it unchanged. The caller's
/// own [`install_live`] is what actually displaces it, via
/// [`EngineView::replace_layout`]. Shared first half of every op that
/// replaces the whole live tab.
///
/// Ends the outgoing pane's open Insert/paste session first, before
/// reading: the session's teardown can edit the outgoing pane's buffer (the
/// blank-line indent trim), which propagates to every pane showing it, so
/// it must run while the outgoing tab's panes are still live in `view`, not
/// against a half-installed incoming tab.
pub(in crate::editor) fn take_live(state: &mut EditorState, view: &EngineView) -> FocusedLayout {
    super::focus::end_focus_sessions(state, view);
    view.focused_layout().clone()
}

/// Install `layout` as the live tab: the shared second half of every op
/// that replaces which tab is on screen. It goes through
/// `focus::install_layout`, since replacing the layout moves focus.
///
/// `install_layout`'s return (the layout being displaced) is discarded: it
/// names the exact same pool entries `take_live` already read moments
/// earlier and the caller has already threaded onward (into `TabStore`'s
/// stash, or dropped via `into_detached` for a closing tab), so nothing is left
/// to leak.
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
pub(in crate::editor) fn install_live(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: FocusedLayout,
) {
    let _ = super::focus::install_layout(state, view, layout);
    view.resync_viewport_dims();
    view.begin_frame();
}

/// Switch the active tab to `target`, snapshotting the outgoing tab's live
/// layout into its stash slot first. No-op if `target` is already
/// current: every caller can pass the current tab's own id without a special case.
pub(in crate::editor) fn switch_to_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    target: TabId,
) {
    if target == state.tabs.current() {
        return;
    }
    let outgoing = take_live(state, view);
    let layout = state.tabs.switch(outgoing, target);
    install_live(state, view, layout);
}
