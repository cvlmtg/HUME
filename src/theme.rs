use ratatui::style::{Color, Modifier, Style};

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
    // ── Content area ──────────────────────────────────────────────────────────

    /// Default text style — terminal default fg and bg, no decoration.
    pub default: Style,

    /// The cursor head cell. Must be visually distinct from `selection` so
    /// the user can see exactly where the cursor is within a selection.
    /// White-on-black gives a solid, unmistakable block on dark backgrounds.
    pub cursor_head: Style,

    /// Selected characters that are not the cursor head.
    /// A muted blue-purple bg lets the text remain readable while making the
    /// selection extent clear.
    pub selection: Style,

    /// Background tint for the entire cursor line (lowest priority — overridden
    /// by `selection` and `cursor_head`). Very subtle so it doesn't fight with
    /// the selection highlight.
    pub cursor_line: Style,

    // ── Gutter ────────────────────────────────────────────────────────────────

    /// Line number gutter on the cursor line.
    /// Slightly brighter than `gutter` so the current line stands out;
    /// shares the `cursor_line` background tint for visual consistency.
    pub gutter_cursor_line: Style,

    /// Line number gutter on non-cursor lines. Dimmed so it recedes behind
    /// the document content.
    pub gutter: Style,

    // ── EOF tilde rows ────────────────────────────────────────────────────────

    /// The `~` drawn on rows past the end of the buffer.
    pub tilde: Style,

    // ── Status bar ────────────────────────────────────────────────────────────

    /// Base style for the entire status bar row (inverted video fill).
    pub status_bar: Style,

    /// Mode label in Normal mode (`NOR`).
    pub status_normal: Style,

    /// Mode label in Insert mode (`INS`). Cyan makes mode transitions obvious.
    pub status_insert: Style,

    /// Mode label in Extend mode (`EXT`). Yellow distinguishes it from Normal.
    pub status_extend: Style,

    /// Mode label in Command mode (`CMD`). Green signals "input expected".
    /// Shown only as a fallback — command mode normally renders a command line
    /// instead of a status bar.
    pub status_command: Style,
}

impl EditorColors {
    pub(crate) fn default() -> Self {
        let reversed = Style::new().add_modifier(Modifier::REVERSED);
        Self {
            default: Style::new(),
            cursor_head: Style::new()
                .bg(Color::Rgb(255, 255, 255))
                .fg(Color::Rgb(0, 0, 0)),
            selection: Style::new().bg(Color::Rgb(68, 68, 120)),
            cursor_line: Style::new().bg(Color::Rgb(35, 35, 45)),
            gutter_cursor_line: Style::new()
                .fg(Color::Rgb(180, 180, 180))
                .bg(Color::Rgb(35, 35, 45)),
            gutter: Style::new().fg(Color::DarkGray),
            tilde: Style::new().fg(Color::DarkGray),
            status_bar: reversed,
            status_normal: reversed,
            status_insert: reversed.fg(Color::Cyan),
            status_extend: reversed.fg(Color::Yellow),
            status_command: reversed.fg(Color::Green),
        }
    }
}
