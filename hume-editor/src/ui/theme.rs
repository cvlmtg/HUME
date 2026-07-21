use ratatui::style::Style;

/// Resolved statusline color slots, read from the active engine [`Theme`].
///
/// Covers only the statusline row; all other UI surfaces (cursor, selection,
/// gutter, completion popup) are styled directly by the engine via scope
/// resolution at render time.
pub(crate) struct EditorColors {
    // ── Statusline ────────────────────────────────────────────────────────────
    // Content-area colors (cursor, selection, highlights, gutter) are now
    // handled by the engine's Theme system via `build_default_theme()` below.
    /// Base style for the entire statusline row (inverted video fill).
    pub statusline: Style,

    /// Mode label in Normal mode (`NOR`).
    pub status_normal: Style,

    /// Mode label in Insert mode (`INS`). Cyan makes mode transitions obvious.
    pub status_insert: Style,

    /// Mode label in Extend mode (`EXT`). Yellow distinguishes it from Normal.
    pub status_extend: Style,

    /// Mode label in Search mode (`SRC`). Magenta makes the prompt visually distinct.
    pub status_search: Style,

    /// Mode label in Command mode (`CMD`). Green distinguishes it from Search.
    pub status_command: Style,

    /// Mode label in Select mode (`SEL`). Blue distinguishes it from Search.
    pub status_select: Style,

    /// Separator glyph (`│`) between statusline elements. Falls back to
    /// `statusline` when a theme omits `ui.statusline.separator`.
    pub statusline_separator: Style,
}

impl EditorColors {
    #[cfg(test)]
    pub(crate) fn default() -> Self {
        use ratatui::style::{Color, Modifier};
        let reversed = Style::new().add_modifier(Modifier::REVERSED);
        Self {
            statusline: reversed,
            status_normal: reversed,
            status_insert: reversed.fg(Color::Cyan),
            status_extend: reversed.fg(Color::Yellow),
            status_search: reversed.fg(Color::Magenta),
            status_command: reversed.fg(Color::Green),
            status_select: reversed.fg(Color::Blue),
            statusline_separator: reversed,
        }
    }

    pub(crate) fn from_theme(theme: &hume_engine::theme::Theme) -> Self {
        use hume_engine::types::Scope;

        let style_for = |s: &'static str| -> Style { theme.resolve_by_name(Scope(s)).into() };

        Self {
            statusline: style_for("ui.statusline"),
            status_normal: style_for("ui.statusline.normal"),
            status_insert: style_for("ui.statusline.insert"),
            status_extend: style_for("ui.statusline.extend"),
            status_search: style_for("ui.statusline.search"),
            status_command: style_for("ui.statusline.command"),
            status_select: style_for("ui.statusline.select"),
            statusline_separator: style_for("ui.statusline.separator"),
        }
    }
}

// ── Engine theme builder ──────────────────────────────────────────────────────

// Default theme content — single source of truth is the TOML file.
// Scope names and palette values live in `runtime/themes/sand.toml`
// (HUME's signature theme).
const DEFAULT_THEME_TOML: &str = include_str!("../../../runtime/themes/sand.toml");

/// Parse and return the default engine [`Theme`] from the embedded TOML.
///
/// The content is `runtime/themes/sand.toml`, embedded at compile time via
/// `include_str!` — editing that file requires a rebuild to take effect.
pub(crate) fn build_default_theme() -> hume_engine::theme::Theme {
    hume_engine::theme::loader::parse_theme(DEFAULT_THEME_TOML)
        .expect("embedded sand.toml must parse — file is compile-time embedded")
}

/// `dark.toml`, embedded for renderer snapshot tests that assert exact
/// colors. Those tests exercise seam/junction/dimming *rendering mechanics*,
/// not the default theme's palette — pinning them to a stable theme means
/// retuning `sand.toml` (the compiled-in default) never forces an unrelated
/// snapshot re-record.
#[cfg(test)]
const DARK_THEME_TOML_FOR_SNAPSHOT_TESTS: &str = include_str!("../../../runtime/themes/dark.toml");

#[cfg(test)]
pub(crate) fn build_dark_theme_for_snapshot_tests() -> hume_engine::theme::Theme {
    hume_engine::theme::loader::parse_theme(DARK_THEME_TOML_FOR_SNAPSHOT_TESTS)
        .expect("embedded dark.toml must parse — file is compile-time embedded")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
