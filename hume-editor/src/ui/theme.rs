use ratatui::style::Style;

/// Resolved statusline color slots, read from the active engine [`hume_engine::theme::Theme`].
///
/// Covers only the statusline row; all other UI surfaces (cursor, selection,
/// gutter, completion popup) are styled directly by the engine via scope
/// resolution at render time.
pub(crate) struct EditorColors {
    // ── Statusline ────────────────────────────────────────────────────────────
    /// Style for the entire statusline row, resolved from the current mode's
    /// scope (`ui.statusline.<mode>`) — see [`mode_scope`]. Every element
    /// except the separator paints with this style, so the whole row tints
    /// with the mode. `None` for the mode (`statusline.mode-colors` off) holds
    /// the theme's base `ui.statusline` style instead.
    ///
    /// Replaces Helix's mode-pill idiom (a colored 3-character corner): the
    /// whole row makes the active mode legible at a glance instead of
    /// requiring a glance at one small corner.
    pub statusline: Style,

    /// Separator glyph (`│`) between statusline elements. When a theme
    /// omits `ui.statusline.separator`, this is the *active row's* style
    /// (`statusline`, above) — not the dot-notation fallback to the untinted
    /// base `ui.statusline` scope, which would paint an opaque hole of the
    /// wrong background in the middle of a mode-tinted row.
    pub statusline_separator: Style,
}

/// The theme scope carrying the row style for `mode` — `None` means
/// `statusline.mode-colors` is off, which must resolve the base
/// `ui.statusline` scope rather than any particular mode's: a theme whose
/// `ui.statusline.normal` is a distinct accent (Helix's old pill idiom) would
/// otherwise leave the opt-out just as loud as the tint it's meant to escape.
fn mode_scope(mode: Option<hume_engine::types::EditorMode>) -> &'static str {
    use hume_engine::types::EditorMode;

    match mode {
        None => "ui.statusline",
        Some(EditorMode::Normal) => "ui.statusline.normal",
        Some(EditorMode::Insert) => "ui.statusline.insert",
        Some(EditorMode::Extend) => "ui.statusline.extend",
        Some(EditorMode::Search) => "ui.statusline.search",
        Some(EditorMode::Command) => "ui.statusline.command",
        Some(EditorMode::Select) => "ui.statusline.select",
    }
}

impl EditorColors {
    #[cfg(test)]
    pub(crate) fn default() -> Self {
        use ratatui::style::Modifier;
        let reversed = Style::new().add_modifier(Modifier::REVERSED);
        Self {
            statusline: reversed,
            statusline_separator: reversed,
        }
    }

    pub(crate) fn from_theme(
        theme: &hume_engine::theme::Theme,
        mode: Option<hume_engine::types::EditorMode>,
    ) -> Self {
        use hume_engine::types::Scope;

        let style_for = |s: &'static str| -> Style { theme.resolve_by_name(Scope(s)).into() };

        let statusline = style_for(mode_scope(mode));
        // `resolve_by_name`'s dot-notation fallback would otherwise land an
        // absent "ui.statusline.separator" on the untinted base
        // "ui.statusline" scope — the wrong target now that the row itself
        // is mode-tinted. Check for an explicit entry first and fall back to
        // the row's own (already-resolved) style instead.
        let statusline_separator = if theme.raw_contains("ui.statusline.separator") {
            style_for("ui.statusline.separator")
        } else {
            statusline
        };

        Self {
            statusline,
            statusline_separator,
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
