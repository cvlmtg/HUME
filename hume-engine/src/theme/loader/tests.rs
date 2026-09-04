use hume_grid::Rgb;
use std::path::Path;

use super::*;
use crate::types::Modifiers;
use tempfile::TempDir;

// ── Test fixture helpers ──────────────────────────────────────────────────

fn write_theme(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(format!("{name}.toml")), content).expect("failed to write test theme");
}

fn paths(dir: &Path) -> Vec<PathBuf> {
    vec![dir.to_path_buf()]
}

// ── Flat scope / happy path ───────────────────────────────────────────────

#[test]
fn flat_scope_happy_path() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "test",
        r##"
"keyword" = "#ff0000"
"keyword.function" = { fg = "#00ff00", modifiers = ["bold"] }
"ui.cursor" = { fg = "#ffffff", bg = "#000000" }
"##,
    );

    let theme = load_theme("test", &paths(dir.path())).unwrap();

    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));
    assert_eq!(kw.bg, None);

    let kw_fn = theme.resolve_by_name(crate::types::Scope("keyword.function"));
    assert_eq!(kw_fn.fg, Some(Rgb(0, 0xff, 0)));
    assert!(kw_fn.modifiers.contains(Modifiers::BOLD));

    // Fallback: "keyword.operator" → "keyword"
    let kw_op = theme.resolve_by_name(crate::types::Scope("keyword.operator"));
    assert_eq!(kw_op.fg, Some(Rgb(0xff, 0, 0)));
}

// ── Palette indirection ───────────────────────────────────────────────────

#[test]
fn palette_indirection() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "pal",
        r##"
"keyword"  = "red"
"comment"  = { fg = "green", modifiers = ["italic"] }
"constant" = "#abcdef"

[palette]
red    = "#cc241d"
green  = "#98971a"
"##,
    );

    let theme = load_theme("pal", &paths(dir.path())).unwrap();

    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xcc, 0x24, 0x1d)));

    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0x98, 0x97, 0x1a)));
    assert!(cm.modifiers.contains(Modifiers::ITALIC));

    // Literal hex still works (not in palette).
    let cn = theme.resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(cn.fg, Some(Rgb(0xab, 0xcd, 0xef)));
}

// ── Inheritance ───────────────────────────────────────────────────────────

#[test]
fn inherits_child_overrides_parent() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base",
        r##"
"keyword"   = "#ff0000"
"comment"   = "#888888"
"constant"  = "#abcdef"
"##,
    );
    write_theme(
        dir.path(),
        "child",
        r##"
inherits = "base"
"keyword" = "#00ff00"
"##,
    );

    let theme = load_theme("child", &paths(dir.path())).unwrap();

    // Child overrides "keyword".
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0, 0xff, 0)));

    // Parent's "comment" and "constant" still present.
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0x88, 0x88, 0x88)));
    let cn = theme.resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(cn.fg, Some(Rgb(0xab, 0xcd, 0xef)));
}

#[test]
fn inherits_child_has_independent_palette() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base2",
        r##"
"keyword" = "red"

[palette]
red = "#ff0000"
"##,
    );
    write_theme(
        dir.path(),
        "child2",
        r##"
inherits = "base2"

"comment" = "blue"

[palette]
blue = "#0000ff"
"##,
    );

    let theme = load_theme("child2", &paths(dir.path())).unwrap();

    // Parent keyword resolved to red via parent palette.
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));

    // Child comment resolved to blue via child palette.
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0, 0, 0xff)));
}

// ── `ui.text` → `default` fold ────────────────────────────────────────────

#[test]
fn ui_text_folds_into_default() {
    let theme = parse_theme(r##""ui.text" = { fg = "#d0d0d0" }"##).unwrap();
    assert_eq!(theme.default.fg, Some(Rgb(0xd0, 0xd0, 0xd0)));
}

#[test]
fn inherits_child_overrides_parent_ui_text_default() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "base3", r##""ui.text" = { fg = "#111111" }"##);
    write_theme(
        dir.path(),
        "child3",
        r##"
inherits = "base3"
"ui.text" = { fg = "#222222" }
"##,
    );

    let theme = load_theme("child3", &paths(dir.path())).unwrap();

    // Child's own `ui.text` re-folds on top of the parent's — last-wins,
    // not a stale copy of the parent's default.
    assert_eq!(theme.default.fg, Some(Rgb(0x22, 0x22, 0x22)));
}

#[test]
fn inherits_child_without_ui_text_keeps_parent_default() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "base4", r##""ui.text" = { fg = "#111111" }"##);
    write_theme(dir.path(), "child4", r#"inherits = "base4""#);

    let theme = load_theme("child4", &paths(dir.path())).unwrap();

    // Child defines no `ui.text` of its own — the parent's already-folded
    // `default` passes through unchanged (re-folding is a no-op here since
    // the child's `scopes` map has no "ui.text" entry to layer again).
    assert_eq!(theme.default.fg, Some(Rgb(0x11, 0x11, 0x11)));
}

// ── Cycle detection ───────────────────────────────────────────────────────

#[test]
fn cycle_is_detected() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "a", r#"inherits = "b""#);
    write_theme(dir.path(), "b", r#"inherits = "a""#);

    let err = load_theme("a", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(err, ThemeError::Cycle { .. }),
        "expected Cycle error, got: {err}"
    );
}

// ── Max depth ─────────────────────────────────────────────────────────────

#[test]
fn max_depth_is_detected() {
    let dir = TempDir::new().unwrap();
    // Chain: t0 → t1 → … → t9 (10 levels, exceeds MAX_DEPTH=8)
    for i in 0..=9usize {
        let content = if i < 9 {
            format!("inherits = \"t{}\"", i + 1)
        } else {
            r##""keyword" = "#ff0000""##.to_owned()
        };
        write_theme(dir.path(), &format!("t{i}"), &content);
    }

    let err = load_theme("t0", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(err, ThemeError::MaxDepth { .. }),
        "expected MaxDepth error, got: {err}"
    );
}

// ── Not found ─────────────────────────────────────────────────────────────

#[test]
fn not_found_returns_error() {
    let dir = TempDir::new().unwrap();
    let err = load_theme("nonexistent", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(matches!(err, ThemeError::NotFound { .. }));
}

// ── Bad palette reference ─────────────────────────────────────────────────

#[test]
fn bad_palette_ref_is_error() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad",
        r#"
"keyword" = "nonexistent_color"
"#,
    );
    let err = load_theme("bad", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(err, ThemeError::BadColor { .. }),
        "expected BadColor, got: {err}"
    );
}

#[test]
fn empty_string_color_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "empty_color",
        r#""constant" = { fg = "white", bg = "" }"#,
    );
    let err = load_theme("empty_color", &paths(dir.path()))
        .err()
        .expect("expected BadColor for empty-string bg");
    assert!(matches!(err, ThemeError::BadColor { .. }), "got: {err}");
}

// ── Bad modifier ─────────────────────────────────────────────────────────

#[test]
fn bad_modifier_is_error() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_mod",
        r##"
"keyword" = { fg = "#ff0000", modifiers = ["wiggly"] }
"##,
    );
    let err = load_theme("bad_mod", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(err, ThemeError::BadModifier { .. }),
        "expected BadModifier, got: {err}"
    );
}

// ── crossed_out (Helix name for strikethrough) ────────────────────────────

#[test]
fn crossed_out_is_accepted_as_strikethrough() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "helix_compat",
        r##"
"keyword" = { fg = "#ff0000", modifiers = ["crossed_out"] }
"##,
    );
    let theme = load_theme("helix_compat", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert!(kw.modifiers.contains(Modifiers::STRIKETHROUGH));
}

// ── Bad underline style ───────────────────────────────────────────────────

#[test]
fn bad_underline_is_error() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_underline",
        r##"
"keyword" = { fg = "#ff0000", underline = "squiggly" }
"##,
    );
    let err = load_theme("bad_underline", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(err, ThemeError::BadUnderline { .. }),
        "expected BadUnderline, got: {err}"
    );
}

// ── Shorthand 3-digit hex ─────────────────────────────────────────────────

#[test]
fn shorthand_hex_expands_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "short", r##""keyword" = "#f0a""##);
    let theme = load_theme("short", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    // #f0a → #ff00aa
    assert_eq!(kw.fg, Some(Rgb(0xff, 0x00, 0xaa)));
}

// ── Path traversal rejection ──────────────────────────────────────────────

#[test]
fn path_traversal_is_rejected() {
    let dir = TempDir::new().unwrap();
    let err = load_theme("../etc/passwd", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(matches!(err, ThemeError::NotFound { .. }));
}

#[test]
fn drive_relative_segment_is_rejected() {
    assert!(!is_safe_theme_name("c:evil"));
}

#[test]
fn quote_embedded_segment_is_rejected() {
    assert!(!is_safe_theme_name("a\"b"));
}

// ── parse_theme ───────────────────────────────────────────────────────────

#[test]
fn parse_theme_handles_palette_indirection() {
    let toml = r##"
"ui.cursor" = { fg = "black", bg = "white" }
"ui.virtual" = { fg = "dark_gray" }

[palette]
black     = "#000000"
white     = "#ffffff"
dark_gray = "#808080"
"##;
    let theme = super::parse_theme(toml).unwrap();

    let cursor = theme.resolve_by_name(crate::types::Scope("ui.cursor"));
    // Independent oracle: expected colors derived directly from palette hex values.
    assert_eq!(cursor.fg, Some(Rgb(0x00, 0x00, 0x00)));
    assert_eq!(cursor.bg, Some(Rgb(0xff, 0xff, 0xff)));

    let virt = theme.resolve_by_name(crate::types::Scope("ui.virtual"));
    assert_eq!(virt.fg, Some(Rgb(0x80, 0x80, 0x80)));
}

#[test]
fn parse_theme_rejects_inherits() {
    let toml = r#"inherits = "base""#;
    // With empty search_paths, `load_recursive("base", &[], …)` must fail NotFound.
    let err = super::parse_theme(toml)
        .err()
        .expect("expected Err for inherits in embedded theme");
    assert!(
        matches!(err, ThemeError::NotFound { .. }),
        "expected NotFound, got: {err}"
    );
}

// ── Independent oracle: expected values built from inputs, not from loader ─

#[test]
fn multiple_modifiers_parse_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "mods",
        r##"
"keyword" = { fg = "#ffffff", modifiers = ["bold", "italic"] }
"##,
    );
    let theme = load_theme("mods", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    // Expected: Modifiers::BOLD | Modifiers::ITALIC (constructed independently)
    let expected = Modifiers::BOLD | Modifiers::ITALIC;
    assert_eq!(kw.modifiers, expected);
}

#[test]
fn underline_style_is_parsed() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underline",
        r##"
"keyword" = { fg = "#ffffff", underline = "wavy" }
"comment" = { fg = "#888888", underline = "solid" }
"##,
    );
    let theme = load_theme("underline", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(kw.underline, UnderlineStyle::Wavy);
    assert_eq!(cm.underline, UnderlineStyle::Solid);
}

// ── Full Helix modifier set ───────────────────────────────────────────────

#[test]
fn all_modifiers_parse_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "all_mods",
        r##"
"keyword" = { modifiers = ["bold","italic","crossed_out","dim","reversed","hidden","slow_blink","rapid_blink"] }
"##,
    );
    let theme = load_theme("all_mods", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    let expected = Modifiers::BOLD
        | Modifiers::ITALIC
        | Modifiers::STRIKETHROUGH
        | Modifiers::DIM
        | Modifiers::REVERSED
        | Modifiers::HIDDEN
        | Modifiers::SLOW_BLINK
        | Modifiers::RAPID_BLINK;
    assert_eq!(kw.modifiers, expected);
}

#[test]
fn underlined_modifier_maps_to_solid() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underlined_mod",
        r##"
"keyword" = { fg = "#ffffff", modifiers = ["underlined"] }
"##,
    );
    let theme = load_theme("underlined_mod", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.underline, UnderlineStyle::Solid);
    assert_eq!(kw.modifiers, Modifiers::empty());
}

#[test]
fn underline_key_wins_over_underlined_modifier() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underline_priority",
        r##"
"keyword" = { underline = "wavy", modifiers = ["underlined"] }
"##,
    );
    let theme = load_theme("underline_priority", &paths(dir.path())).unwrap();
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.underline, UnderlineStyle::Wavy);
}
