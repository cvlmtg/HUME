//! Helix-compatible TOML theme loader.
//!
//! Supports:
//! - `inherits = "parent"` — recursive parent loading (child wins on conflict)
//! - `[palette]` — named-color indirection; palette names are resolved before
//!   they appear as `fg`/`bg` values in scope entries
//! - Flat dotted keys: `"keyword.function" = { fg = "red", modifiers = ["bold"] }`
//! - Shorthand string values: `"keyword" = "red"` sets `fg` from the named color

use std::path::PathBuf;

use rustc_hash::{FxHashMap, FxHashSet};

use hume_grid::Rgb;

use crate::theme::Theme;
use crate::theme::error::ThemeError;
use crate::types::{Modifiers, ResolvedStyle, UnderlineStyle};

const MAX_DEPTH: usize = 8;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Load a theme by name from the given ordered search paths.
///
/// `search_paths` is searched in order; the first `<name>.toml` file found
/// wins. Child scopes override parent scopes from `inherits` chains.
///
/// Returns a fully-resolved, un-baked [`Theme`]. Call [`Theme::bake`] with
/// the live [`crate::theme::ScopeRegistry`] before the first render.
pub fn load_theme(name: &str, search_paths: &[PathBuf]) -> Result<Theme, ThemeError> {
    let mut visited: FxHashSet<PathBuf> = FxHashSet::default();
    let (scopes, default) = load_recursive(name, search_paths, &mut visited, 0)?;
    Ok(Theme::from_owned(scopes, default))
}

/// Parse a theme from a TOML string.
///
/// Supports palette indirection and all scope value forms, but does **not**
/// support `inherits` — the document must be a self-contained leaf. Passing a
/// document with `inherits` returns [`ThemeError::NotFound`] for the named
/// parent.
///
/// Intended for embedded themes (e.g. `include_str!` in the binary).
pub fn parse_theme(toml_str: &str) -> Result<Theme, ThemeError> {
    let mut visited: FxHashSet<PathBuf> = FxHashSet::default();
    // Empty search_paths: any `inherits` key will fail with NotFound, which is
    // the correct behaviour for a self-contained embedded document.
    let (scopes, default) = parse_recursive(toml_str, &[], &mut visited, 0)?;
    Ok(Theme::from_owned(scopes, default))
}

// ---------------------------------------------------------------------------
// Recursive loader
// ---------------------------------------------------------------------------

/// Intermediate representation: resolved scope styles + default style.
type ThemeData = (FxHashMap<String, ResolvedStyle>, ResolvedStyle);

fn load_recursive(
    name: &str,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
    depth: usize,
) -> Result<ThemeData, ThemeError> {
    if depth > MAX_DEPTH {
        return Err(ThemeError::MaxDepth {
            name: name.to_owned(),
        });
    }

    let (canonical, source) = find_theme_file(name, search_paths)?;

    // Cycle detection via canonical path.
    if !visited.insert(canonical) {
        return Err(ThemeError::Cycle {
            name: name.to_owned(),
        });
    }

    parse_recursive(&source, search_paths, visited, depth)
}

/// Parse one TOML document into `ThemeData`, recursing into `inherits` parents
/// via `load_recursive` when `search_paths` is non-empty.
fn parse_recursive(
    source: &str,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
    depth: usize,
) -> Result<ThemeData, ThemeError> {
    let table: toml::Table = source.parse().map_err(ThemeError::Parse)?;

    // ── Base from parent (if any) ────────────────────────────────────────────
    let mut palette: FxHashMap<String, Rgb> = FxHashMap::default();
    let mut scopes: FxHashMap<String, ResolvedStyle> = FxHashMap::default();
    let mut default = ResolvedStyle::default();

    if let Some(parent_name) = table.get("inherits").and_then(|v| v.as_str()) {
        let (parent_scopes, parent_default) =
            load_recursive(parent_name, search_paths, visited, depth + 1)?;
        scopes = parent_scopes;
        default = parent_default;
        // Parent palette is not exposed to the child — it was consumed while
        // resolving the parent's scopes. Child defines its own palette below.
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
    for (key, value) in &table {
        // Reserved keys — not scope entries.
        if key == "inherits" || key == "palette" {
            continue;
        }
        let style = parse_scope_value(key, value, &palette)?;
        scopes.insert(key.clone(), style);
    }

    // `ui.text` (Helix's base-foreground convention) folds into `default`, the
    // style every cell starts from (see `style::apply_styles`). Without this,
    // plain/unhighlighted text has `fg: None` → renders as the terminal's own
    // default colour, which the pane dim has no numeric value to blend and so
    // leaves at full strength in an unfocused pane.
    if let Some(ui_text) = scopes.get("ui.text") {
        default = default.layer(*ui_text);
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
    palette: &FxHashMap<String, Rgb>,
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
        other => Err(ThemeError::BadScopeValue {
            key: key.to_owned(),
            value: format!("{other:?}"),
        }),
    }
}

fn parse_style_table(
    key: &str,
    t: &toml::map::Map<String, toml::Value>,
    palette: &FxHashMap<String, Rgb>,
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
            style.underline = parse_underline(key, s)?;
        } else if let Some(ut) = v.as_table() {
            // `underline = { color = "#...", style = "..." }` (Helix extended form)
            if let Some(color_v) = ut.get("color").and_then(|c| c.as_str()) {
                style.underline_color = Some(resolve_color(key, color_v, palette)?);
            }
            if let Some(style_v) = ut.get("style").and_then(|s| s.as_str()) {
                style.underline = parse_underline(key, style_v)?;
            }
        }
    }
    if let Some(v) = t.get("modifiers")
        && let Some(arr) = v.as_array()
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                match s {
                    // Helix exposes underline as a modifier; route it to the dedicated
                    // underline field so underline has a single source of truth. A more
                    // specific `underline = "..."` key (parsed just above) wins.
                    "underlined" => {
                        if style.underline == UnderlineStyle::None {
                            style.underline = UnderlineStyle::Solid;
                        }
                    }
                    _ => style.modifiers |= parse_modifier(key, s)?,
                }
            }
        }
    }

    Ok(style)
}

// ---------------------------------------------------------------------------
// Colour resolution
// ---------------------------------------------------------------------------

fn resolve_color(key: &str, s: &str, palette: &FxHashMap<String, Rgb>) -> Result<Rgb, ThemeError> {
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

fn parse_hex_color(s: &str) -> Result<Rgb, ()> {
    let hex = s.strip_prefix('#').ok_or(())?;
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ())?;
            Ok(Rgb(r, g, b))
        }
        3 => {
            // Expand shorthand #rgb → #rrggbb.
            let r = u8::from_str_radix(&hex[0..1], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[1..2], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[2..3], 16).map_err(|_| ())?;
            Ok(Rgb(r * 17, g * 17, b * 17))
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
        // Accept both "strikethrough" (HUME name) and "crossed_out" (Helix name).
        "strikethrough" | "crossed_out" => Ok(Modifiers::STRIKETHROUGH),
        "dim" => Ok(Modifiers::DIM),
        "reversed" => Ok(Modifiers::REVERSED),
        "hidden" => Ok(Modifiers::HIDDEN),
        "slow_blink" => Ok(Modifiers::SLOW_BLINK),
        "rapid_blink" => Ok(Modifiers::RAPID_BLINK),
        // Treat unrecognized modifiers as errors so themes don't silently lose styling.
        _ => Err(ThemeError::BadModifier {
            key: key.to_owned(),
            value: s.to_owned(),
        }),
    }
}

fn parse_underline(key: &str, s: &str) -> Result<UnderlineStyle, ThemeError> {
    match s {
        "line" | "solid" => Ok(UnderlineStyle::Solid),
        "curl" | "wavy" | "undercurl" => Ok(UnderlineStyle::Wavy),
        "dotted" => Ok(UnderlineStyle::Dotted),
        "dashed" => Ok(UnderlineStyle::Dashed),
        _ => Err(ThemeError::BadUnderline {
            key: key.to_owned(),
            value: s.to_owned(),
        }),
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// Returns `(canonical_path, source)`.
///
/// Reads the file first (matching on `NotFound` to skip to the next search dir),
/// then canonicalizes the path for cycle detection. Canonicalize failure after a
/// successful read uses the unresolved path as the cycle key — safe because a
/// deleted-after-read file cannot form a cycle.
fn find_theme_file(name: &str, search_paths: &[PathBuf]) -> Result<(PathBuf, String), ThemeError> {
    // Reject names with path separators or suspicious segments.
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(ThemeError::NotFound {
            name: name.to_owned(),
        });
    }
    let filename = format!("{name}.toml");
    for dir in search_paths {
        let candidate = dir.join(&filename);
        match std::fs::read_to_string(&candidate) {
            Ok(source) => {
                // Canonicalize after read — residual race only affects cycle-key
                // accuracy, not file content. Not a security prefix check.
                let canonical = std::fs::canonicalize(&candidate).unwrap_or(candidate);
                return Ok((canonical, source));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(ThemeError::Io {
                    name: name.to_owned(),
                    error: e,
                });
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
mod tests;
