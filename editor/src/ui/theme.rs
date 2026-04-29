use std::collections::HashMap;

use ratatui::style::Style;

use engine::types::{Modifiers, ResolvedStyle};

/// Semantic color slots for the editor UI.
///
/// This is a flat struct with hardcoded defaults — not a theme system.
/// Field names follow Helix scope conventions (`ui.cursor` → `cursor_head`,
/// `ui.selection` → `selection`, `ui.cursorline` → `cursor_line`) so they
/// map cleanly when hierarchical theme support is added later.
///
/// # Dark-terminal assumption
///
/// All defaults target dark terminal backgrounds (the overwhelming majority of
/// terminal users). When config/theme loading is implemented, this struct
/// becomes the runtime-mutable output of the theme resolver; the hardcoded
/// values become the built-in "dark" fallback theme.
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

    pub(crate) fn from_theme(theme: &engine::theme::Theme) -> Self {
        use engine::types::Scope;

        let style_for = |s: &'static str| -> Style {
            theme.resolve_by_name(Scope(s)).into()
        };

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

/// Construct the default engine [`Theme`] from the same hardcoded color values
/// as `EditorColors::default()`, translated to Helix-style scope names.
///
/// Scope name conventions (Helix-compatible):
/// - `"ui.cursor"`              — block cursor (Normal/Extend)
/// - `"ui.cursor.insert"`       — bar cursor (Insert mode)
/// - `"ui.selection"`           — non-cursor selected chars
/// - `"ui.cursorline"`          — cursor-line background tint
/// - `"ui.virtual"`             — tilde rows and virtual text
/// - `"ui.linenr"`              — gutter line numbers
/// - `"ui.linenr.selected"`     — gutter on the cursor line
/// - `"ui.cursor.match"`        — bracket match highlight
/// - `"ui.selection.search"`    — search match highlight (Helix convention)
/// - `"ui.whitespace"`          — whitespace indicator characters
/// - `"ui.statusline"`          — base statusline style
/// - `"ui.statusline.mode.*"`   — per-mode label colors
pub(crate) fn build_default_theme() -> engine::theme::Theme {
    fn rgb(r: u8, g: u8, b: u8) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(r, g, b)
    }
    fn dark_gray() -> ratatui::style::Color {
        ratatui::style::Color::DarkGray
    }

    // "Reversed" in ratatui means swapping fg/bg — used for the statusline.
    // In engine ResolvedStyle there's no Modifiers::REVERSED; we simulate it
    // by setting explicit fg/bg that invert the terminal defaults. The terminal
    // default is typically white-on-black, so reversed ≈ black-on-white.
    // For colored mode labels (e.g. Cyan fg), we keep the reversed background
    // and just set the fg color.
    let statusline_bg = ratatui::style::Color::White;
    let statusline_fg = ratatui::style::Color::Black;

    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();

    let mut s = |scope: &'static str, style: ResolvedStyle| {
        styles.insert(scope, style);
    };

    // ── Cursor ──────────────────────────────────────────────────────────────
    // Primary cursor: bright white block — stands out as the main caret.
    s(
        "ui.cursor.primary",
        ResolvedStyle {
            fg: Some(rgb(0, 0, 0)),
            bg: Some(rgb(255, 255, 255)),
            ..Default::default()
        },
    );
    // Secondary cursors (multi-cursor): dimmer gray block — present but not dominant.
    s(
        "ui.cursor",
        ResolvedStyle {
            fg: Some(rgb(0, 0, 0)),
            bg: Some(rgb(140, 140, 160)),
            ..Default::default()
        },
    );
    // In bar-cursor modes the terminal cursor is the sole visual indicator —
    // no cell background override so the character underneath stays readable.
    s("ui.cursor.insert", ResolvedStyle::default());

    // ── Selection / cursor-line ──────────────────────────────────────────────
    s(
        "ui.selection",
        ResolvedStyle {
            bg: Some(rgb(68, 68, 120)),
            ..Default::default()
        },
    );
    s(
        "ui.cursorline",
        ResolvedStyle {
            bg: Some(rgb(58, 58, 58)),
            ..Default::default()
        },
    );

    // ── Virtual text / tilde rows ────────────────────────────────────────────
    s(
        "ui.virtual",
        ResolvedStyle {
            fg: Some(dark_gray()),
            ..Default::default()
        },
    );

    // ── Gutter ───────────────────────────────────────────────────────────────
    s(
        "ui.linenr",
        ResolvedStyle {
            fg: Some(dark_gray()),
            ..Default::default()
        },
    );
    // No bg: the cursorline row_bg fill shows through, unifying gutter and content.
    s(
        "ui.linenr.selected",
        ResolvedStyle {
            fg: Some(rgb(200, 200, 210)),
            ..Default::default()
        },
    );

    // ── Highlights ───────────────────────────────────────────────────────────
    s(
        "ui.cursor.match",
        ResolvedStyle {
            fg: Some(rgb(220, 180, 50)),
            bg: Some(rgb(60, 55, 20)),
            modifiers: Modifiers::BOLD,
            ..Default::default()
        },
    );
    s(
        "ui.selection.search",
        ResolvedStyle {
            fg: Some(rgb(255, 180, 80)),
            bg: Some(rgb(80, 40, 0)),
            ..Default::default()
        },
    );

    // ── Whitespace ───────────────────────────────────────────────────────────
    s(
        "ui.whitespace",
        ResolvedStyle {
            fg: Some(rgb(70, 70, 80)),
            ..Default::default()
        },
    );

    // ── Statusline ───────────────────────────────────────────────────────────
    s(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(statusline_fg),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.normal",
        ResolvedStyle {
            fg: Some(statusline_fg),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.insert",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Cyan),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.extend",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Yellow),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.search",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Magenta),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.command",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Green),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );
    s(
        "ui.statusline.mode.select",
        ResolvedStyle {
            fg: Some(ratatui::style::Color::Blue),
            bg: Some(statusline_bg),
            ..Default::default()
        },
    );

    engine::theme::Theme::new(styles, ResolvedStyle::default())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use engine::types::ResolvedStyle;
    use ratatui::style::{Color, Style};

    use super::*;

    fn make_theme_with_statusline(
        base_fg: Color,
        base_bg: Color,
        insert_fg: Color,
    ) -> engine::theme::Theme {
        let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
        styles.insert(
            "ui.statusline",
            ResolvedStyle { fg: Some(base_fg), bg: Some(base_bg), ..Default::default() },
        );
        styles.insert(
            "ui.statusline.mode.insert",
            ResolvedStyle { fg: Some(insert_fg), bg: Some(base_bg), ..Default::default() },
        );
        engine::theme::Theme::new(styles, ResolvedStyle::default())
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
            ResolvedStyle { fg: Some(Color::White), bg: Some(Color::DarkGray), ..Default::default() },
        );
        let theme = engine::theme::Theme::new(styles, ResolvedStyle::default());
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
