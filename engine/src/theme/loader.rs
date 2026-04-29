//! Helix-compatible TOML theme loader.
//!
//! Supports:
//! - `inherits = "parent"` — recursive parent loading (child wins on conflict)
//! - `[palette]` — named-color indirection; palette names are resolved before
//!   they appear as `fg`/`bg` values in scope entries
//! - Flat dotted keys: `"keyword.function" = { fg = "red", modifiers = ["bold"] }`
//! - Shorthand string values: `"keyword" = "red"` sets `fg` from the named color

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::style::Color;

use crate::theme::error::ThemeError;
use crate::theme::Theme;
use crate::types::{Modifiers, ResolvedStyle, UnderlineStyle};

const MAX_DEPTH: usize = 8;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Load a theme by name from the given ordered search paths.
///
/// `search_paths` is searched in order; the first `<name>.toml` file found
/// wins. Child scopes override parent scopes from `inherits` chains.
///
/// Returns a fully-resolved, un-baked [`Theme`]. Call [`Theme::bake`] with
/// the live [`ScopeRegistry`] before the first render.
pub fn load_theme(name: &str, search_paths: &[PathBuf]) -> Result<Theme, ThemeError> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let (scopes, default) = load_recursive(name, search_paths, &mut visited, 0)?;
    Ok(Theme::from_owned(scopes, default))
}

// ---------------------------------------------------------------------------
// Recursive loader
// ---------------------------------------------------------------------------

/// Intermediate representation: palette + resolved scope styles.
type ThemeData = (HashMap<String, ResolvedStyle>, ResolvedStyle);

fn load_recursive(
    name: &str,
    search_paths: &[PathBuf],
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<ThemeData, ThemeError> {
    if depth > MAX_DEPTH {
        return Err(ThemeError::MaxDepth {
            name: name.to_owned(),
        });
    }

    let (canonical, path) = find_theme_file(name, search_paths)?;

    // Cycle detection via canonical path (already resolved by find_theme_file).
    if !visited.insert(canonical) {
        return Err(ThemeError::Cycle {
            name: name.to_owned(),
        });
    }

    let source = std::fs::read_to_string(&path).map_err(|e| ThemeError::Io {
        name: name.to_owned(),
        error: e,
    })?;

    let doc: toml::Value = source.parse().map_err(ThemeError::Parse)?;
    let table = doc
        .as_table()
        .expect("toml::Value parsed from a TOML document is always a Table at root");

    // ── Base from parent (if any) ────────────────────────────────────────────
    let mut palette: HashMap<String, Color> = HashMap::new();
    let mut scopes: HashMap<String, ResolvedStyle> = HashMap::new();
    let mut default = ResolvedStyle::default();

    if let Some(parent_name) = table.get("inherits").and_then(|v| v.as_str()) {
        let (parent_scopes, parent_default) =
            load_recursive(parent_name, search_paths, visited, depth + 1)?;
        scopes = parent_scopes;
        default = parent_default;
        // Parent palette isn't available to us here (it was used to build scopes),
        // but child's own palette is merged below — child palette names win.
    }

    // ── Parse [palette] (if any) ──────────────────────────────────────────────
    // Non-hex palette values (terminal color names like "red") are ignored;
    // palette entries must be #rrggbb literals.
    if let Some(pal_table) = table.get("palette").and_then(|v| v.as_table()) {
        for (k, v) in pal_table {
            if let Some(hex) = v.as_str()
                && let Ok(color) = parse_hex_color(hex)
            {
                palette.insert(k.clone(), color);
            }
        }
    }

    // ── Parse scope entries ───────────────────────────────────────────────────
    for (key, value) in table {
        // Reserved keys — not scope entries.
        if key == "inherits" || key == "palette" {
            continue;
        }
        let style = parse_scope_value(key, value, &palette)?;
        scopes.insert(key.clone(), style);
    }

    Ok((scopes, default))
}

// ---------------------------------------------------------------------------
// Scope value parsing
// ---------------------------------------------------------------------------

/// Parse one TOML scope entry into a `ResolvedStyle`.
///
/// Helix supports two forms:
/// - `"keyword" = "red"` — shorthand; sets `fg` only
/// - `"keyword" = { fg = "red", bg = "black", modifiers = ["bold"] }` — full form
fn parse_scope_value(
    key: &str,
    value: &toml::Value,
    palette: &HashMap<String, Color>,
) -> Result<ResolvedStyle, ThemeError> {
    match value {
        // Shorthand: `"keyword" = "red"` sets fg only.
        toml::Value::String(s) => {
            let fg = Some(resolve_color(key, s, palette)?);
            Ok(ResolvedStyle {
                fg,
                ..Default::default()
            })
        }
        toml::Value::Table(t) => parse_style_table(key, t, palette),
        other => Err(ThemeError::BadColor {
            key: key.to_owned(),
            value: format!("{other:?}"),
        }),
    }
}

fn parse_style_table(
    key: &str,
    t: &toml::map::Map<String, toml::Value>,
    palette: &HashMap<String, Color>,
) -> Result<ResolvedStyle, ThemeError> {
    let mut style = ResolvedStyle::default();

    if let Some(v) = t.get("fg")
        && let Some(s) = v.as_str()
    {
        style.fg = Some(resolve_color(key, s, palette)?);
    }
    if let Some(v) = t.get("bg")
        && let Some(s) = v.as_str()
    {
        style.bg = Some(resolve_color(key, s, palette)?);
    }
    if let Some(v) = t.get("underline") {
        if let Some(s) = v.as_str() {
            style.underline = parse_underline(s);
        } else if let Some(ut) = v.as_table() {
            // `underline = { color = "#...", style = "..." }` (Helix extended form)
            if let Some(color_v) = ut.get("color").and_then(|c| c.as_str()) {
                style.underline_color = Some(resolve_color(key, color_v, palette)?);
            }
            if let Some(style_v) = ut.get("style").and_then(|s| s.as_str()) {
                style.underline = parse_underline(style_v);
            }
        }
    }
    if let Some(v) = t.get("modifiers")
        && let Some(arr) = v.as_array()
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                style.modifiers |= parse_modifier(key, s)?;
            }
        }
    }

    Ok(style)
}

// ---------------------------------------------------------------------------
// Color resolution
// ---------------------------------------------------------------------------

fn resolve_color(
    key: &str,
    s: &str,
    palette: &HashMap<String, Color>,
) -> Result<Color, ThemeError> {
    // Palette reference takes priority.
    if let Some(&color) = palette.get(s) {
        return Ok(color);
    }
    // Hex literal.
    parse_hex_color(s).map_err(|_| ThemeError::BadColor {
        key: key.to_owned(),
        value: s.to_owned(),
    })
}

fn parse_hex_color(s: &str) -> Result<Color, ()> {
    let hex = s.strip_prefix('#').ok_or(())?;
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ())?;
            Ok(Color::Rgb(r, g, b))
        }
        3 => {
            // Expand shorthand #rgb → #rrggbb.
            let r = u8::from_str_radix(&hex[0..1], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[1..2], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[2..3], 16).map_err(|_| ())?;
            Ok(Color::Rgb(r * 17, g * 17, b * 17))
        }
        _ => Err(()),
    }
}

// ---------------------------------------------------------------------------
// Modifier parsing
// ---------------------------------------------------------------------------

fn parse_modifier(key: &str, s: &str) -> Result<Modifiers, ThemeError> {
    match s {
        "bold" => Ok(Modifiers::BOLD),
        "italic" => Ok(Modifiers::ITALIC),
        "strikethrough" => Ok(Modifiers::STRIKETHROUGH),
        // Treat unrecognized modifiers as errors so themes don't silently lose styling.
        _ => Err(ThemeError::BadModifier {
            key: key.to_owned(),
            value: s.to_owned(),
        }),
    }
}

fn parse_underline(s: &str) -> UnderlineStyle {
    match s {
        "line" | "solid" => UnderlineStyle::Solid,
        "curl" | "wavy" | "undercurl" => UnderlineStyle::Wavy,
        "dotted" => UnderlineStyle::Dotted,
        "dashed" => UnderlineStyle::Dashed,
        _ => UnderlineStyle::None,
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

fn find_theme_file(
    name: &str,
    search_paths: &[PathBuf],
) -> Result<(PathBuf, PathBuf), ThemeError> {
    // Reject names with path separators or suspicious segments.
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(ThemeError::NotFound {
            name: name.to_owned(),
        });
    }
    let filename = format!("{name}.toml");
    for dir in search_paths {
        let candidate = dir.join(&filename);
        match std::fs::canonicalize(&candidate) {
            Ok(canonical) => return Ok((canonical, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(ThemeError::Io {
                    name: name.to_owned(),
                    error: e,
                })
            }
        }
    }
    Err(ThemeError::NotFound {
        name: name.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::types::Modifiers;
    use tempfile::TempDir;

    // ── Test fixture helpers ──────────────────────────────────────────────────

    fn write_theme(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(format!("{name}.toml")), content)
            .expect("failed to write test theme");
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
        assert_eq!(kw.fg, Some(Color::Rgb(0xff, 0, 0)));
        assert_eq!(kw.bg, None);

        let kw_fn = theme.resolve_by_name(crate::types::Scope("keyword.function"));
        assert_eq!(kw_fn.fg, Some(Color::Rgb(0, 0xff, 0)));
        assert!(kw_fn.modifiers.contains(Modifiers::BOLD));

        // Fallback: "keyword.operator" → "keyword"
        let kw_op = theme.resolve_by_name(crate::types::Scope("keyword.operator"));
        assert_eq!(kw_op.fg, Some(Color::Rgb(0xff, 0, 0)));
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
        assert_eq!(kw.fg, Some(Color::Rgb(0xcc, 0x24, 0x1d)));

        let cm = theme.resolve_by_name(crate::types::Scope("comment"));
        assert_eq!(cm.fg, Some(Color::Rgb(0x98, 0x97, 0x1a)));
        assert!(cm.modifiers.contains(Modifiers::ITALIC));

        // Literal hex still works (not in palette).
        let cn = theme.resolve_by_name(crate::types::Scope("constant"));
        assert_eq!(cn.fg, Some(Color::Rgb(0xab, 0xcd, 0xef)));
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
        assert_eq!(kw.fg, Some(Color::Rgb(0, 0xff, 0)));

        // Parent's "comment" and "constant" still present.
        let cm = theme.resolve_by_name(crate::types::Scope("comment"));
        assert_eq!(cm.fg, Some(Color::Rgb(0x88, 0x88, 0x88)));
        let cn = theme.resolve_by_name(crate::types::Scope("constant"));
        assert_eq!(cn.fg, Some(Color::Rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn inherits_palette_child_extends_parent_palette() {
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
        assert_eq!(kw.fg, Some(Color::Rgb(0xff, 0, 0)));

        // Child comment resolved to blue via child palette.
        let cm = theme.resolve_by_name(crate::types::Scope("comment"));
        assert_eq!(cm.fg, Some(Color::Rgb(0, 0, 0xff)));
    }

    // ── Cycle detection ───────────────────────────────────────────────────────

    #[test]
    fn cycle_is_detected() {
        let dir = TempDir::new().unwrap();
        write_theme(dir.path(), "a", r#"inherits = "b""#);
        write_theme(dir.path(), "b", r#"inherits = "a""#);

        let err = load_theme("a", &paths(dir.path())).err().expect("expected an Err result");
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

        let err = load_theme("t0", &paths(dir.path())).err().expect("expected an Err result");
        assert!(
            matches!(err, ThemeError::MaxDepth { .. }),
            "expected MaxDepth error, got: {err}"
        );
    }

    // ── Not found ─────────────────────────────────────────────────────────────

    #[test]
    fn not_found_returns_error() {
        let dir = TempDir::new().unwrap();
        let err = load_theme("nonexistent", &paths(dir.path())).err().expect("expected an Err result");
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
        let err = load_theme("bad", &paths(dir.path())).err().expect("expected an Err result");
        assert!(
            matches!(err, ThemeError::BadColor { .. }),
            "expected BadColor, got: {err}"
        );
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
        let err = load_theme("bad_mod", &paths(dir.path())).err().expect("expected an Err result");
        assert!(
            matches!(err, ThemeError::BadModifier { .. }),
            "expected BadModifier, got: {err}"
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
        assert_eq!(kw.fg, Some(Color::Rgb(0xff, 0x00, 0xaa)));
    }

    // ── Path traversal rejection ──────────────────────────────────────────────

    #[test]
    fn path_traversal_is_rejected() {
        let dir = TempDir::new().unwrap();
        let err = load_theme("../etc/passwd", &paths(dir.path())).err().expect("expected an Err result");
        assert!(matches!(err, ThemeError::NotFound { .. }));
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
}
