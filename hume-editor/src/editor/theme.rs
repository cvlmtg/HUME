//! Theme loading applied to a live editor.

use hume_engine::pipeline::EngineView;
use hume_engine::theme::loader::load_theme;

use crate::editor::message_log::{MessageLog, Severity};

/// Load a theme by name and apply it to the engine view.
///
/// Searches `<config_dir>/themes/<name>.toml` first, then
/// `<data_dir>/themes/<name>.toml`, then `<runtime_dir>/themes/<name>.toml`.
/// On success the engine view's theme is
/// replaced; `prepare_frame`'s `bake_if_stale` re-bakes it against the live
/// scope registry before the next render (a freshly loaded theme's `baked`
/// table starts empty, which is always stale). On failure a warning is pushed
/// to `message_log` and written to `status_msg`, leaving the current theme
/// unchanged. Returns `true` on success.
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
        Ok(theme) => {
            engine_view.theme = theme;
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
