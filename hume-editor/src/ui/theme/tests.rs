use std::collections::HashMap;

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::{ResolvedStyle, Scope};
use ratatui::style::{Color, Modifier, Style};

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
        "ui.statusline.insert",
        ResolvedStyle {
            fg: Some(insert_fg),
            bg: Some(base_bg),
            ..Default::default()
        },
    );
    hume_engine::theme::Theme::new(styles, ResolvedStyle::default())
}

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
        .expect("runtime/themes/sand.toml must load via the production theme loader");

    let registry = ScopeRegistry::new();
    embedded.bake(&registry);
    from_disk.bake(&registry);

    // Scopes exercised by the renderer's hot paths: cursor, selection,
    // menu, statusline, pane background/seam.
    for scope in [
        "ui.cursor.primary",
        "ui.cursor",
        "ui.selection",
        "ui.menu",
        "ui.statusline",
        "ui.statusline.separator",
        "ui.statusline.normal",
        "ui.background",
        "ui.window",
        "ui.window.focused",
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
    // blend can act on instead of escaping it as `Color::Reset`.
    assert_eq!(
        embedded.default, from_disk.default,
        "embedded sand.toml's default style (ui.text fold) disagrees with the on-disk file"
    );
}

#[test]
fn from_theme_reads_statusline_scope() {
    let theme = make_theme_with_statusline(Color::Red, Color::Green, Color::Cyan);
    let colors = EditorColors::from_theme(&theme);

    // Independent oracle: expected values come from the input scopes, not from
    // from_theme. ResolvedStyle -> ratatui::Style is fully-specifying (every
    // modifier not explicitly enabled is explicitly turned off), so a plain
    // fg/bg style also clears every modifier bit.
    let want_base = Style::default()
        .fg(Color::Red)
        .bg(Color::Green)
        .remove_modifier(Modifier::all());
    let want_insert = Style::default()
        .fg(Color::Cyan)
        .bg(Color::Green)
        .remove_modifier(Modifier::all());

    assert_eq!(colors.statusline, want_base);
    assert_eq!(colors.status_insert, want_insert);
}

#[test]
fn from_theme_fallback_to_statusline_when_mode_missing() {
    // Only "ui.statusline" is defined; all mode-specific and separator keys are absent.
    // The dot-fallback chain must resolve each ui.statusline.* to ui.statusline.
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

    // Fully-specifying: a plain fg/bg style also clears every modifier bit
    // (see the `From<ResolvedStyle> for ratatui::style::Style` contract).
    let want = Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .remove_modifier(Modifier::all());
    assert_eq!(colors.statusline, want);
    assert_eq!(colors.status_normal, want);
    assert_eq!(colors.status_insert, want);
    assert_eq!(colors.status_extend, want);
    assert_eq!(colors.status_search, want);
    assert_eq!(colors.status_command, want);
    assert_eq!(colors.status_select, want);
    assert_eq!(colors.statusline_separator, want);
}

#[test]
fn separator_scope_honored_when_defined() {
    // "ui.statusline.separator" carries its own fg (no bg); other scopes
    // fall back to the base "ui.statusline" style.
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(Color::White),
            bg: Some(Color::DarkGray),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.separator",
        ResolvedStyle {
            fg: Some(Color::Yellow),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = EditorColors::from_theme(&theme);

    let want_base = Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .remove_modifier(Modifier::all());
    let want_separator = Style::default()
        .fg(Color::Yellow)
        .remove_modifier(Modifier::all());

    assert_eq!(colors.statusline_separator, want_separator);
    assert_eq!(colors.status_normal, want_base);
}
