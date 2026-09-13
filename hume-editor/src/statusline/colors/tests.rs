use hume_grid::Rgb;
use std::collections::HashMap;

use hume_engine::types::{EditorMode, ResolvedStyle};

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
        EditorMode::Sift,
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
/// explicit separator scope (every bundled theme does).
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

#[test]
fn tabline_colors_reads_its_own_scopes() {
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.tabline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.tabline.active",
        ResolvedStyle {
            fg: Some(Rgb(0, 0, 0)),
            bg: Some(Rgb(0, 255, 255)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = TablineColors::from_theme(&theme);

    assert_eq!(
        colors.inactive,
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        }
    );
    assert_eq!(
        colors.active,
        ResolvedStyle {
            fg: Some(Rgb(0, 0, 0)),
            bg: Some(Rgb(0, 255, 255)),
            ..Default::default()
        }
    );
}

#[test]
fn tabline_active_falls_back_to_the_base_tabline_scope_when_undefined() {
    // No "ui.tabline.active" entry — an active tab with no themed override
    // must look like every other tab, via the engine's own dot-fallback.
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.tabline",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 255)),
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = TablineColors::from_theme(&theme);

    assert_eq!(colors.active, colors.inactive);
}

#[test]
fn tabline_colors_falls_back_to_helix_s_bufferline_scopes_when_untabbed() {
    // A Helix theme (or one ported from Helix) defines `ui.bufferline*` and
    // never `ui.tabline` — without a fallback, both slots would collapse to
    // the bare default scope and the bar would have no ground at all.
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.bufferline",
        ResolvedStyle {
            fg: Some(Rgb(200, 200, 200)),
            bg: Some(Rgb(30, 30, 30)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.bufferline.active",
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 0)),
            bg: Some(Rgb(60, 60, 60)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = TablineColors::from_theme(&theme);

    assert_eq!(
        colors.inactive,
        ResolvedStyle {
            fg: Some(Rgb(200, 200, 200)),
            bg: Some(Rgb(30, 30, 30)),
            ..Default::default()
        },
        "inactive must read ui.bufferline when ui.tabline is unthemed"
    );
    assert_eq!(
        colors.active,
        ResolvedStyle {
            fg: Some(Rgb(255, 255, 0)),
            bg: Some(Rgb(60, 60, 60)),
            ..Default::default()
        },
        "active must read ui.bufferline.active when ui.tabline.active is unthemed"
    );
}

#[test]
fn tabline_colors_prefers_its_own_scopes_over_bufferline_when_both_are_themed() {
    let mut styles: HashMap<&'static str, ResolvedStyle> = HashMap::new();
    styles.insert(
        "ui.tabline",
        ResolvedStyle {
            fg: Some(Rgb(1, 1, 1)),
            bg: Some(Rgb(2, 2, 2)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.bufferline",
        ResolvedStyle {
            fg: Some(Rgb(9, 9, 9)),
            bg: Some(Rgb(9, 9, 9)),
            ..Default::default()
        },
    );
    let theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());
    let colors = TablineColors::from_theme(&theme);

    assert_eq!(
        colors.inactive,
        ResolvedStyle {
            fg: Some(Rgb(1, 1, 1)),
            bg: Some(Rgb(2, 2, 2)),
            ..Default::default()
        },
        "ui.tabline must win over ui.bufferline when both are themed"
    );
}

#[test]
fn tabline_colors_falls_back_to_default_when_neither_tabline_nor_bufferline_is_themed() {
    let theme = hume_engine::theme::Theme::new(HashMap::new(), ResolvedStyle::default());
    let colors = TablineColors::from_theme(&theme);

    assert_eq!(colors.inactive, ResolvedStyle::default());
    assert_eq!(colors.active, ResolvedStyle::default());
}
