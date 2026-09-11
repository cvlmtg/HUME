//! Applying a setting change — the single production path.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (`:set`, `:theme`) and the Steel
//! builtins (`set-option!`, which receives individual `&mut` references via
//! `SteelCtx`/`EditorHostImpl` rather than a whole `&mut Editor`).
//!
//! [`apply_global`]/[`apply_buffer`] are the only places production code
//! should write a setting: they write the raw value via
//! [`super::write_global`]/[`super::write_buffer`] and then resync whatever
//! derived state depends on it (the undo-tree cap on every open buffer, the
//! minibuffer prompt-history capacity, every open pane's jump-list capacity,
//! the loaded theme). Calling `write_global`/`write_buffer` alone would
//! silently skip those — write and resync must not be split apart.
//!
//! A child module of `settings`, not a sibling — see `settings`'s own doc
//! for why: it lets `write_global`/`write_buffer` narrow to
//! `pub(in crate::editor::settings)`, reachable from exactly this module and
//! `settings::tests`, rather than from every file under `crate::editor`.

use hume_engine::pipeline::{BufferId, EngineView};

use super::{ResyncKey, THEME_KEY, resync_key};
use crate::editor::EditorState;
use crate::editor::theme;

/// Write a global setting and resync every piece of derived state that
/// depends on it.
pub(in crate::editor) fn apply_global(
    state: &mut EditorState,
    view: &mut EngineView,
    key: &str,
    value: &str,
) -> Result<(), String> {
    // Theme is the only effect that can fail after a successful write, so it's
    // the only one that needs a value to roll back to.
    let prev_theme = (key == THEME_KEY).then(|| state.settings.theme.clone());

    super::write_global(key, value, &mut state.settings)?;

    let resynced = resync_key(key).is_none_or(|rk| resync_derived_state(state, view, rk));
    if !resynced && let Some(prev) = prev_theme {
        let failed_theme = std::mem::replace(&mut state.settings.theme, prev);
        return Err(format!(
            "theme '{failed_theme}' failed to load — see :messages for details"
        ));
    }

    // Raised after the write (and any resync) succeeds — a plugin reacting
    // to this sees the setting already in its new, live state. The single
    // raise site for every `:set global`/`set-option!`/`:theme` write, since
    // this is the single write path all three funnel through (see the
    // module doc) — a plugin owning one setting's policy (e.g. the LSP
    // inlay-hints plugin) needs exactly one hook, not one per write path.
    state.queue_event(crate::editor::event::EditorEvent::OnOptionChange {
        key: key.to_string(),
        value: value.to_string(),
    });

    Ok(())
}

/// Write a global setting's raw value with no resync — [`super::write_global`]
/// itself, exposed at crate visibility for `testing::MockHost` only.
///
/// `write_global` is `pub(in crate::editor::settings)`: every production
/// write goes through [`apply_global`] above, which has the
/// `EditorState`/`EngineView` this function's resync needs. `MockHost`
/// models neither — it has no history rings, buffers, or view to resync
/// derived state against — so it needs the raw writer directly, the same way
/// `apply_global` does before its own resync step, but from outside
/// `crate::editor::settings` where `MockHost` lives. This is that one
/// `#[cfg]`-gated forwarding call — it does not exist in a production
/// build — not a widening of `write_global`'s own visibility.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn write_global_for_test(
    key: &str,
    value: &str,
    settings: &mut crate::editor::settings::EditorSettings,
) -> Result<(), String> {
    super::write_global(key, value, settings)
}

/// Reset every global setting to its compiled-in default and rerun every
/// `resync: true` effect against the reset values — called by
/// `:reload-config`'s reset so a runtime `:set global`/`:theme` change (or
/// one applied by the previous `init.scm`) never survives a reload.
///
/// `state.settings = EditorSettings::default()` alone would leave
/// `view.theme` still baked with the old theme: `theme`'s default is the
/// empty string, and `resync_derived_state`'s `ResyncKey::theme` arm
/// deliberately no-ops on empty (nothing to load), so the view's theme is
/// set directly to the same compiled-in default `Editor::open` uses instead
/// of relying on that arm.
pub(in crate::editor) fn reset_globals(state: &mut EditorState, view: &mut EngineView) {
    state.settings = crate::editor::settings::EditorSettings::default();
    view.theme = crate::editor::theme::build_default_theme();
    // Same reasoning as the `ResyncKey::theme` arm below: `view.theme` just
    // changed, so a baked popup's styles must be rebuilt against it.
    if let Some(popup) = state.config.popup.as_mut() {
        popup.content = None;
    }
    for &key in crate::editor::settings::all_setting_keys() {
        if let Some(rk) = resync_key(key) {
            resync_derived_state(state, view, rk);
        }
    }
}

/// Write a buffer-scoped setting override. No buffer-scoped key has a
/// derived-state effect today (see [`super::write_buffer`]'s doc), so unlike
/// [`apply_global`] there is nothing to resync here.
pub(in crate::editor) fn apply_buffer(
    state: &mut EditorState,
    bid: BufferId,
    key: &str,
    value: &str,
) -> Result<(), String> {
    super::write_buffer(key, value, &mut state.buffers.get_mut(bid).overrides)
}

/// Resync derived state after a successful [`super::write_global`]
/// for the key `rk` decodes. Returns `false` if an effect failed (theme load
/// only) — the caller rolls the setting back so a bad value never persists.
///
/// Exhaustive over [`ResyncKey`] — see that type's own doc for why a new
/// `resync: true` declaration with no arm here fails to compile.
fn resync_derived_state(state: &mut EditorState, view: &mut EngineView, rk: ResyncKey) -> bool {
    match rk {
        ResyncKey::history_capacity => {
            state.history.set_capacity(state.settings.history_capacity);
            true
        }
        ResyncKey::undo_levels => {
            state
                .buffers
                .set_undo_levels_all(state.settings.undo_levels);
            true
        }
        ResyncKey::jump_list_capacity => {
            state
                .panes
                .jumps
                .set_capacity(state.settings.jump_list_capacity);
            true
        }
        ResyncKey::theme if !state.settings.theme.is_empty() => {
            let loaded = theme::load_theme_by_name(
                view,
                &mut state.message_log,
                &mut state.status_msg,
                &state.settings.theme,
            );
            // A successful load replaces `view.theme` above — an open
            // popup's baked styles (`PopupModel::content_mut`) must be
            // rebuilt against it, not left holding the outgoing theme's
            // colors until an unrelated width change forces a re-wrap.
            if loaded && let Some(popup) = state.config.popup.as_mut() {
                popup.content = None;
            }
            loaded
        }
        // Empty theme (cleared, or never set): nothing to load.
        ResyncKey::theme => true,
    }
}
