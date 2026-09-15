//! Theme loading applied to a live editor.

use hume_engine::pipeline::EngineView;
use hume_engine::theme::loader::load_theme;

use crate::editor::message_log::{MessageLog, Severity};
use crate::editor::overlay_models::PopupModel;

/// Replace `view.theme` and invalidate everything that caches against its
/// baked colors — currently just an open popup's per-width/style cache
/// (`PopupModel::content`), the one input to that cache besides `text`/
/// `syntax` (which never change during a popup's lifetime) that can change
/// out from under it. The single chokepoint for replacing a *live*
/// `view.theme`, so a future third replacement site can't forget the
/// invalidation the way a hand-placed `popup.content = None` next to each
/// write site could. `Editor::open`'s initial construction bypasses this on
/// purpose — there is no popup yet to invalidate.
pub(in crate::editor) fn set_theme(
    view: &mut EngineView,
    popup: Option<&mut PopupModel>,
    theme: hume_engine::theme::Theme,
) {
    view.theme = theme;
    if let Some(popup) = popup {
        popup.content = None;
    }
}

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
/// document-level failure writes its own message to `status_msg` instead and
/// leaves `popup` untouched.
///
/// Returns `true` unless the load failed outright.
///
/// `engine_view`, `message_log`, `status_msg`, and `popup` are disjoint
/// `Editor` fields; passing them separately lets the caller hold
/// `&editor.settings.theme` for the `name` argument without cloning.
pub(in crate::editor) fn load_theme_by_name(
    engine_view: &mut EngineView,
    message_log: &mut MessageLog,
    status_msg: &mut Option<String>,
    popup: Option<&mut PopupModel>,
    name: &str,
) -> bool {
    match load_theme(name, &super::theme_search_paths()) {
        Ok(loaded) => {
            set_theme(engine_view, popup, loaded.theme);
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

// Default theme content — single source of truth is the TOML file.
// Scope names and palette values live in `runtime/themes/sand.toml`
// (HUME's signature theme).
const DEFAULT_THEME_TOML: &str = include_str!("../../../runtime/themes/sand.toml");

/// Parse and return the default engine [`hume_engine::theme::Theme`] from the embedded TOML.
///
/// The content is `runtime/themes/sand.toml`, embedded at compile time via
/// `include_str!` — editing that file requires a rebuild to take effect.
pub(in crate::editor) fn build_default_theme() -> hume_engine::theme::Theme {
    let loaded = hume_engine::theme::loader::parse_theme(DEFAULT_THEME_TOML)
        .expect("embedded sand.toml must parse — file is compile-time embedded");
    // Unlike a user's own theme, sand.toml is HUME's shipped content — a
    // warning here is a bug in this repo, not a typo to shrug off, so it's
    // stated as an invariant at the one site that would otherwise drop it
    // silently (`load_theme_by_name` surfaces the same warnings for every
    // other load path). `load_bundled_themes` in `editor/tests/theme_loading.rs`
    // pins the same guarantee for the on-disk copy of this file.
    // A real `assert!`, not `debug_assert!`: this runs once at startup over
    // content fixed at compile time, so it costs nothing, and a `debug_assert`
    // would drop in release exactly the builds where a shipped-theme mistake
    // reaches users. `crate::testing::build_snapshot_theme` asserts the same way.
    assert!(
        loaded.warnings.is_empty(),
        "embedded sand.toml produced load warnings: {:?}",
        loaded.warnings
    );
    loaded.theme
}

#[cfg(test)]
mod tests {
    use super::build_default_theme;
    use hume_engine::theme::ScopeRegistry;
    use hume_engine::theme::ui_scopes;
    use hume_engine::types::Scope;

    /// The embedded default theme (`sand.toml`, inlined via `include_str!` at
    /// compile time) must match the *same* file loaded through the production
    /// runtime loader (`load_theme`, the path `:theme <name>` uses) — not
    /// hardcoded hex colors, which drift every time the palette is tuned and
    /// then need manual updates here. This only breaks if the embed points at
    /// the wrong file, the content fails to parse, or the two loaders disagree.
    #[test]
    fn embedded_default_matches_sand_toml_on_disk() {
        use std::path::PathBuf;

        let mut embedded = build_default_theme();
        let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
        let mut from_disk = hume_engine::theme::loader::load_theme("sand", &[themes_dir])
            .expect("runtime/themes/sand.toml must load via the production theme loader")
            .theme;

        let registry = ScopeRegistry::new();
        embedded.bake(&registry);
        from_disk.bake(&registry);

        // Scopes exercised by the renderer's hot paths: cursor, selection,
        // menu, statusline, pane background/seam.
        for scope in [
            hume_engine::theme::CURSOR_PRIMARY,
            hume_engine::theme::CURSOR,
            ui_scopes::SELECTION,
            ui_scopes::MENU,
            ui_scopes::TEXT_FOCUS,
            ui_scopes::STATUSLINE,
            ui_scopes::STATUSLINE_SEPARATOR,
            ui_scopes::STATUSLINE_NORMAL,
            ui_scopes::TABLINE,
            ui_scopes::TABLINE_ACTIVE,
            ui_scopes::BACKGROUND,
            ui_scopes::WINDOW,
            ui_scopes::WINDOW_FOCUSED,
        ] {
            assert_eq!(
                embedded.resolve_by_name(Scope(scope)),
                from_disk.resolve_by_name(Scope(scope)),
                "embedded sand.toml disagrees with the on-disk file for scope '{scope}'"
            );
        }

        // `ui.text` must fold into `theme.default` — the base style every
        // plain-text cell starts from (see `style::apply_styles`) — so
        // unhighlighted text carries an explicit color the focus-dimming
        // blend can act on instead of escaping it as `None` (the terminal's
        // own default, which the blend has no numeric value to act on).
        assert_eq!(
            embedded.default, from_disk.default,
            "embedded sand.toml's default style (ui.text fold) disagrees with the on-disk file"
        );
    }
}
