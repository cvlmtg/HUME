use hume_grid::Rgb;
use std::collections::HashMap;

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::{EditorMode, ResolvedStyle, Scope};

use super::*;

fn make_theme_with_statusline(
    base_fg: Rgb,
    base_bg: Rgb,
    insert_fg: Rgb,
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
        "ui.text.focus",
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
    // blend can act on instead of escaping it as `None` (the terminal's
    // own default, which the blend has no numeric value to act on).
    assert_eq!(
        embedded.default, from_disk.default,
        "embedded sand.toml's default style (ui.text fold) disagrees with the on-disk file"
    );
}

#[test]
fn from_theme_reads_statusline_scope() {
    let theme = make_theme_with_statusline(Rgb(255, 0, 0), Rgb(0, 255, 0), Rgb(0, 255, 255));

    // Independent oracle: expected values come from the input scopes, not
    // from `from_theme`. A scope that sets only fg/bg carries no modifiers.
    let want_base = ResolvedStyle {
        fg: Some(Rgb(255, 0, 0)),
        bg: Some(Rgb(0, 255, 0)),
        ..Default::default()
    };
    let want_insert = ResolvedStyle {
        fg: Some(Rgb(0, 255, 255)),
        bg: Some(Rgb(0, 255, 0)),
        ..Default::default()
    };

    // This fixture defines no "ui.statusline.normal" entry, so Normal falls
    // back to the base "ui.statusline" style via the dot-fallback chain.
    assert_eq!(
        EditorColors::from_theme(&theme, Some(EditorMode::Normal)).statusline,
        want_base
    );
    // Insert has its own entry — the whole row picks it up.
    assert_eq!(
        EditorColors::from_theme(&theme, Some(EditorMode::Insert)).statusline,
        want_insert
    );
}

#[test]
fn from_theme_fallback_to_statusline_when_mode_missing() {
    // Only "ui.statusline" is defined; all mode-specific and separator keys are absent.
    // The dot-fallback chain must resolve each ui.statusline.* to ui.statusline.
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());

    // Fully-specifying: a plain fg/bg style also clears every modifier bit
    // (see the `From<ResolvedStyle> for ResolvedStyle` contract).
    let want = ResolvedStyle {
        fg: Some(Rgb(255, 255, 255)),
        bg: Some(Rgb(64, 64, 64)),
        ..Default::default()
    };
    for mode in [
        EditorMode::Normal,
        EditorMode::Insert,
        EditorMode::Extend,
        EditorMode::Search,
        EditorMode::Command,
        EditorMode::Select,
    ] {
        let colors = EditorColors::from_theme(&theme, Some(mode));
        assert_eq!(
            colors.statusline, want,
            "mode {mode:?} should fall back to ui.statusline"
        );
        assert_eq!(colors.statusline_separator, want);
    }
}

#[test]
fn separator_scope_honored_when_defined() {
    // "ui.statusline.separator" carries its own fg (no bg); other scopes
    // fall back to the base "ui.statusline" style.
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.separator",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 0)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = EditorColors::from_theme(&theme, Some(EditorMode::Normal));

    let want_base = ResolvedStyle {
        fg: Some(Rgb(255, 255, 255)),
        bg: Some(Rgb(64, 64, 64)),
        ..Default::default()
    };
    let want_separator = ResolvedStyle {
        fg: Some(Rgb(255, 255, 0)),
        ..Default::default()
    };

    assert_eq!(colors.statusline_separator, want_separator);
    assert_eq!(colors.statusline, want_base);
}

/// A theme that tints a mode's row but never defines
/// `ui.statusline.separator` must not punch the *base* `ui.statusline`
/// color through the tinted row: the separator has to inherit whatever the
/// row itself resolved to, not dot-fallback to its own untinted parent
/// scope. Covers every imported/Helix/user theme that doesn't define an
/// explicit separator scope (all four bundled themes do).
#[test]
fn separator_falls_back_to_the_active_row_style_not_the_base_scope_when_undefined() {
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.insert",
        ResolvedStyle {
            fg: Some(Rgb(0, 0, 0)),
            bg: Some(Rgb(0, 255, 255)),
            ..Default::default()
        },
    );
    // No "ui.statusline.separator" entry.
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = EditorColors::from_theme(&theme, Some(EditorMode::Insert));

    let want_insert = ResolvedStyle {
        fg: Some(Rgb(0, 0, 0)),
        bg: Some(Rgb(0, 255, 255)),
        ..Default::default()
    };

    assert_eq!(
        colors.statusline, want_insert,
        "sanity: the row itself must pick up the Insert-mode tint"
    );
    assert_eq!(
        colors.statusline_separator, want_insert,
        "the separator must inherit the tinted row style, not the base ui.statusline bg — \
         otherwise it paints an opaque hole of the wrong color in the middle of the row"
    );
}

/// `statusline.mode-colors=false` passes `None` for the mode — it must read
/// the base `ui.statusline` scope, not silently substitute `EditorMode::Normal`
/// (whose own scope can be a distinct accent in an imported theme, e.g.
/// Helix's old pill idiom). `ui.statusline.normal` is given a different bg
/// from `ui.statusline` here specifically so a regression back to
/// `Some(EditorMode::Normal)` fails this test instead of passing by
/// coincidence, as it would against every bundled theme.
#[test]
fn from_theme_without_a_mode_reads_the_base_scope() {
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.normal",
        ResolvedStyle {
            fg: Some(Rgb(0, 0, 0)),
            bg: Some(Rgb(0, 0, 255)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());

    let want_base = ResolvedStyle {
        fg: Some(Rgb(255, 255, 255)),
        bg: Some(Rgb(64, 64, 64)),
        ..Default::default()
    };
    let want_normal = ResolvedStyle {
        fg: Some(Rgb(0, 0, 0)),
        bg: Some(Rgb(0, 0, 255)),
        ..Default::default()
    };

    assert_eq!(EditorColors::from_theme(&theme, None).statusline, want_base);
    assert_eq!(
        EditorColors::from_theme(&theme, Some(EditorMode::Normal)).statusline,
        want_normal
    );
}
