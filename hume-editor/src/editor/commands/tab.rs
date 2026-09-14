//! Tab-page commands: `:tabnew`/`:tabclose` cores (the typed `:` wrappers
//! live in `typed_misc.rs`, next to `:split`/`:vsplit`, so they can share
//! `open_path_arg`) and the mappable `goto-next-tab`/`goto-prev-tab`.
//!
//! Each tab owns its own window layout (its own `LayoutTree` + focused
//! pane) — see `editor::tab`'s module doc for the full model. Panes are
//! never shared across tabs, but all live in the single global
//! `view.panes: SlotMap<PaneId, Pane>` regardless of which tab is active —
//! only `view.layout` (which panes are *reachable*) differs per tab.
//!
//! `goto-next-tab`/`goto-prev-tab` deliberately carry no `.jump()` meta,
//! unlike `goto-next-buffer`/`goto-prev-buffer`: a tab switch changes
//! `state.focus` itself (a different pane, possibly in a different tab), and
//! the jump list is written to `state.panes.jumps[state.focus.id()]` read
//! *after* the switch — the same reason `pane-focus-next`/`-left`/`-right`/
//! `-up`/`-down` (which also change `state.focus`) carry no `.jump()`
//! either, while buffer-switch commands (which keep the same pane) do.

use hume_engine::pipeline::{BufferId, EngineView};
use hume_ops::MotionMode;

use crate::editor::EditorState;
use crate::editor::error::CommandError;
use crate::editor::tab::{TabId, install_live, switch_to_tab, take_live};

use super::pane::{drop_pane_state, open_pane_as_new_tab};

/// `:tabnew [path]`'s core — open a fresh pane viewing `bid` in a brand new
/// tab inserted right after the current one (Vim's placement), and switch
/// to it. A new tab never inherits the source tab's cursor/scroll state —
/// it's a different window entirely, matching `:split <path>`'s
/// fresh-buffer semantics (see `split_pane_onto`'s doc), never a
/// same-buffer split's inherited-state semantics.
///
/// Thin delegate to `pane::open_pane_as_new_tab` — kept as its own
/// meaningfully-named entry point in this file (alongside `close_tab`,
/// `goto_tab_in_order`) rather than inlined at the one caller, matching how
/// every other typed tab-command core lives here.
pub(in crate::editor) fn open_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    bid: BufferId,
) -> TabId {
    open_pane_as_new_tab(state, view, bid).1
}

/// `:tabclose`'s core. Precondition: more than one tab exists — callers
/// check `state.tabs.len() > 1` first, reported to the user as "cannot close
/// the last tab page" rather than reaching this function at all. Also
/// `typed_quit`'s own tab-close step, for `:q` on a tab's last pane.
///
/// Commits the adjacent tab (Vim's placement — see `TabStore::adjacent`) as
/// live before dropping any pane state: `adjacent`/`close_current` both
/// assert `len() > 1`, so checking that precondition before anything is
/// destroyed keeps `view`/`state` consistent at every point in between.
///
/// Routes the closing tab's own layout/focus through `tab::take_live` rather
/// than reading `view.layout` directly — `take_live` ends the closing tab's
/// open Insert session at the one moment `view.layout`/`state.focus`
/// still jointly name it, before `install_live` swaps them to the survivor.
/// Left any later, the teardown's per-(pane, buffer) writes would land after
/// `drop_pane_state` has already removed the closing pane's own state below.
/// `close_current` only mutates `TabStore`, not `view`, so running it after
/// `take_live` is safe.
///
/// `closing_layout.into_detached()` is what makes discarding the closing
/// tab's tree and freeing every pane it reached a single step — there is no
/// window where the tree is gone but the pool still holds one of its panes,
/// or a token exists for a pane no tree ever named.
pub(in crate::editor) fn close_tab(state: &mut EditorState, view: &mut EngineView) {
    let survivor = state.tabs.adjacent();
    let (closing_layout, _outgoing_focus) = take_live(state, view);
    let (layout, focus) = state.tabs.close_current(survivor);
    install_live(state, view, layout, focus);
    for detached in closing_layout.into_detached() {
        drop_pane_state(state, view, detached);
    }
}

pub(super) enum TabStep {
    Next,
    Prev,
}

/// The one place a display-order tab step is taken — shared by the mappable
/// `goto-next-tab`/`goto-prev-tab` (`Ctrl+p t`/`Ctrl+p T`) and their typed
/// `:tabnext`/`:tabprev` spellings (`typed_misc::typed_tabnext`/`typed_tabprev`).
/// Mirrors `jump::goto_buffer_in_order`'s shape.
pub(super) fn goto_tab_in_order(state: &mut EditorState, view: &mut EngineView, step: TabStep) {
    let target = match step {
        TabStep::Next => state.tabs.next(),
        TabStep::Prev => state.tabs.prev(),
    };
    switch_to_tab(state, view, target);
}

// ── Mappable commands ───────────────────────────────────────────────────────

/// `tab-new` — the mappable sibling of `:tabnew` with no path argument, for
/// `bind-key!`/`call!` callers that have no typed-command dispatch path.
pub(crate) fn cmd_tab_new(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    open_tab(state, view, super::focused_buffer_id(state, view));
    Ok(())
}

/// `goto-next-tab` — switch to the next tab in display order.
pub(crate) fn cmd_goto_next_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_tab_in_order(state, view, TabStep::Next);
    Ok(())
}

/// `goto-prev-tab` — switch to the previous tab in display order.
pub(crate) fn cmd_goto_prev_tab(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_tab_in_order(state, view, TabStep::Prev);
    Ok(())
}
