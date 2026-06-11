use std::str::FromStr;

use slotmap::SecondaryMap;

use crate::layout::gutter_width_for_line;
use crate::pipeline::BufferId;
use crate::providers::ProviderSet;
use crate::types::Selection;
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
// Scroll position  (per-pane, per-buffer scroll memory)
// ---------------------------------------------------------------------------

/// Saved scroll position for one (pane, buffer) pair.
///
/// Stored in `Pane::saved_scrolls` so each pane remembers where it was in a
/// buffer when it switches away. Restored by `recall_scroll` on switch-back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollPosition {
    pub top_line: usize,
    pub top_row_offset: u16,
    pub horizontal_offset: u16,
}

// ---------------------------------------------------------------------------
// Wrap mode
// ---------------------------------------------------------------------------

/// How the formatter handles lines that exceed the content width.
///
/// For `Soft`, `Word`, and `Indent`, `width: 0` is a sentinel meaning "wrap at
/// the content width" (pane width minus gutter). Call `WrapMode::resolve(content_width)`
/// to substitute a concrete column count before handing the mode to engine code.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// No wrapping — horizontal scroll.
    #[default]
    None,
    /// Break at `width` columns (`0` = content width sentinel).
    Soft { width: u16 },
    /// Break at whitespace boundaries; prefer not to split words (`0` = content width sentinel).
    Word { width: u16 },
    /// Word wrap + indent continuation rows to match the line's indent level (`0` = content width sentinel).
    Indent { width: u16 },
}

impl FromStr for WrapMode {
    type Err = String;

    /// Parse a wrap mode from a string.
    ///
    /// Accepted forms:
    /// - `none`                 — no wrapping
    /// - `soft` / `word` / `indent` — wrap at terminal width
    /// - `soft:N` / `word:N` / `indent:N` — wrap at column N (N=0 also means terminal width)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        if lower == "none" {
            return Ok(WrapMode::None);
        }
        // Bare keyword with no colon → sentinel width 0 (terminal width).
        if lower == "soft"   { return Ok(WrapMode::Soft   { width: 0 }); }
        if lower == "word"   { return Ok(WrapMode::Word   { width: 0 }); }
        if lower == "indent" { return Ok(WrapMode::Indent { width: 0 }); }
        let (kind, rest) = lower.split_once(':').ok_or_else(|| {
            format!("invalid wrap-mode '{s}': expected none, soft[:N], word[:N], or indent[:N]")
        })?;
        let width: u16 = rest.parse().map_err(|_| {
            format!("invalid wrap-mode width in '{s}': expected a column count, got '{rest}'")
        })?;
        match kind {
            "soft"   => Ok(WrapMode::Soft   { width }),
            "word"   => Ok(WrapMode::Word   { width }),
            "indent" => Ok(WrapMode::Indent { width }),
            _ => Err(format!(
                "invalid wrap-mode kind '{kind}' in '{s}': expected soft, word, or indent"
            )),
        }
    }
}

impl WrapMode {
    /// Concrete wrap column, or `None` if wrapping is off.
    ///
    /// The caller must have already resolved the `width: 0` sentinel via
    /// `WrapMode::resolve(content_width)` — passing an unresolved sentinel is
    /// a bug and panics in both debug and release. The alternative (returning
    /// `None` or `Some(0)`) would silently format at column 0 in production.
    pub fn wrap_width(&self) -> Option<u16> {
        match self {
            WrapMode::None => None,
            WrapMode::Soft { width }
            | WrapMode::Word { width }
            | WrapMode::Indent { width } => {
                assert!(
                    *width != 0,
                    "wrap_width() received unresolved sentinel (width: 0) — \
                     call WrapMode::resolve(content_width) before reaching the engine",
                );
                Some(*width)
            }
        }
    }

    /// Replace the `width: 0` sentinel with a concrete column count.
    ///
    /// `WrapMode::None` and concrete non-zero widths pass through unchanged.
    /// Call this once at the editor→engine boundary, passing `pane_width − gutter_width`.
    pub fn resolve(self, content_width: u16) -> WrapMode {
        match self {
            WrapMode::Soft   { width: 0 } => WrapMode::Soft   { width: content_width },
            WrapMode::Word   { width: 0 } => WrapMode::Word   { width: content_width },
            WrapMode::Indent { width: 0 } => WrapMode::Indent { width: content_width },
            other => other,
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
    pub buffer_id: BufferId,
    /// Scroll and size state.
    pub viewport: ViewportState,
    /// Per-buffer scroll memory: where this pane was when it last viewed each buffer.
    /// Populated by `remember_scroll` on buffer switch; restored by `recall_scroll`.
    pub saved_scrolls: SecondaryMap<BufferId, ScrollPosition>,
    /// All active selections, sorted by `head` position.
    /// (`SelectionSet` is start-sorted; `populate_sorted_sels` asserts head order.)
    pub selections: Vec<Selection>,
    /// Index of the primary selection within `selections`.
    pub primary_idx: usize,
    /// Registered providers for this pane.
    pub providers: ProviderSet,
}

impl Pane {
    /// Create a new pane viewing `buffer_id` with default settings.
    ///
    /// Callers that need custom providers should use `Pane { providers, ..Pane::new(bid) }`.
    pub fn new(buffer_id: BufferId) -> Self {
        Self {
            buffer_id,
            viewport: ViewportState::new(80, 24),
            saved_scrolls: SecondaryMap::new(),
            selections: vec![Selection { anchor: 0, head: 0 }],
            primary_idx: 0,
            providers: ProviderSet::new(),
        }
    }

    /// Width available for text after subtracting the gutter, clamped to at least 1.
    ///
    /// `total_lines` is the buffer's current line count (used to size the line-number column).
    /// Call this before `WrapMode::resolve` to get the concrete wrap column.
    pub fn content_width(&self, total_lines: usize) -> u16 {
        let gutter_w = gutter_width_for_line(
            self.providers.gutter_columns(),
            total_lines.saturating_sub(1),
        );
        self.viewport.width.saturating_sub(gutter_w).max(1)
    }

    /// Snapshot the current viewport scroll into `saved_scrolls` for `buffer_id`.
    pub fn remember_scroll(&mut self) {
        self.saved_scrolls.insert(
            self.buffer_id,
            ScrollPosition {
                top_line: self.viewport.top_line,
                top_row_offset: self.viewport.top_row_offset,
                horizontal_offset: self.viewport.horizontal_offset,
            },
        );
    }

    /// Restore the saved scroll for `id`, or reset to top on first visit.
    pub fn recall_scroll(&mut self, id: BufferId) {
        let sp = self.saved_scrolls.get(id).copied().unwrap_or_default();
        self.viewport.top_line = sp.top_line;
        self.viewport.top_row_offset = sp.top_row_offset;
        self.viewport.horizontal_offset = sp.horizontal_offset;
    }

    /// Drop the saved scroll entry for `id` (called when the buffer is closed).
    pub fn forget_buffer(&mut self, id: BufferId) {
        self.saved_scrolls.remove(id);
    }

    /// Line index of the primary selection head, resolved via the rope.
    ///
    /// Called once per frame from the pipeline — O(log n) rope lookup.
    /// Panics in debug builds if the pane has no selections.
    pub fn primary_head_line(&self, rope: &Rope) -> usize {
        debug_assert!(!self.selections.is_empty(), "pane has no selections");
        let head_char = self
            .selections
            .get(self.primary_idx)
            .map(|s| s.head)
            .unwrap_or(0);
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
        assert_eq!(
            "soft:80".parse::<WrapMode>().unwrap(),
            WrapMode::Soft { width: 80 }
        );
        assert_eq!(
            "word:40".parse::<WrapMode>().unwrap(),
            WrapMode::Word { width: 40 }
        );
        assert_eq!(
            "indent:76".parse::<WrapMode>().unwrap(),
            WrapMode::Indent { width: 76 }
        );
    }

    #[test]
    fn wrap_mode_from_str_bare_keywords() {
        // Bare keyword (no colon) → sentinel width 0 (terminal width).
        assert_eq!("soft".parse::<WrapMode>().unwrap(),   WrapMode::Soft   { width: 0 });
        assert_eq!("word".parse::<WrapMode>().unwrap(),   WrapMode::Word   { width: 0 });
        assert_eq!("indent".parse::<WrapMode>().unwrap(), WrapMode::Indent { width: 0 });
    }

    #[test]
    fn wrap_mode_from_str_colon_zero_is_sentinel() {
        // `:0` is the same sentinel as bare keyword.
        assert_eq!("soft:0".parse::<WrapMode>().unwrap(), WrapMode::Soft { width: 0 });
    }

    #[test]
    fn wrap_mode_from_str_case_insensitive() {
        assert_eq!(
            "Soft:80".parse::<WrapMode>().unwrap(),
            WrapMode::Soft { width: 80 }
        );
        assert_eq!(
            "INDENT:76".parse::<WrapMode>().unwrap(),
            WrapMode::Indent { width: 76 }
        );
    }

    #[test]
    fn wrap_mode_from_str_error_unknown_kind() {
        assert!("hard:80".parse::<WrapMode>().is_err());
    }

    #[test]
    fn wrap_mode_from_str_error_non_numeric_width() {
        assert!("soft:abc".parse::<WrapMode>().is_err());
    }

    // ── WhitespaceRender::FromStr ─────────────────────────────────────────

    #[test]
    fn whitespace_render_from_str_all_variants() {
        assert_eq!(
            "none".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::None
        );
        assert_eq!(
            "all".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::All
        );
        assert_eq!(
            "trailing".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::Trailing
        );
    }

    #[test]
    fn whitespace_render_from_str_case_insensitive() {
        assert_eq!(
            "None".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::None
        );
        assert_eq!(
            "ALL".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::All
        );
        assert_eq!(
            "Trailing".parse::<WhitespaceRender>().unwrap(),
            WhitespaceRender::Trailing
        );
    }

    #[test]
    fn whitespace_render_from_str_error() {
        let err = "always".parse::<WhitespaceRender>().unwrap_err();
        assert!(err.contains("always"), "error should contain input: {err}");
    }

    #[test]
    fn wrap_mode_wrap_width() {
        assert_eq!(WrapMode::None.wrap_width(), None);
        assert_eq!(WrapMode::Soft   { width: 80 }.wrap_width(), Some(80));
        assert_eq!(WrapMode::Word   { width: 40 }.wrap_width(), Some(40));
        assert_eq!(WrapMode::Indent { width: 60 }.wrap_width(), Some(60));
    }

    #[test]
    fn wrap_mode_resolve() {
        // Sentinel → concrete.
        assert_eq!(WrapMode::Soft   { width: 0 }.resolve(80), WrapMode::Soft   { width: 80 });
        assert_eq!(WrapMode::Word   { width: 0 }.resolve(80), WrapMode::Word   { width: 80 });
        assert_eq!(WrapMode::Indent { width: 0 }.resolve(80), WrapMode::Indent { width: 80 });
        // Concrete and None pass through unchanged.
        assert_eq!(WrapMode::Soft   { width: 40 }.resolve(80), WrapMode::Soft   { width: 40 });
        assert_eq!(WrapMode::None.resolve(80), WrapMode::None);
    }

    #[test]
    fn wrap_mode_is_wrapping() {
        assert!(!WrapMode::None.is_wrapping());
        assert!(WrapMode::Soft   { width: 80 }.is_wrapping());
        assert!(WrapMode::Word   { width: 80 }.is_wrapping());
        assert!(WrapMode::Indent { width: 80 }.is_wrapping());
        // Sentinel (width: 0 = terminal width) must still report is_wrapping()
        // = true; it must not be conflated with WrapMode::None.
        assert!(WrapMode::Indent { width: 0 }.is_wrapping());
        assert!(WrapMode::Soft   { width: 0 }.is_wrapping());
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
            selections: vec![Selection {
                anchor: head_char,
                head: head_char,
            }],
            ..Pane::new(crate::pipeline::BufferId::default())
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
