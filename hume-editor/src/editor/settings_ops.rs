//! Free functions for applying a setting change — the single production path.
//!
//! Extracted from `impl Editor` so the same logic can be called by both the
//! `Editor` methods (`:set`, `:theme`) and the Steel builtins (`set-option!`,
//! which receives individual `&mut` references via `SteelCtx`/`EditorHostImpl`
//! rather than a whole `&mut Editor`).
//!
//! [`apply`] is the only place production code should write a setting: it
//! writes the raw value via [`crate::settings::write_setting`] and then
//! resyncs whatever derived state depends on it (the undo-tree cap on every
//! open buffer, the minibuffer prompt-history capacity, the loaded theme).
//! `write_setting` alone would silently skip those — that split used to be
//! two steps a caller had to remember to pair, and the Steel path forgot.

use hume_engine::pipeline::{BufferId, EngineView};

use crate::editor::EditorState;
use crate::editor::theme;
use crate::settings::{BufferOverrides, SettingScope};

/// Write a setting and resync every piece of derived state that depends on it.
///
/// `bid` supplies the buffer whose [`BufferOverrides`] a `Text`-scope write
/// lands in; pass `None` for a `Global`-scope write with no acting buffer
/// (the Steel `set-option!` path, which is global-only).
pub(crate) fn apply(
    state: &mut EditorState,
    view: &mut EngineView,
    scope: SettingScope,
    key: &str,
    value: &str,
    bid: Option<BufferId>,
) -> Result<(), String> {
    // Theme is the only effect that can fail after a successful write, so it's
    // the only one that needs a value to roll back to.
    let prev_theme = (key == "theme").then(|| state.settings.theme.clone());

    let mut scratch = BufferOverrides::default();
    let result = {
        let settings = &mut state.settings;
        let overrides = match bid {
            Some(id) => &mut state.buffers.get_mut(id).overrides,
            None => &mut scratch,
        };
        crate::settings::write_setting(scope, key, value, settings, overrides)
    };

    if result.is_ok()
        && !resync_derived_state(state, view, key)
        && let Some(prev) = prev_theme
    {
        state.settings.theme = prev;
    }

    result
}

/// Resync derived state after a successful [`crate::settings::write_setting`]
/// for `key`. Returns `false` if an effect failed (theme load only) — the
/// caller rolls the setting back so a bad value never persists.
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
        "theme" if !state.settings.theme.is_empty() => theme::load_theme_by_name(
            view,
            &mut state.message_log,
            &mut state.status_msg,
            &state.settings.theme,
        ),
        _ => true,
    }
}
