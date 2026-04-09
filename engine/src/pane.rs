use std::str::FromStr;

use crate::providers::ProviderSet;
use crate::types::{EditorMode, Selection};
use ropey::Rope;

// ---------------------------------------------------------------------------
// Viewport state  (per-pane scroll / size)
// ---------------------------------------------------------------------------

/// The scrolling and sizing state of one pane's viewport.
#[derive(Clone, Debug)]
pub struct ViewportState {
    /// First fully-visible buffer line.
    pub top_line: usize,
    /// How many display rows of `top_line` to skip (sub-row offset for
    /// partially-scrolled wrapped lines).
    pub top_row_offset: u16,
    /// Horizontal scroll in columns (0 when soft-wrap is on).
    pub horizontal_offset: u16,
    /// Total width of the pane in terminal cells (gutter + content).
    pub width: u16,
    /// Total height of the pane in terminal cells.
    pub height: u16,
}

impl ViewportState {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            top_line: 0,
            top_row_offset: 0,
            horizontal_offset: 0,
            width,
            height,
        }
    }
}

// ---------------------------------------------------------------------------
// Wrap mode
// ---------------------------------------------------------------------------

/// How the formatter handles lines that exceed the content width.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// No wrapping — horizontal scroll.
    #[default]
    None,
    /// Break at `width` columns.
    Soft { width: u16 },
    /// Break at whitespace boundaries; prefer not to split words.
    Word { width: u16 },
    /// Word wrap + indent continuation rows to match the line's indent level.
    Indent { width: u16 },
}

impl FromStr for WrapMode {
    type Err = String;

    /// Parse a wrap mode from a string.
    ///
    /// Accepted forms: `none`, `soft:N`, `word:N`, `indent:N`
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        if lower == "none" {
            return Ok(WrapMode::None);
        }
        let (kind, rest) = lower.split_once(':').ok_or_else(|| {
            format!("invalid wrap-mode '{s}': expected none, soft:N, word:N, or indent:N")
        })?;
        let width: u16 = rest.parse().map_err(|_| {
            format!("invalid wrap-mode width in '{s}': expected a column count, got '{rest}'")
        })?;
        if width == 0 {
            return Err(format!("invalid wrap-mode width in '{s}': must be at least 1"));
        }
        match kind {
            "soft" => Ok(WrapMode::Soft { width }),
            "word" => Ok(WrapMode::Word { width }),
            "indent" => Ok(WrapMode::Indent { width }),
            _ => Err(format!("invalid wrap-mode kind '{kind}' in '{s}': expected soft, word, or indent")),
        }
    }
}

impl WrapMode {
    pub fn wrap_width(&self) -> Option<u16> {
        match self {
            WrapMode::None => None,
            WrapMode::Soft { width } | WrapMode::Word { width } | WrapMode::Indent { width } => {
                Some(*width)
            }
        }
    }

    pub fn is_wrapping(&self) -> bool {
        !matches!(self, WrapMode::None)
    }
}

// ---------------------------------------------------------------------------
// Whitespace indicators
// ---------------------------------------------------------------------------

/// When to render a whitespace indicator for a particular whitespace type.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WhitespaceRender {
    /// Never render an indicator.
    #[default]
    None,
    /// Always render an indicator.
    All,
    /// Only render for trailing whitespace (before end-of-line).
    Trailing,
}

impl FromStr for WhitespaceRender {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(WhitespaceRender::None),
            "all" => Ok(WhitespaceRender::All),
            "trailing" => Ok(WhitespaceRender::Trailing),
            _ => Err(format!(
                "invalid whitespace render '{s}': expected none, all, or trailing"
            )),
        }
    }
}

/// Configuration for whitespace indicator rendering.
#[derive(Clone, Debug)]
pub struct WhitespaceConfig {
    pub space: WhitespaceRender,
    pub tab: WhitespaceRender,
    pub newline: WhitespaceRender,
    /// Character to show in place of a space when rendered.
    pub space_char: &'static str,
    /// Character to show at the start of a tab expansion.
    pub tab_char: &'static str,
    /// Character to show in place of a newline when rendered.
    pub newline_char: &'static str,
}

impl Default for WhitespaceConfig {
    fn default() -> Self {
        Self {
            space: WhitespaceRender::None,
            tab: WhitespaceRender::None,
            newline: WhitespaceRender::None,
            space_char: "·",
            tab_char: "→",
            newline_char: "⏎",
        }
    }
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

/// A single editor pane — an independent view into a buffer.
pub struct Pane {
    /// Which buffer this pane views.
    pub buffer_id: crate::pipeline::BufferId,
    /// Scroll and size state.
    pub viewport: ViewportState,
    /// All active selections, sorted in ascending document order.
    pub selections: Vec<Selection>,
    /// Index of the primary selection within `selections`.
    pub primary_idx: usize,
    /// Current editor mode.
    pub mode: EditorMode,
    /// Wrap mode for the content area.
    pub wrap_mode: WrapMode,
    /// Tab stop width.
    pub tab_width: u8,
    /// Whitespace indicator configuration.
    pub whitespace: WhitespaceConfig,
    /// Registered providers for this pane.
    pub providers: ProviderSet,
}

impl Pane {
    /// Line index of the primary selection head, resolved via the rope.
    ///
    /// Called once per frame from the pipeline — O(log n) rope lookup.
    /// Panics in debug builds if the pane has no selections.
    pub fn primary_head_line(&self, rope: &Rope) -> usize {
        debug_assert!(!self.selections.is_empty(), "pane has no selections");
        let head_char = self.selections.get(self.primary_idx).map(|s| s.head).unwrap_or(0);
        rope.char_to_line(head_char)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Selection;

    #[test]
    fn viewport_state_defaults() {
        let vp = ViewportState::new(80, 24);
        assert_eq!(vp.top_line, 0);
        assert_eq!(vp.top_row_offset, 0);
        assert_eq!(vp.horizontal_offset, 0);
        assert_eq!(vp.width, 80);
        assert_eq!(vp.height, 24);
    }

    // ── WrapMode::FromStr ────────────────────────────────────────────────

    #[test]
    fn wrap_mode_from_str_none() {
        assert_eq!("none".parse::<WrapMode>().unwrap(), WrapMode::None);
        assert_eq!("NONE".parse::<WrapMode>().unwrap(), WrapMode::None);
    }

    #[test]
    fn wrap_mode_from_str_variants() {
        assert_eq!("soft:80".parse::<WrapMode>().unwrap(), WrapMode::Soft { width: 80 });
        assert_eq!("word:40".parse::<WrapMode>().unwrap(), WrapMode::Word { width: 40 });
        assert_eq!("indent:76".parse::<WrapMode>().unwrap(), WrapMode::Indent { width: 76 });
    }

    #[test]
    fn wrap_mode_from_str_case_insensitive() {
        assert_eq!("Soft:80".parse::<WrapMode>().unwrap(), WrapMode::Soft { width: 80 });
        assert_eq!("INDENT:76".parse::<WrapMode>().unwrap(), WrapMode::Indent { width: 76 });
    }

    #[test]
    fn wrap_mode_from_str_error_unknown_kind() {
        assert!("hard:80".parse::<WrapMode>().is_err());
    }

    #[test]
    fn wrap_mode_from_str_error_zero_width() {
        let err = "soft:0".parse::<WrapMode>().unwrap_err();
        assert!(err.contains("soft:0"), "error should contain input: {err}");
    }

    #[test]
    fn wrap_mode_from_str_error_missing_colon() {
        assert!("soft".parse::<WrapMode>().is_err());
    }

    #[test]
    fn wrap_mode_from_str_error_non_numeric_width() {
        assert!("soft:abc".parse::<WrapMode>().is_err());
    }

    // ── WhitespaceRender::FromStr ─────────────────────────────────────────

    #[test]
    fn whitespace_render_from_str_all_variants() {
        assert_eq!("none".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::None);
        assert_eq!("all".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::All);
        assert_eq!("trailing".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::Trailing);
    }

    #[test]
    fn whitespace_render_from_str_case_insensitive() {
        assert_eq!("None".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::None);
        assert_eq!("ALL".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::All);
        assert_eq!("Trailing".parse::<WhitespaceRender>().unwrap(), WhitespaceRender::Trailing);
    }

    #[test]
    fn whitespace_render_from_str_error() {
        let err = "always".parse::<WhitespaceRender>().unwrap_err();
        assert!(err.contains("always"), "error should contain input: {err}");
    }

    #[test]
    fn wrap_mode_wrap_width() {
        assert_eq!(WrapMode::None.wrap_width(), None);
        assert_eq!(WrapMode::Soft { width: 80 }.wrap_width(), Some(80));
        assert_eq!(WrapMode::Word { width: 40 }.wrap_width(), Some(40));
        assert_eq!(WrapMode::Indent { width: 60 }.wrap_width(), Some(60));
    }

    #[test]
    fn wrap_mode_is_wrapping() {
        assert!(!WrapMode::None.is_wrapping());
        assert!(WrapMode::Soft { width: 80 }.is_wrapping());
        assert!(WrapMode::Word { width: 80 }.is_wrapping());
        assert!(WrapMode::Indent { width: 80 }.is_wrapping());
    }

    #[test]
    fn whitespace_config_defaults() {
        let wc = WhitespaceConfig::default();
        assert_eq!(wc.space, WhitespaceRender::None);
        assert_eq!(wc.tab, WhitespaceRender::None);
        assert_eq!(wc.newline, WhitespaceRender::None);
        assert_eq!(wc.space_char, "·");
        assert_eq!(wc.tab_char, "→");
        assert_eq!(wc.newline_char, "⏎");
    }

    fn make_pane_at_char(head_char: usize) -> Pane {
        Pane {
            buffer_id: crate::pipeline::BufferId::default(),
            viewport: ViewportState::new(80, 24),
            selections: vec![Selection { anchor: head_char, head: head_char }],
            primary_idx: 0,
            mode: crate::types::EditorMode::Normal,
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            providers: crate::providers::ProviderSet::new(),
        }
    }

    #[test]
    fn primary_head_line_returns_head_line() {
        // "aaa\nbbb\nccc" — line 0 is chars 0..3, line 1 is chars 4..7, line 2 is chars 8..11.
        // Char 8 (start of line 2) should resolve to line 2.
        let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
        let pane = make_pane_at_char(8); // first char of line 2
        assert_eq!(pane.primary_head_line(&rope), 2);
    }

    #[test]
    fn primary_head_line_uses_primary_idx() {
        // Two selections; primary_idx points to the second one (on line 2).
        // "aaa\nbbb\nccc": char 0 = line 0, char 8 = line 2.
        let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
        let mut pane = make_pane_at_char(0); // first selection on line 0
        pane.selections.push(Selection { anchor: 8, head: 8 }); // second on line 2
        pane.primary_idx = 1;
        assert_eq!(pane.primary_head_line(&rope), 2);
    }
}
