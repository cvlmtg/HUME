//! Applying a setting change: the single production path.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (`:set`, `:theme`) and the Steel
//! builtins (`set-option!`, which receives individual `&mut` references via
//! `SteelCtx`/`EditorHostImpl` rather than a whole `&mut Editor`).
//!
//! [`apply_global`](crate::editor::settings::ops::apply_global)/
//! [`apply_buffer_target`](crate::editor::settings::ops::apply_buffer_target)
//! are the only places production code should write a setting: they write the
//! raw value via [`write_global`]/[`write_buffer`] and then resync whatever
//! derived state depends on it (each [`ResyncKey`]'s effect, applied by
//! `resync_derived_state`). Calling `write_global`/`write_buffer` alone would
//! skip those, so write and resync must not be split apart.
//! A language write is the exception: `apply_buffer_target` parses it and
//! returns the change for its caller to make, since it activates plugins.
//! [`read_buffer_target`](crate::editor::settings::ops::read_buffer_target)
//! reads back what a buffer holds, the line ending and language included.
//!
//! A child module of `settings`, not a sibling. See `settings`'s own doc
//! for why: it lets `write_global`/`write_buffer` narrow to
//! `pub(in crate::editor::settings)`, reachable from exactly this module and
//! `settings::tests`, rather than from every file under `crate::editor`.

#[cfg(any(test, feature = "test-util"))]
use hume_core::OptionValue;
use hume_core::{
    BufferId, BufferIntrinsicOption, BufferOptionWrite, ConfigOrigin, LanguageOptionText,
    OptionInput,
};
use hume_editing::text::LineEnding;
use hume_engine::pipeline::EngineView;

#[cfg(any(test, feature = "test-util"))]
use super::OptionAccess;
use super::{
    BufferKey, BufferTarget, GlobalKey, ResyncKey, SettingText, Shape, THEME, TableKey, kind,
};
use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::theme;

/// Write a global setting from `by` and resync every piece of derived
/// state that depends on it. A write a higher origin's value shadows
/// changes nothing and is reported at Trace.
pub(in crate::editor) fn apply_global(
    state: &mut EditorState,
    view: &mut EngineView,
    key: TableKey,
    value: &OptionInput,
    by: ConfigOrigin,
) -> Result<(), String> {
    // Read before the write, for the theme rollback below.
    let held_theme = state.settings.origins.slot(TableKey::Global(THEME));
    if !super::write_global(key, value, &mut state.settings, by)? {
        state.report_shadowed("option", format_args!("'{}' value", key.name()));
        return Ok(());
    }

    let resynced = match key {
        TableKey::Global(key) => key
            .resync()
            .is_none_or(|rk| resync_derived_state(state, view, rk)),
        TableKey::Buffer(_) => true,
    };
    if !resynced && key == TableKey::Global(THEME) {
        // Theme is the only effect that can fail after a successful write.
        // The setting goes back to the theme still on screen, and to the
        // origin that set it.
        let shown = state.shown_theme.clone();
        let failed_theme = std::mem::replace(&mut state.settings.theme, shown);
        state
            .settings
            .origins
            .restore(TableKey::Global(THEME), held_theme);
        return Err(format!(
            "theme '{failed_theme}' failed to load (see :messages)"
        ));
    }

    // Raised after the write (and any resync) succeeds: a plugin reacting
    // to this sees the setting already in its new, live state. The single
    // raise site for every `:set global`/`set-option!`/`:theme` write, since
    // this is the single write path all three funnel through (see the
    // module doc). A plugin owning one setting's policy (e.g. `core:lsp`
    // re-cutting diagnostics on `lsp-diagnostics-min-severity`) needs
    // one hook, not one per write path.
    state.queue_event(crate::editor::event::EditorEvent::OnOptionChange {
        key: key.name().to_string(),
        value: super::read_global(key, &state.settings).value(),
    });

    Ok(())
}

/// Write the global setting a raw `key` names with no resync, for
/// `testing::MockHost`, which has no editor state to resync, and
/// `settings::tests`.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn write_global_for_test(
    key: &str,
    value: &OptionInput,
    settings: &mut crate::editor::settings::EditorSettings,
    by: ConfigOrigin,
) -> Result<bool, String> {
    let key = super::global_key(key, OptionAccess::Write)?;
    super::write_global(key, value, settings, by)
}

/// The global value a raw `key` names as `get-option` reads it, for
/// `testing::MockHost`.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn global_option(
    key: &str,
    settings: &crate::editor::settings::EditorSettings,
) -> Result<OptionValue, String> {
    super::global_key(key, OptionAccess::Read).map(|key| super::read_global(key, settings).value())
}

/// The value a raw `key` names on a buffer with no overrides, as
/// `get-buffer-option` reads it, for `testing::MockHost`, which models no
/// buffers.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn buffer_option_for_test(
    key: &str,
    settings: &crate::editor::settings::EditorSettings,
) -> Result<OptionValue, String> {
    match super::buffer_target(key, OptionAccess::Read)? {
        BufferTarget::Override(key) => {
            let overrides = crate::editor::settings::BufferOverrides::default();
            Ok(super::read_buffer(key, settings, &overrides).value())
        }
        BufferTarget::Intrinsic(opt) => Err(format!(
            "'{}' is stored on a buffer, which the test host does not model",
            opt.name()
        )),
    }
}

/// Reset every global setting to its compiled-in default and rerun every
/// `resync: true` effect against the reset values. Called by
/// `:reload-config`'s reset so a runtime `:set global`/`:theme` change (or
/// one applied by the previous `init.scm`) never survives a reload.
///
/// `state.settings = EditorSettings::default()` alone would leave
/// `view.theme` still baked with the old theme, so the view gets the
/// compiled-in fallback `Editor::bootstrap` starts with. The default theme is not
/// loaded here: `init_scripting` applies it after `init.scm`, so a configured
/// theme that fails to load ends on the fallback rather than on the default.
pub(in crate::editor) fn reset_globals(state: &mut EditorState, view: &mut EngineView) {
    state.settings = crate::editor::settings::EditorSettings::default();
    theme::set_theme(
        view,
        &mut state.shown_theme,
        &mut state.input,
        crate::editor::theme::fallback_theme(),
        "",
    );
    for rk in GlobalKey::ALL.iter().filter_map(|key| key.resync()) {
        if rk != ResyncKey::theme {
            resync_derived_state(state, view, rk);
        }
    }
}

/// Load the `theme` setting if config left it naming something other than
/// the theme on screen. Run once after `init.scm`; a theme `init.scm` set has
/// already loaded, so this only ever loads the default config never chose.
/// When that theme is not on disk the setting becomes empty, matching the
/// fallback on screen.
pub(in crate::editor) fn apply_pending_theme(state: &mut EditorState, view: &mut EngineView) {
    if state.settings.theme == state.shown_theme {
        return;
    }
    theme::load_default_theme(
        view,
        &mut state.shown_theme,
        &mut state.message_log,
        &mut state.status_msg,
        &mut state.input,
        &state.dirs,
        &state.settings.theme,
    );
    state.settings.theme.clone_from(&state.shown_theme);
}

/// Write `target` on `bid` from `by`: the one write path for `:set buffer`
/// and `set-buffer-option!`. A `language` value is parsed and returned as
/// the change it names, not written: the change activates plugins, so `:set`
/// makes it and a script's write is queued, both ranked as `by` then.
pub(in crate::editor) fn apply_buffer_target(
    state: &mut EditorState,
    bid: BufferId,
    target: BufferTarget,
    value: &OptionInput,
    by: ConfigOrigin,
) -> Result<BufferOptionWrite, String> {
    match target {
        BufferTarget::Override(key) => {
            apply_buffer(state, bid, key, value, by).map(|()| BufferOptionWrite::Applied)
        }
        BufferTarget::Intrinsic(BufferIntrinsicOption::LineEnding) => {
            set_line_ending(state, bid, value, by).map(|()| BufferOptionWrite::Applied)
        }
        BufferTarget::Intrinsic(opt @ BufferIntrinsicOption::Language) => {
            let text = kind::parse_as::<kind::FreeText, String>(opt.name(), value)?;
            let language = LanguageOptionText::decode(&text).map(str::to_owned);
            Ok(BufferOptionWrite::Language(language))
        }
    }
}

/// `target`'s value on `buf`: the one reader for `:get buffer` and
/// `get-buffer-option`.
pub(in crate::editor) fn read_buffer_target(
    state: &EditorState,
    buf: &Buffer,
    target: BufferTarget,
) -> Shape {
    match target {
        BufferTarget::Override(key) => super::read_buffer(key, &state.settings, &buf.overrides),
        BufferTarget::Intrinsic(opt) => Shape::Text(intrinsic_text(state, buf, opt)),
    }
}

/// Write a buffer-scoped setting override from `by`. No buffer-scoped key
/// has a derived-state effect today (see [`super::write_buffer`]'s doc), so
/// unlike [`apply_global`] there is nothing to resync here. A write a higher
/// origin's override shadows changes nothing and is reported at Trace.
fn apply_buffer(
    state: &mut EditorState,
    bid: BufferId,
    key: BufferKey,
    value: &OptionInput,
    by: ConfigOrigin,
) -> Result<(), String> {
    if !super::write_buffer(key, value, &mut state.buffers.get_mut(bid).overrides, by)? {
        state.report_shadowed("option", format_args!("'{}' value", key.name()));
    }
    Ok(())
}

/// Parse `value` as a line ending and set it on `bid`'s text from `by`. It
/// is not an override: it changes whether the buffer reads as unsaved, and
/// is refused on a read-only buffer. A write a higher origin's line ending
/// shadows is reported at Trace.
fn set_line_ending(
    state: &mut EditorState,
    bid: BufferId,
    value: &OptionInput,
    by: ConfigOrigin,
) -> Result<(), String> {
    let key = BufferIntrinsicOption::LineEnding.name();
    let line_ending = kind::parse_as::<kind::Typed, LineEnding>(key, value)?;
    if !state
        .buffers
        .get_mut(bid)
        .set_line_ending(line_ending, by)?
    {
        state.report_shadowed("option", format_args!("'{key}' value"));
    }
    Ok(())
}

/// `opt`'s value on `buf` as the text `:set buffer` takes back.
fn intrinsic_text(state: &EditorState, buf: &Buffer, opt: BufferIntrinsicOption) -> String {
    match opt {
        BufferIntrinsicOption::Language => {
            LanguageOptionText::encode(buf.language_name(&state.config.languages)).to_owned()
        }
        BufferIntrinsicOption::LineEnding => buf.text().line_ending().to_text(),
    }
}

/// Resync derived state after a successful [`super::write_global`]
/// for the key `rk` decodes. Returns `false` if an effect failed (theme load
/// only): the caller rolls the setting back so a bad value never persists.
///
/// Exhaustive over [`ResyncKey`]. See that type's own doc for why a new
/// `resync: true` declaration with no arm here fails to compile.
fn resync_derived_state(state: &mut EditorState, view: &mut EngineView, rk: ResyncKey) -> bool {
    match rk {
        ResyncKey::prompt_history_levels => {
            state
                .history
                .set_capacity(state.settings.prompt_history_levels);
            true
        }
        ResyncKey::undo_levels => {
            state
                .buffers
                .set_undo_levels_all(state.settings.undo_levels);
            true
        }
        ResyncKey::jump_list_capacity => {
            for entry in state.panes.entries.values_mut() {
                entry.jumps.set_capacity(state.settings.jump_list_capacity);
            }
            true
        }
        ResyncKey::theme if !state.settings.theme.is_empty() => theme::load_theme_by_name(
            view,
            &mut state.shown_theme,
            &mut state.message_log,
            &mut state.status_msg,
            &mut state.input,
            &state.dirs,
            &state.settings.theme,
        ),
        // Empty theme: the compiled-in fallback.
        ResyncKey::theme => {
            theme::set_theme(
                view,
                &mut state.shown_theme,
                &mut state.input,
                crate::editor::theme::fallback_theme(),
                "",
            );
            true
        }
    }
}
