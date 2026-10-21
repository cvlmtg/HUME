//! Tab-page commands: `:tabnew`/`:tabclose` cores (the typed `:` wrappers
//! live in `typed_misc.rs`, next to `:split`/`:vsplit`, so they can share
//! `open_path_arg`) and the mappable `goto-next-tab`/`goto-prev-tab`.
//!
//! Each tab owns its own window layout (its own `LayoutTree` + focused
//! pane); see `editor::tab`'s module doc for the full model. Panes are
//! never shared across tabs, but all live in the single global
//! `view.panes: SlotMap<PaneId, Pane>` regardless of which tab is active:
//! only `view.layout` (which panes are *reachable*) differs per tab.
//!
//! `goto-next-tab`/`goto-prev-tab` carry no `.jump()` meta,
//! unlike `goto-next-buffer`/`goto-prev-buffer`: a tab switch changes
//! the focused pane itself (a different pane, possibly in a different tab),
//! and the jump list is written to `state.panes.entries[view.focused_pane()].jumps`
//! read *after* the switch, the same reason `pane-focus-next`/`-prev`/`-left`/
//! `-right`/`-up`/`-down` (which also change the focused pane) carry no `.jump()`
//! either, while buffer-switch commands (which keep the same pane) do.

use crate::editor::registry::Count;
use hume_engine::pipeline::{Cycle, EngineView};
use hume_ops::MotionMode;

use crate::editor::EditorState;
use crate::editor::error::CommandError;
use crate::editor::tab::{SingleTab, switch_to_tab};

use super::FocusedPane;
use super::pane::{CloseRefused, check_no_pipe_from, drop_pane_state, open_pane_as_new_tab};

/// `:tabclose`'s core, and `typed_quit`'s tab-close step for `:q` on a
/// tab's last pane: close the current tab and free every pane it reached.
/// Refused, with nothing changed, when it is the only tab, or when one of
/// its panes started a pipe run still in flight.
///
/// The adjacent tab (Vim's placement, see `TabStore::adjacent`) goes live
/// before any pane state is dropped. `focus::install_layout` ends the
/// closing tab's open Insert/paste session while `view` still holds that
/// tab's layout. Left any later, the teardown's per-(pane, buffer) writes
/// would land after `drop_pane_state` has already removed the closing
/// pane's own state below.
///
/// The closed tab's layout comes back only as its detached panes, one
/// token per pane its tree reached, so every pane the tab held is freed.
pub(in crate::editor) fn close_tab(
    state: &mut EditorState,
    view: &mut EngineView,
) -> Result<(), CloseRefused> {
    if state.tabs.len() == 1 {
        return Err(CloseRefused::Last);
    }
    check_no_pipe_from(state, &view.active_pane_ids())?;
    let detached = crate::editor::tab::close_tab(state, view)
        .unwrap_or_else(|SingleTab| unreachable!("one of several tabs closes"));
    for detached in detached {
        drop_pane_state(state, view, detached);
    }
    Ok(())
}

/// The one place a display-order tab step is taken. Mirrors
/// `jump::goto_buffer_in_order`'s shape.
pub(super) fn goto_tab_in_order(state: &mut EditorState, view: &mut EngineView, cycle: Cycle) {
    let target = state.tabs.step(cycle);
    switch_to_tab(state, view, target);
}

// ── Mappable commands ───────────────────────────────────────────────────────

/// `tab-new`: the mappable sibling of `:tabnew` with no path argument, for
/// `bind-key!`/`call!` callers that have no typed-command dispatch path.
pub(crate) fn cmd_tab_new(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = fp.bid(view);
    open_pane_as_new_tab(state, view, bid);
    Ok(())
}

/// `goto-next-tab`: switch to the next tab in display order.
pub(crate) fn cmd_goto_next_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_tab_in_order(state, view, Cycle::Next);
    Ok(())
}

/// `goto-prev-tab`: switch to the previous tab in display order.
pub(crate) fn cmd_goto_prev_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_tab_in_order(state, view, Cycle::Prev);
    Ok(())
}
