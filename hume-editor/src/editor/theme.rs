//! Theme loading applied to a live editor.

use hume_core::{ConfigOrigin, ScopeStyleInput};
use hume_engine::pipeline::EngineView;
use hume_engine::theme::error::ThemeError;
use hume_engine::theme::loader::{LoadedTheme, load_theme};
use hume_engine::theme::style_spec::StyleSpec;

use crate::editor::EditorState;
use hume_core::Severity;

/// Replace `view.theme`, record the name it came from in `shown_theme` (empty
/// for the fallback), layer the scripts' scope styles onto it, and rebuild or
/// drop everything built against its baked colors: each input-stack layer
/// handles its own ([`crate::editor::input_stack::InputStack::theme_changed`]).
/// The single chokepoint for replacing a *live* `view.theme`, so no
/// replacement site can skip those steps. `Editor::bootstrap`'s initial
/// construction bypasses this: there is no layer and no script style yet.
///
/// `load_warnings` are the theme file's own; they are reported together
/// with any scope style color the theme does not resolve.
fn install_theme(
    state: &mut EditorState,
    view: &mut EngineView,
    theme: hume_engine::theme::Theme,
    name: &str,
    mut load_warnings: Vec<ThemeError>,
) {
    view.theme = theme;
    name.clone_into(&mut state.shown_theme);
    load_warnings.extend(apply_scope_styles(state, view));
    let count = load_warnings.len();
    report_warnings(
        state,
        load_warnings,
        format!(
            "{} loaded with {count} warning{}",
            theme_label(name),
            plural(count)
        ),
    );
}

/// [`install_theme`] for a theme with no load warnings of its own.
pub(in crate::editor) fn set_theme(
    state: &mut EditorState,
    view: &mut EngineView,
    theme: hume_engine::theme::Theme,
    name: &str,
) {
    install_theme(state, view, theme, name, Vec::new());
}

/// `(set-scope-styles! styles)` from `by`: refuses the whole call for one
/// malformed entry, before any is stored. The live theme takes the styles
/// at once unless a theme install is pending, which takes them itself:
/// resolved now, a palette name would warn against a theme about to go.
pub(in crate::editor) fn set_scope_styles(
    state: &mut EditorState,
    view: &mut EngineView,
    styles: &[ScopeStyleInput],
    by: ConfigOrigin,
) -> Result<(), String> {
    let specs = styles
        .iter()
        .map(|input| StyleSpec::from_input(input).map(|spec| (input.scope.as_str(), spec)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (scope, spec) in specs {
        if state.config.scope_styles.set(scope, spec, by) {
            state.report_shadowed("set-scope-styles!", format_args!("'{scope}' style"));
        }
    }
    if state.theme_install_pending() {
        return Ok(());
    }
    let warnings = apply_scope_styles(state, view);
    let count = warnings.len();
    let summary = format!(
        "{count} scope style color{} not in {}",
        plural(count),
        theme_label(&state.shown_theme)
    );
    report_warnings(state, warnings, summary);
    Ok(())
}

/// Layer the scripts' scope styles onto `view.theme` and rebuild what the
/// input-stack layers built against its colors. Returns a warning for each
/// color the theme does not resolve.
fn apply_scope_styles(state: &mut EditorState, view: &mut EngineView) -> Vec<ThemeError> {
    let warnings = view.theme.apply_scope_styles(&state.config.scope_styles);
    state.input.theme_changed(&mut view.theme, &view.registry);
    warnings
}

/// Log each warning, and show `summary` in the status bar when there is one.
fn report_warnings(state: &mut EditorState, warnings: Vec<ThemeError>, summary: String) {
    if warnings.is_empty() {
        return;
    }
    for warning in &warnings {
        state
            .message_log
            .push(Severity::Warning, warning.to_string());
    }
    state.status_msg.show(Severity::Warning, summary);
}

fn theme_label(name: &str) -> String {
    if name.is_empty() {
        "the built-in fallback theme".to_owned()
    } else {
        format!("theme '{name}'")
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Load a theme by name and apply it to the engine view.
///
/// Searches `themes/<name>.toml` under the config, data, then runtime dir.
///
/// Malformed entries (a bad color, an unsupported scope shape) only produce
/// warnings: the theme still loads and replaces the current one. A
/// document-level failure (unreadable TOML, a missing `inherits` parent)
/// leaves the current theme unchanged; see
/// [`hume_engine::theme::loader::LoadedTheme`]. `prepare_frame` bakes the new
/// theme before the next render.
///
/// Each warning goes to `message_log`, with a one-line count in `status_msg`.
/// A document-level failure shows its own message in `status_msg` and
/// leaves `input` untouched. Returns `true` unless the load failed outright.
pub(in crate::editor) fn load_theme_by_name(
    state: &mut EditorState,
    view: &mut EngineView,
    name: &str,
) -> bool {
    let result = load_theme(name, &super::theme_search_paths(&state.dirs));
    install_load_result(state, view, result, name)
}

/// [`load_theme_by_name`] for the default theme config never chose. A theme
/// file that is not there is the expected way to end up on the fallback, so it
/// logs a Trace line and leaves the view alone; any other failure is reported
/// like a named theme's.
pub(in crate::editor) fn load_default_theme(
    state: &mut EditorState,
    view: &mut EngineView,
    name: &str,
) {
    match load_theme(name, &super::theme_search_paths(&state.dirs)) {
        Err(ThemeError::NotFound { name: missing }) if missing == name => {
            state.message_log.push(
                Severity::Trace,
                format!("theme '{name}' not found, using the built-in fallback"),
            );
        }
        result => {
            install_load_result(state, view, result, name);
        }
    }
}

fn install_load_result(
    state: &mut EditorState,
    view: &mut EngineView,
    result: Result<LoadedTheme, ThemeError>,
    name: &str,
) -> bool {
    match result {
        Ok(loaded) => {
            install_theme(state, view, loaded.theme, name, loaded.warnings);
            true
        }
        Err(e) => {
            let text = e.to_string();
            state.status_msg.show(Severity::Warning, text.clone());
            state.message_log.push(Severity::Warning, text);
            false
        }
    }
}

// ── Engine theme builder ──────────────────────────────────────────────────────

// Scope names and palette values live in `assets/fallback-theme.toml`. It is
// compiled in only and never shipped to the user's disk, so it is independent
// of the bundled `sand` theme.
const FALLBACK_THEME_TOML: &str = include_str!("../../assets/fallback-theme.toml");

/// Parse and return the fallback engine [`hume_engine::theme::Theme`] from the embedded TOML.
///
/// Installed until a named theme loads, and again when the configured theme
/// is missing or empty. The content is `assets/fallback-theme.toml`, embedded
/// at compile time via `include_str!`, so editing that file requires a
/// rebuild to change the fallback.
pub(in crate::editor) fn fallback_theme() -> hume_engine::theme::Theme {
    let loaded = hume_engine::theme::loader::parse_theme(FALLBACK_THEME_TOML)
        .expect("embedded fallback-theme.toml must parse: file is compile-time embedded");
    // Unlike a user's own theme, fallback-theme.toml is HUME's shipped content: a
    // warning here is a bug in this repo, not a typo to shrug off, so it's
    // stated as an invariant at the one site that would otherwise drop it
    // silently (`load_theme_by_name` surfaces the same warnings for every
    // other load path).
    // A real `assert!`, not `debug_assert!`: this runs once at startup over
    // content fixed at compile time, so it costs nothing, and a `debug_assert`
    // would drop in release exactly the builds where a shipped-theme mistake
    // reaches users. `crate::testing::build_snapshot_theme` asserts the same way.
    assert!(
        loaded.warnings.is_empty(),
        "embedded fallback-theme.toml produced load warnings: {:?}",
        loaded.warnings
    );
    loaded.theme
}

impl EditorState {
    /// Whether the `theme` setting names a theme other than the one shown,
    /// which `apply_pending_theme` installs once `init.scm` has run.
    pub(in crate::editor) fn theme_install_pending(&self) -> bool {
        self.settings.theme != self.shown_theme
    }
}
