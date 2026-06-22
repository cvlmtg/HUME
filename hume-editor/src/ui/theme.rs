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
        }
    }

    pub(crate) fn from_theme(theme: &hume_engine::theme::Theme) -> Self {
        use hume_engine::types::Scope;

        let style_for = |s: &'static str| -> Style { theme.resolve_by_name(Scope(s)).into() };

        Self {
            statusline: style_for("ui.statusline"),
            status_normal: style_for("ui.statusline.mode.normal"),
            status_insert: style_for("ui.statusline.mode.insert"),
            status_extend: style_for("ui.statusline.mode.extend"),
            status_search: style_for("ui.statusline.mode.search"),
            status_command: style_for("ui.statusline.mode.command"),
            status_select: style_for("ui.statusline.mode.select"),
        }
    }
}

// ── Engine theme builder ──────────────────────────────────────────────────────

// Default theme content — single source of truth is the TOML file.
// Scope names and palette values live in `runtime/themes/dark.toml`.
const DEFAULT_THEME_TOML: &str = include_str!("../../../runtime/themes/dark.toml");

/// Parse and return the default engine [`Theme`] from the embedded TOML.
///
/// The content is `runtime/themes/dark.toml`, embedded at compile time via
/// `include_str!`. Any edit to that file takes effect on the next build.
pub(crate) fn build_default_theme() -> hume_engine::theme::Theme {
    hume_engine::theme::loader::parse_theme(DEFAULT_THEME_TOML)
        .expect("embedded dark.toml must parse — file is compile-time embedded")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use hume_engine::theme::ScopeRegistry;
    use hume_engine::types::{ResolvedStyle, Scope};
    use ratatui::style::{Color, Style};

    use super::*;

    fn make_theme_with_statusline(
        base_fg: Color,
        base_bg: Color,
        insert_fg: Color,
    ) -> hume_engine::theme::Theme {
        let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
        styles.insert(
            "ui.statusline",
            ResolvedStyle {
                fg: Some(base_fg),
                bg: Some(base_bg),
                ..Default::default()
            },
        );
        styles.insert(
            "ui.statusline.mode.insert",
            ResolvedStyle {
                fg: Some(insert_fg),
                bg: Some(base_bg),
                ..Default::default()
            },
        );
        hume_engine::theme::Theme::new(styles, ResolvedStyle::default())
    }

    #[test]
    fn embedded_default_resolves_known_scopes() {
        let mut theme = build_default_theme();
        let registry = ScopeRegistry::new();
        theme.bake(&registry);

        // Independent oracle: expected values derived directly from dark.toml palette.
        // black = #000000, white = #ffffff, mid_gray = #8c8ca0, selection = #444478
        let cursor_primary = theme.resolve_by_name(Scope("ui.cursor.primary"));
        assert_eq!(
            cursor_primary.fg,
            Some(Color::Rgb(0x00, 0x00, 0x00)),
            "cursor.primary fg"
        );
        assert_eq!(
            cursor_primary.bg,
            Some(Color::Rgb(0xff, 0xff, 0xff)),
            "cursor.primary bg"
        );

        let cursor = theme.resolve_by_name(Scope("ui.cursor"));
        assert_eq!(cursor.fg, Some(Color::Rgb(0x00, 0x00, 0x00)), "cursor fg");
        assert_eq!(cursor.bg, Some(Color::Rgb(0x8c, 0x8c, 0xa0)), "cursor bg");

        let selection = theme.resolve_by_name(Scope("ui.selection"));
        assert_eq!(
            selection.bg,
            Some(Color::Rgb(0x44, 0x44, 0x78)),
            "selection bg"
        );

        // menu: fg = #b4b4c8, bg = #282832
        let menu = theme.resolve_by_name(Scope("ui.menu"));
        assert_eq!(menu.fg, Some(Color::Rgb(0xb4, 0xb4, 0xc8)), "menu fg");
        assert_eq!(menu.bg, Some(Color::Rgb(0x28, 0x28, 0x32)), "menu bg");

        // statusline: fg = black (#000000), bg = white (#ffffff)
        let statusline = theme.resolve_by_name(Scope("ui.statusline"));
        assert_eq!(
            statusline.fg,
            Some(Color::Rgb(0x00, 0x00, 0x00)),
            "statusline fg"
        );
        assert_eq!(
            statusline.bg,
            Some(Color::Rgb(0xff, 0xff, 0xff)),
            "statusline bg"
        );
    }

    #[test]
    fn from_theme_reads_statusline_scope() {
        let theme = make_theme_with_statusline(Color::Red, Color::Green, Color::Cyan);
        let colors = EditorColors::from_theme(&theme);

        // Independent oracle: expected values come from the input scopes, not from from_theme.
        let want_base = Style::default().fg(Color::Red).bg(Color::Green);
        let want_insert = Style::default().fg(Color::Cyan).bg(Color::Green);

        assert_eq!(colors.statusline, want_base);
        assert_eq!(colors.status_insert, want_insert);
    }

    #[test]
    fn from_theme_fallback_to_statusline_when_mode_missing() {
        // Only "ui.statusline" is defined; all mode-specific keys are absent.
        // The dot-fallback chain must resolve each ui.statusline.mode.* to ui.statusline.
        let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
        styles.insert(
            "ui.statusline",
            ResolvedStyle {
                fg: Some(Color::White),
                bg: Some(Color::DarkGray),
                ..Default::default()
            },
        );
        let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
        let colors = EditorColors::from_theme(&theme);

        let want = Style::default().fg(Color::White).bg(Color::DarkGray);
        assert_eq!(colors.statusline, want);
        assert_eq!(colors.status_normal, want);
        assert_eq!(colors.status_insert, want);
        assert_eq!(colors.status_extend, want);
        assert_eq!(colors.status_search, want);
        assert_eq!(colors.status_command, want);
        assert_eq!(colors.status_select, want);
    }
}
