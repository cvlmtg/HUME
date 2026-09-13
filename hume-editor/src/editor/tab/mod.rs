//! Tab pages — Vim's sense: a tab is a saved window layout (its own
//! `LayoutTree` + its own focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! This module holds the one chokepoint every tab *switch* goes through,
//! [`switch_to_tab`], plus [`take_live`]/[`install_live`] — the shared
//! exchange between `view.layout`/`state.focus` (the live tab) and
//! a `TabStore` call's own layout/focus pair. Opening/closing a tab also has
//! to create or tear down panes (`commands::pane::open_pane_as_new_tab`/
//! `drop_pane_state`), which is a `commands`-layer concern — see
//! `commands::tab::open_tab`/`close_tab`, which call `TabStore`'s
//! lower-level `open_after_current`/`close_current` directly rather than
//! through `switch_to_tab`, but still route the live-tab exchange itself
//! through `take_live`/`install_live`.

pub(crate) mod store;
pub(crate) use store::TabId;
pub(in crate::editor) use store::TabStore;

use hume_engine::pipeline::{EngineView, LayoutTree, PaneId};

use crate::editor::EditorState;

/// Read the live layout/focus out of `view`/`state`, leaving `view`
/// unchanged — the caller's own [`install_live`] is what actually displaces
/// it, via [`EngineView::replace_layout`]. Shared first half of every op
/// that replaces the whole live tab.
///
/// Ends the outgoing pane's open Insert session first, before reading:
/// `view.layout()` names the outgoing tab and `state.focus.id()` names
/// its focused pane together, right up to this point — the last moment
/// either is true until `install_live` runs. `end_insert_session_if_active`'s
/// own edit can shrink the outgoing pane's buffer (the blank-line indent
/// trim), and any per-(pane, buffer) state it touches should resolve
/// against a still-consistent (layout, focus) pair, not a half-installed
/// incoming tab.
pub(in crate::editor) fn take_live(
    state: &mut EditorState,
    view: &EngineView,
) -> (LayoutTree, PaneId) {
    super::commands::end_insert_session_if_active(state, view);
    (view.layout().clone(), state.focus.id())
}

/// Install `layout`/`focus` as the live tab. Shared second half of every op
/// that replaces `view.layout`/`state.focus` together — the pair that
/// jointly define which tab is on screen. The focus half goes through
/// `focus::focus_pane` rather than a bare assignment: its Insert-session
/// teardown is already done by `take_live` above by the time this runs (a
/// no-op here as a result), but its paste-session commit isn't — that one
/// resolves the outgoing pane/buffer by id rather than through the layout
/// tree, so it has no matching earlier call and still needs to run here,
/// before focus moves to `focus`.
///
/// `replace_layout`'s return (the tree being displaced) is discarded: it
/// names the exact same pool entries `take_live` already read moments
/// earlier and the caller has already threaded onward (into `TabStore`'s
/// stash, or dropped via `into_detached` for a closing tab) — nothing left
/// to leak.
///
/// `resync_viewport_dims`/`begin_frame` run here, not just at the next
/// frame's own `sync_viewport_dims`/`prepare_frame`: a command dispatch that
/// switches tabs and then reads pane geometry in the same call (a scroll
/// bound after a tab-switch key, a Steel body chaining `(call! "goto-next-tab")`
/// onto a motion) would otherwise see the outgoing tab's stale viewport and
/// an un-rewound line store for the incoming one. This does not touch the
/// *hidden* tab's staleness model — a backgrounded pane is still untouched
/// until its own tab is next focused; this only closes the window for the
/// tab landing here, right now.
pub(in crate::editor) fn install_live(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: LayoutTree,
    focus: PaneId,
) {
    let _ = view.replace_layout(layout);
    view.resync_viewport_dims();
    view.begin_frame();
    super::focus::focus_pane(state, view, focus);
}

/// Switch the active tab to `target`, snapshotting the outgoing tab's live
/// layout/focus into its stash slot first. No-op if `target` is already
/// current — every caller (`:tabnext`/`:tabprev`, `Ctrl+p t`/`Ctrl+p T`, a
/// tabline click) can pass the current tab's own id without a special case.
pub(in crate::editor) fn switch_to_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    target: TabId,
) {
    if target == state.tabs.current() {
        return;
    }
    let (outgoing_layout, outgoing_focus) = take_live(state, view);
    let (layout, focus) = state.tabs.switch(outgoing_layout, outgoing_focus, target);
    install_live(state, view, layout, focus);
}
