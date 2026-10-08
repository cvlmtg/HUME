//! Theme loading applied to a live editor.

use hume_engine::pipeline::EngineView;
use hume_engine::theme::error::ThemeError;
use hume_engine::theme::loader::{LoadedTheme, load_theme};

use crate::editor::input_stack::PopupLayer;
use crate::editor::message_log::{MessageLog, Severity};

/// Replace `view.theme`, record the name it came from in `shown_theme` (empty
/// for the fallback), and invalidate everything that caches against its baked
/// colors: currently just an open popup's per-width/style cache
/// (`PopupLayer::content`), the one input to that cache besides `text`/
/// `syntax` (which never change during a popup's lifetime) that can change
/// out from under it. The single chokepoint for replacing a *live*
/// `view.theme`, so a future third replacement site can't forget the
/// invalidation the way a hand-placed `popup.content = None` next to each
/// write site could. `Editor::open`'s initial construction bypasses this on
/// purpose; there is no popup yet to invalidate.
pub(in crate::editor) fn set_theme(
    view: &mut EngineView,
    shown_theme: &mut String,
    popup: Option<&mut PopupLayer>,
    theme: hume_engine::theme::Theme,
    name: &str,
) {
    view.theme = theme;
    name.clone_into(shown_theme);
    if let Some(popup) = popup {
        popup.content = None;
    }
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
/// A document-level failure writes its own message to `status_msg` and
/// leaves `popup` untouched. Returns `true` unless the load failed outright.
///
/// The `Editor` fields are passed separately so the caller can borrow
/// `&editor.settings.theme` for `name` without cloning.
pub(in crate::editor) fn load_theme_by_name(
    engine_view: &mut EngineView,
    shown_theme: &mut String,
    message_log: &mut MessageLog,
    status_msg: &mut Option<String>,
    popup: Option<&mut PopupLayer>,
    dirs: &hume_platform::dirs::Dirs,
    name: &str,
) -> bool {
    let result = load_theme(name, &super::theme_search_paths(dirs));
    install_load_result(
        result,
        engine_view,
        shown_theme,
        message_log,
        status_msg,
        popup,
        name,
    )
}

/// [`load_theme_by_name`] for the default theme config never chose. A theme
/// file that is not there is the expected way to end up on the fallback, so it
/// logs a Trace line and leaves the view alone; any other failure is reported
/// like a named theme's.
pub(in crate::editor) fn load_default_theme(
    engine_view: &mut EngineView,
    shown_theme: &mut String,
    message_log: &mut MessageLog,
    status_msg: &mut Option<String>,
    popup: Option<&mut PopupLayer>,
    dirs: &hume_platform::dirs::Dirs,
    name: &str,
) {
    match load_theme(name, &super::theme_search_paths(dirs)) {
        Err(ThemeError::NotFound { name: missing }) if missing == name => {
            message_log.push(
                Severity::Trace,
                format!("theme '{name}' not found, using the built-in fallback"),
            );
        }
        result => {
            install_load_result(
                result,
                engine_view,
                shown_theme,
                message_log,
                status_msg,
                popup,
                name,
            );
        }
    }
}

fn install_load_result(
    result: Result<LoadedTheme, ThemeError>,
    engine_view: &mut EngineView,
    shown_theme: &mut String,
    message_log: &mut MessageLog,
    status_msg: &mut Option<String>,
    popup: Option<&mut PopupLayer>,
    name: &str,
) -> bool {
    match result {
        Ok(loaded) => {
            set_theme(engine_view, shown_theme, popup, loaded.theme, name);
            if !loaded.warnings.is_empty() {
                let count = loaded.warnings.len();
                for warning in &loaded.warnings {
                    message_log.push(Severity::Warning, warning.to_string());
                }
                *status_msg = Some(format!(
                    "theme '{name}' loaded with {count} warning{}",
                    if count == 1 { "" } else { "s" }
                ));
            }
            true
        }
        Err(e) => {
            let text = e.to_string();
            *status_msg = Some(text.clone());
            message_log.push(Severity::Warning, text);
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
