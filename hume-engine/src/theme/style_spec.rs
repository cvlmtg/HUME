//! A scope's style before its colors are resolved: the shape a theme file
//! entry and a `set-scope-styles!` entry both parse to, so the two share one
//! vocabulary and one resolver.

use hume_core::ScopeStyleInput;
use hume_grid::Rgb;
use rustc_hash::FxHashMap;

use crate::theme::error::ThemeError;
use crate::theme::values::{
    ANSI_COLORS, UNDERLINE_MODIFIER, parse_hex_color, parse_modifier, parse_underline,
};
use crate::types::{Modifiers, ResolvedStyle, UnderlineStyle};

/// A theme's `[palette]`. `None` is an entry that failed to parse: a color
/// naming it fails too, rather than falling through to an ANSI name.
pub type Palette = FxHashMap<String, Option<Rgb>>;

/// A color as written. A `#` string is a hex literal, checked when parsed;
/// any other string is a name, looked up in the palette and then the ANSI
/// names only when the style is resolved against a theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ColorRef {
    Rgb(Rgb),
    Name(Box<str>),
}

impl ColorRef {
    /// `field` is passed straight to [`ThemeError::BadColor`]: `None` for a
    /// scope's own `fg`/`bg`/shorthand, `Some` for the nested
    /// `underline.color`, whose failure `key` alone can't tell apart.
    pub(crate) fn parse(
        key: &str,
        field: Option<&'static str>,
        s: &str,
    ) -> Result<Self, ThemeError> {
        if !s.starts_with('#') {
            return Ok(Self::Name(s.into()));
        }
        parse_hex_color(s)
            .map(Self::Rgb)
            .map_err(|()| bad_color(key, field, s))
    }

    fn resolve(
        &self,
        key: &str,
        field: Option<&'static str>,
        palette: &Palette,
    ) -> Result<Rgb, ThemeError> {
        let name = match self {
            Self::Rgb(rgb) => return Ok(*rgb),
            Self::Name(name) => name.as_ref(),
        };
        // A theme's palette entry overrides the ANSI name it shares, as
        // Helix's `ThemePalette::new` merges a palette over its name map.
        match palette.get(name) {
            Some(Some(color)) => Ok(*color),
            Some(None) => Err(bad_color(key, field, name)),
            None => ANSI_COLORS
                .iter()
                .find(|(ansi, _)| *ansi == name)
                .map(|&(_, color)| color)
                .ok_or_else(|| bad_color(key, field, name)),
        }
    }
}

pub(crate) fn bad_color(key: &str, field: Option<&'static str>, value: &str) -> ThemeError {
    ThemeError::BadColor {
        key: key.to_owned(),
        field,
        value: value.to_owned(),
    }
}

/// One scope's style with its colors unresolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleSpec {
    pub(crate) fg: Option<ColorRef>,
    pub(crate) bg: Option<ColorRef>,
    pub(crate) underline: UnderlineStyle,
    pub(crate) underline_color: Option<ColorRef>,
    pub(crate) modifiers: Modifiers,
}

impl StyleSpec {
    /// Parse a script's entry, refusing a malformed hex color or an unknown
    /// modifier or underline name. A color name is checked only when the
    /// style is resolved against a theme.
    pub fn from_input(input: &ScopeStyleInput) -> Result<Self, ThemeError> {
        let key = input.scope.as_str();
        let color = |s: &Option<String>, field| {
            s.as_deref()
                .map(|s| ColorRef::parse(key, field, s))
                .transpose()
        };
        let mut spec = Self {
            fg: color(&input.fg, None)?,
            bg: color(&input.bg, None)?,
            underline_color: color(&input.underline_color, Some("underline.color"))?,
            underline: input
                .underline_style
                .as_deref()
                .map(|s| parse_underline(key, s))
                .transpose()?
                .unwrap_or_default(),
            modifiers: Modifiers::empty(),
        };
        for name in &input.modifiers {
            spec.add_modifier(key, name)?;
        }
        Ok(spec)
    }

    /// Add the modifier `name` to the style. `underlined` sets a solid
    /// underline instead, unless an underline style is already set, so an
    /// underline style wins whichever is parsed first.
    pub(crate) fn add_modifier(&mut self, key: &str, name: &str) -> Result<(), ThemeError> {
        if name == UNDERLINE_MODIFIER {
            if self.underline == UnderlineStyle::None {
                self.underline = UnderlineStyle::Solid;
            }
            return Ok(());
        }
        self.modifiers |= parse_modifier(key, name)?;
        Ok(())
    }

    /// Resolve the colors against `palette`. A color that does not resolve
    /// is left unset and warned, keeping the rest of the style.
    pub(crate) fn resolve(&self, key: &str, palette: &Palette) -> (ResolvedStyle, Vec<ThemeError>) {
        let mut warnings = Vec::new();
        let mut color = |c: &Option<ColorRef>, field| {
            c.as_ref()?
                .resolve(key, field, palette)
                .map_err(|e| warnings.push(e))
                .ok()
        };
        let style = ResolvedStyle {
            fg: color(&self.fg, None),
            bg: color(&self.bg, None),
            underline: self.underline,
            underline_color: color(&self.underline_color, Some("underline.color")),
            modifiers: self.modifiers,
        };
        (style, warnings)
    }
}

#[cfg(test)]
mod tests;
