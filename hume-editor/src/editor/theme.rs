//! Theme loading applied to a live editor.

use hume_engine::pipeline::EngineView;
use hume_engine::theme::loader::load_theme;

use crate::editor::message_log::{MessageLog, Severity};

/// Load a theme by name and apply it to the engine view.
///
/// Searches `<config_dir>/themes/<name>.toml` first, then
/// `<data_dir>/themes/<name>.toml`, then `<runtime_dir>/themes/<name>.toml`.
///
/// A theme with a malformed entry — a bad color, an unsupported scope shape
/// — still loads and still replaces the engine view's theme; only a problem
/// with the document as a whole (unreadable TOML, a missing `inherits`
/// parent) fails outright and leaves the current theme unchanged. See
/// [`hume_engine::theme::loader::LoadedTheme`]'s doc for the exact split.
/// `prepare_frame`'s `bake_if_stale` re-bakes a newly applied theme against
/// the live scope registry before the next render (a freshly loaded theme's
/// `baked` table starts empty, which is always stale).
///
/// Every warning is pushed to `message_log` individually; a non-empty
/// `warnings` also gets a one-line count written to `status_msg`, so the
/// theme applies but the user can see something didn't come through. A
/// document-level failure writes its own message to `status_msg` instead.
///
/// Returns `true` unless the load failed outright.
///
/// `engine_view`, `message_log`, and `status_msg` are disjoint `Editor` fields;
/// passing them separately lets the caller hold `&editor.settings.theme` for
/// the `name` argument without cloning.
pub(crate) fn load_theme_by_name(
    engine_view: &mut EngineView,
    message_log: &mut MessageLog,
    status_msg: &mut Option<String>,
    name: &str,
) -> bool {
    match load_theme(name, &super::theme_search_paths()) {
        Ok(loaded) => {
            engine_view.theme = loaded.theme;
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
