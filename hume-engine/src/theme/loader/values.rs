//! Colour and modifier/underline name resolution. Pure `&str -> value`
//! maps, no outward dependencies beyond `ThemeError`.

use hume_grid::Rgb;
use rustc_hash::FxHashMap;

use crate::theme::error::ThemeError;
use crate::types::{Modifiers, UnderlineStyle};

/// `field` is passed straight to [`ThemeError::BadColor`] — `None` for a
/// scope's own `fg`/`bg`/shorthand, `Some` for the nested `underline.color`,
/// whose failure `key` alone can't distinguish from those.
pub(super) fn resolve_color(
    key: &str,
    field: Option<&'static str>,
    s: &str,
    palette: &FxHashMap<String, Option<Rgb>>,
) -> Result<Rgb, ThemeError> {
    // Palette reference takes priority. A palette entry that was itself
    // malformed (`Some(None)`, already warned when `[palette]` was parsed)
    // must not fall through to an ANSI/hex guess at the same name — that
    // would silently recolor every referencing scope instead of cascading
    // the warning.
    match palette.get(s) {
        Some(Some(color)) => return Ok(*color),
        Some(None) => {
            return Err(ThemeError::BadColor {
                key: key.to_owned(),
                field,
                value: s.to_owned(),
            });
        }
        None => {}
    }
    // Built-in ANSI name — a theme's own palette entry of the same name
    // (checked above) overrides it, matching how Helix's `ThemePalette::new`
    // merges a theme's palette over its default name map.
    if let Some(&(_, color)) = ANSI_COLORS.iter().find(|(name, _)| *name == s) {
        return Ok(color);
    }
    // Hex literal.
    parse_hex_color(s).map_err(|_| ThemeError::BadColor {
        key: key.to_owned(),
        field,
        value: s.to_owned(),
    })
}

/// The sixteen terminal colour names Helix themes may use in place of a hex
/// literal, resolved to fixed RGB values rather than the terminal's actual
/// configured colours.
///
/// HUME requires a numeric [`Rgb`] to blend for the inactive-pane dim
/// ([`Rgb::lerp`](hume_grid::Rgb::lerp)), and there is no way to ask a
/// terminal what its `red` currently renders as: `termina` only models OSC
/// 10-19 (cursor/foreground/background), not the OSC 4 palette-query xterm
/// extension a name like `red` would need. So a name means the same fixed
/// colour on every terminal, rather than tracking a user's own palette
/// customization the way the *terminal's* rendering of `red` would.
///
/// Values are the xterm default palette — verified against Helix's own
/// `Color` → backend conversions (`helix-view/src/graphics.rs`), which agree
/// on every index across both its crossterm and termina impls. Note the
/// index order: `gray` is 8 (bright black) and `light-gray` is 7 (the
/// non-bright palette's *white* slot) — Helix's mapping, not a typo here.
pub(super) const ANSI_COLORS: [(&str, Rgb); 16] = [
    ("black", Rgb(0x00, 0x00, 0x00)),
    ("red", Rgb(0xcd, 0x00, 0x00)),
    ("green", Rgb(0x00, 0xcd, 0x00)),
    ("yellow", Rgb(0xcd, 0xcd, 0x00)),
    ("blue", Rgb(0x00, 0x00, 0xee)),
    ("magenta", Rgb(0xcd, 0x00, 0xcd)),
    ("cyan", Rgb(0x00, 0xcd, 0xcd)),
    ("light-gray", Rgb(0xe5, 0xe5, 0xe5)),
    ("gray", Rgb(0x7f, 0x7f, 0x7f)),
    ("light-red", Rgb(0xff, 0x00, 0x00)),
    ("light-green", Rgb(0x00, 0xff, 0x00)),
    ("light-yellow", Rgb(0xff, 0xff, 0x00)),
    ("light-blue", Rgb(0x5c, 0x5c, 0xff)),
    ("light-magenta", Rgb(0xff, 0x00, 0xff)),
    ("light-cyan", Rgb(0x00, 0xff, 0xff)),
    ("white", Rgb(0xff, 0xff, 0xff)),
];

// pub(super): loader.rs calls this directly too, from resolve_theme_table's
// palette-parsing loop, in addition to resolve_color's own use of it below.
pub(super) fn parse_hex_color(s: &str) -> Result<Rgb, ()> {
    let hex = s.strip_prefix('#').ok_or(())?;
    // `from_str_radix` accepts a leading sign, which would make `#+f0000`
    // parse as `Rgb(15, 0, 0)`. Nothing but hex digits is a colour here.
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
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

/// `parse_modifier`'s vocabulary as a table rather than a `match`, so
/// `hume-engine/src/theme/loader/vocabulary.rs` can enumerate it for
/// `tools/theme-editor/src/lib/vocabulary.generated.js` — a `match`'s arms
/// aren't a value anything can iterate. `crossed_out` maps to
/// `Modifiers::STRIKETHROUGH`: the TOML word is Helix's own spelling, the
/// flag is HUME's.
pub(super) const MODIFIER_NAMES: [(&str, Modifiers); 8] = [
    ("bold", Modifiers::BOLD),
    ("italic", Modifiers::ITALIC),
    ("crossed_out", Modifiers::STRIKETHROUGH),
    ("dim", Modifiers::DIM),
    ("reversed", Modifiers::REVERSED),
    ("hidden", Modifiers::HIDDEN),
    ("slow_blink", Modifiers::SLOW_BLINK),
    ("rapid_blink", Modifiers::RAPID_BLINK),
];

/// The one `modifiers = [...]` literal the loader accepts outside
/// [`MODIFIER_NAMES`] — intercepted in `parse_style_table` (`loader.rs`)
/// before `parse_modifier` ever sees it, and routed to the dedicated
/// underline field instead of the modifier bitset. Named so it can join
/// [`MODIFIER_NAMES`] in the generated vocabulary rather than being a bare
/// literal only `loader.rs`'s match arm and the JS side each know about.
pub(super) const UNDERLINE_MODIFIER: &str = "underlined";

pub(super) fn parse_modifier(key: &str, s: &str) -> Result<Modifiers, ThemeError> {
    MODIFIER_NAMES
        .iter()
        .find(|(name, _)| *name == s)
        .map(|&(_, m)| m)
        // Treat unrecognized modifiers as errors so themes don't silently lose styling.
        .ok_or_else(|| ThemeError::BadModifier {
            key: key.to_owned(),
            value: s.to_owned(),
        })
}

/// `parse_underline`'s vocabulary as a table — same reason as
/// [`MODIFIER_NAMES`].
pub(super) const UNDERLINE_NAMES: [(&str, UnderlineStyle); 5] = [
    ("line", UnderlineStyle::Solid),
    ("curl", UnderlineStyle::Wavy),
    ("dotted", UnderlineStyle::Dotted),
    ("dashed", UnderlineStyle::Dashed),
    ("double_line", UnderlineStyle::Double),
];

pub(super) fn parse_underline(key: &str, s: &str) -> Result<UnderlineStyle, ThemeError> {
    UNDERLINE_NAMES
        .iter()
        .find(|(name, _)| *name == s)
        .map(|&(_, u)| u)
        .ok_or_else(|| ThemeError::BadUnderline {
            key: key.to_owned(),
            value: s.to_owned(),
        })
}
