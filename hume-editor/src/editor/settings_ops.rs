//! Free functions for applying a setting change — the single production path.
//!
//! Extracted from `impl Editor` so the same logic can be called by both the
//! `Editor` methods (`:set`, `:theme`) and the Steel builtins (`set-option!`,
//! which receives individual `&mut` references via `SteelCtx`/`EditorHostImpl`
//! rather than a whole `&mut Editor`).
//!
//! [`apply_global`]/[`apply_buffer`] are the only places production code
//! should write a setting: they write the raw value via
//! [`crate::settings::write_global`]/[`crate::settings::write_buffer`] and
//! then resync whatever derived state depends on it (the undo-tree cap on
//! every open buffer, the minibuffer prompt-history capacity, every open
//! pane's jump-list capacity, the loaded theme). Calling `write_global`/
//! `write_buffer` alone would silently skip those — that split used to be
//! two steps a caller had to remember to pair, and the Steel path forgot.

use hume_engine::pipeline::{BufferId, EngineView};

use crate::editor::EditorState;
use crate::editor::theme;

/// Write a global setting and resync every piece of derived state that
/// depends on it.
pub(crate) fn apply_global(
    state: &mut EditorState,
    view: &mut EngineView,
    key: &str,
    value: &str,
) -> Result<(), String> {
    // Theme is the only effect that can fail after a successful write, so it's
    // the only one that needs a value to roll back to.
    let prev_theme = (key == "theme").then(|| state.settings.theme.clone());

    crate::settings::write_global(key, value, &mut state.settings)?;

    if !resync_derived_state(state, view, key)
        && let Some(prev) = prev_theme
    {
        state.settings.theme = prev;
    }

    Ok(())
}

/// Write a buffer-scoped setting override. No buffer-scoped key has a
/// derived-state effect today (see [`crate::settings::write_buffer`]'s doc),
/// so unlike [`apply_global`] there is nothing to resync here.
pub(crate) fn apply_buffer(
    state: &mut EditorState,
    bid: BufferId,
    key: &str,
    value: &str,
) -> Result<(), String> {
    crate::settings::write_buffer(key, value, &mut state.buffers.get_mut(bid).overrides)
}

/// Resync derived state after a successful [`crate::settings::write_global`]
/// for `key`. Returns `false` if an effect failed (theme load only) — the
/// caller rolls the setting back so a bad value never persists.
///
/// The fallthrough arm's `debug_assert!` is the cross-check against
/// `settings.rs`'s `resync: true` declarations: a global key that declares a
/// resync effect but has no matching arm here panics immediately in any
/// debug build or test run, instead of the effect silently never firing.
fn resync_derived_state(state: &mut EditorState, view: &mut EngineView, key: &str) -> bool {
    match key {
        "history-capacity" => {
            state.history.set_capacity(state.settings.history_capacity);
            true
        }
        "undo-levels" => {
            state
                .buffers
                .set_undo_levels_all(state.settings.undo_levels);
            true
        }
        "jump-list-capacity" => {
            let capacity = state.settings.jump_list_capacity;
            for jumps in state.panes.jumps.values_mut() {
                jumps.set_capacity(capacity);
            }
            true
        }
        "theme" if !state.settings.theme.is_empty() => theme::load_theme_by_name(
            view,
            &mut state.message_log,
            &mut state.status_msg,
            &state.settings.theme,
        ),
        // Empty theme (cleared, or never set): nothing to load, and this
        // must not fall through to the `_` arm below — "theme" declares
        // `resync: true`, so the debug_assert there would fire.
        "theme" => true,
        _ => {
            debug_assert!(
                !crate::settings::has_declared_resync(key),
                "'{key}' declares `resync: true` in define_settings! but \
                 resync_derived_state has no matching arm for it"
            );
            true
        }
    }
}
