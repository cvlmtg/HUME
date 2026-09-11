use hume_engine::types::ResolvedStyle;

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
    pub statusline: ResolvedStyle,

    /// Separator glyph (`│`) between statusline elements. When a theme
    /// omits `ui.statusline.separator`, this is the *active row's* style
    /// (`statusline`, above) — not the dot-notation fallback to the untinted
    /// base `ui.statusline` scope, which would paint an opaque hole of the
    /// wrong background in the middle of a mode-tinted row.
    pub statusline_separator: ResolvedStyle,
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
        // Extend is HUME's name for the mode Helix calls Select, so it reads
        // Helix's real `ui.statusline.select` scope.
        Some(EditorMode::Extend) => "ui.statusline.select",
        Some(EditorMode::Search) => "ui.statusline.search",
        Some(EditorMode::Command) => "ui.statusline.command",
        // HUME's own Sift mode (the `s` regex prompt) has no Helix
        // equivalent, so it gets its own scope rather than squatting on
        // Helix's `ui.statusline.select`, which belongs to Extend above.
        Some(EditorMode::Sift) => "ui.statusline.sift",
    }
}

impl EditorColors {
    #[cfg(test)]
    pub(crate) fn default() -> Self {
        let reversed = ResolvedStyle {
            modifiers: hume_engine::types::Modifiers::REVERSED,
            ..Default::default()
        };
        Self {
            statusline: reversed,
            statusline_separator: reversed,
        }
    }

    pub(in crate::statusline) fn from_theme(
        theme: &hume_engine::theme::Theme,
        mode: Option<hume_engine::types::EditorMode>,
    ) -> Self {
        use hume_engine::types::Scope;

        let style_for = |s: &'static str| -> ResolvedStyle { theme.resolve_by_name(Scope(s)) };

        let statusline = style_for(mode_scope(mode));
        // `resolve_by_name`'s dot-notation fallback would otherwise land an
        // absent "ui.statusline.separator" on the untinted base
        // "ui.statusline" scope — the wrong target, since the row itself is
        // mode-tinted. Check for an explicit entry first and fall back to
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

#[cfg(test)]
mod tests;
