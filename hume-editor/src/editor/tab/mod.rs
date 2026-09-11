//! Tab pages — Vim's sense: a tab is a saved window layout (its own
//! `LayoutTree` + its own focused pane), not a per-buffer strip. See
//! [`store`]'s module doc for the full model and why panes are never
//! duplicated per tab.
//!
//! This module holds the one chokepoint every tab *switch* goes through,
//! [`switch_to_tab`], plus [`take_live`]/[`install_live`] — the shared
//! exchange between `view.layout`/`state.focused_pane_id` (the live tab) and
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

/// Take the live layout/focus out of `view`/`state`, leaving a placeholder
/// behind — displaced immediately by the caller's own [`install_live`], so
/// never observable as the live layout. Shared first half of every op that
/// replaces the whole live tab.
///
/// Ends the outgoing pane's open Insert session first, before the
/// placeholder goes in: `view.layout` names the outgoing tab and
/// `state.focused_pane_id` names its focused pane together, right up to
/// this point — the last moment either is true until `install_live` runs.
/// `end_insert_session_if_active`'s own edit can shrink the outgoing pane's
/// buffer (the blank-line indent trim), and any per-(pane, buffer) state it
/// touches should resolve against a still-consistent (layout, focus) pair,
/// not against the placeholder `Leaf(PaneId::default())` below or a
/// half-installed incoming tab.
pub(in crate::editor) fn take_live(
    state: &mut EditorState,
    view: &mut EngineView,
) -> (LayoutTree, PaneId) {
    super::commands::end_insert_session_if_active(state, view);
    let focus = state.focused_pane_id;
    let placeholder = LayoutTree::Leaf(PaneId::default());
    // Swaps the live tab's whole tree out for a transient placeholder,
    // displaced immediately by the caller's own install_live — the pool is
    // untouched either way.
    (std::mem::replace(&mut view.layout, placeholder), focus)
}

/// Install `layout`/`focus` as the live tab. Shared second half of every op
/// that replaces `view.layout`/`state.focused_pane_id` together — the pair
/// that jointly define which tab is on screen. The focus half goes through
/// `commands::focus_pane` rather than a bare assignment: its Insert-session
/// teardown is already done by `take_live` above by the time this runs (a
/// no-op here as a result), but its paste-session commit isn't — that one
/// resolves the outgoing pane/buffer by id rather than through the layout
/// tree, so it has no matching earlier call and still needs to run here,
/// before `focused_pane_id` moves to `focus`.
pub(in crate::editor) fn install_live(
    state: &mut EditorState,
    view: &mut EngineView,
    layout: LayoutTree,
    focus: PaneId,
) {
    view.layout = layout;
    super::commands::focus_pane(state, view, focus);
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
