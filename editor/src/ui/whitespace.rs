/// When to show visual indicators for a given whitespace character type.
///
/// Each whitespace type (space, tab, newline) can be configured independently,
/// following the Helix `[editor.whitespace.render]` model — the Rust equivalent
/// of vim's `listchars`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)] // All variants used in tests; wired to Steel config in a later milestone.
pub(crate) enum WhitespaceShow {
    /// No indicator — the character renders normally (or invisibly).
    #[default]
    None,
    /// Show indicators for all occurrences.
    All,
    /// Show indicators only for trailing whitespace (whitespace between the
    /// last non-whitespace grapheme and the end of the line).
    Trailing,
}

/// Per-type render settings: which whitespace types get visual indicators.
///
/// Default: all `None` — existing behaviour, no indicators anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WhitespaceRender {
    pub space: WhitespaceShow,
    pub tab: WhitespaceShow,
    pub newline: WhitespaceShow,
}

/// Replacement characters drawn in place of invisible whitespace.
///
/// Only used when the corresponding [`WhitespaceShow`] setting is not `None`.
/// Chosen to be unambiguous single-width Unicode characters that are clearly
/// distinct from normal content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WhitespaceChars {
    /// Indicator for a space character. Default: `·` (U+00B7 MIDDLE DOT).
    pub space: char,
    /// Indicator for the first column of a tab stop. Default: `→` (U+2192 RIGHTWARDS ARROW).
    pub tab: char,
    /// Indicator for a newline at end-of-line. Default: `⏎` (U+23CE RETURN SYMBOL).
    pub newline: char,
}

impl Default for WhitespaceChars {
    fn default() -> Self {
        Self {
            space: '·',
            tab: '→',
            newline: '⏎',
        }
    }
}

/// Complete whitespace rendering configuration.
///
/// Controls which whitespace characters get visual indicators and what
/// characters are used to represent them. Stored on [`ViewState`] alongside
/// other display-only settings.
///
/// Default: all indicators off — existing rendering unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WhitespaceConfig {
    pub render: WhitespaceRender,
    pub chars: WhitespaceChars,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_nothing() {
        let cfg = WhitespaceConfig::default();
        assert_eq!(cfg.render.space, WhitespaceShow::None);
        assert_eq!(cfg.render.tab, WhitespaceShow::None);
        assert_eq!(cfg.render.newline, WhitespaceShow::None);
    }

    #[test]
    fn default_chars() {
        let chars = WhitespaceChars::default();
        assert_eq!(chars.space, '·');
        assert_eq!(chars.tab, '→');
        assert_eq!(chars.newline, '⏎');
    }
}
