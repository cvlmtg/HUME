//! What the Tab key inserts in Insert mode.

use std::fmt;
use std::str::FromStr;

/// `Hard` inserts a literal `\t` character; `Soft` inserts enough spaces to
/// reach the next tab stop (governed by `tab-width`). This is the single knob
/// — there is no separate "shiftwidth" or "softtabstop": `tab-width` is the
/// only width, used for both rendering and Tab-key spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabStyle {
    /// Tab key inserts one `\t` character per press.
    #[default]
    Hard,
    /// Tab key inserts spaces up to the next tab stop.
    Soft,
}

impl TabStyle {
    /// The wire-format strings `FromStr` accepts — the single source
    /// `:set buffer tab-style=<Tab>` completion mirrors, so the two can never
    /// drift out of sync.
    pub const VALUES: &'static [&'static str] = &["hard", "soft"];
}

impl fmt::Display for TabStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hard => f.write_str("hard"),
            Self::Soft => f.write_str("soft"),
        }
    }
}

impl FromStr for TabStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "hard" => Ok(Self::Hard),
            "soft" => Ok(Self::Soft),
            _ => Err(format!(
                "invalid tab-style: expected 'hard' or 'soft', got '{s}'"
            )),
        }
    }
}
