use std::ops::Range;

use bitflags::bitflags;

// ---------------------------------------------------------------------------
// Theme & Style
// ---------------------------------------------------------------------------

/// A semantic scope name emitted by providers. All style decisions go through
/// the Theme — providers never emit raw colors.
///
/// Built-in scopes use `&'static str`. The scope format follows dot-notation
/// with automatic fallback: `keyword.function` → `keyword` → default.
///
/// Use `Scope` at construction time (theme maps, scope_map slices, gutter
/// cells). Use [`ScopeId`] on the hot path — it is an O(1) Vec index into the
/// theme's baked style array, with no hashing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scope(pub &'static str);

/// An interned scope identifier produced by [`crate::theme::ScopeRegistry`].
///
/// Resolved once at provider-construction time; used on the per-grapheme hot
/// path to look up [`ResolvedStyle`] from [`crate::theme::Theme`] in O(1) via
/// a direct `Vec` index.
///
/// The mapping is stable within a session but not persistent — do not store
/// `ScopeId` values across sessions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub u16);

/// The engine's internal style representation. Richer than ratatui's — the
/// Render stage maps this to `ratatui::Style` as the final step.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedStyle {
    pub fg: Option<ratatui::style::Color>,
    pub bg: Option<ratatui::style::Color>,
    pub underline: UnderlineStyle,
    pub underline_color: Option<ratatui::style::Color>,
    pub modifiers: Modifiers,
}

impl ResolvedStyle {
    /// Layer `over` on top of `self`. Non-None / non-default fields in `over` win.
    /// This is the primitive compositing operation for the style cascade.
    pub fn layer(self, over: ResolvedStyle) -> ResolvedStyle {
        ResolvedStyle {
            fg: over.fg.or(self.fg),
            bg: over.bg.or(self.bg),
            underline: if over.underline != UnderlineStyle::None {
                over.underline
            } else {
                self.underline
            },
            underline_color: over.underline_color.or(self.underline_color),
            modifiers: self.modifiers | over.modifiers,
        }
    }
}

/// Underline variants supported by modern terminals.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Solid,
    Wavy,
    Dotted,
    Dashed,
}

bitflags! {
    /// Text modifiers that compose independently (bold, italic, strikethrough).
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct Modifiers: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const STRIKETHROUGH = 0b0000_0100;
    }
}

impl From<ResolvedStyle> for ratatui::style::Style {
    fn from(s: ResolvedStyle) -> Self {
        let mut style = ratatui::style::Style::default();
        if let Some(fg) = s.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = s.bg {
            style = style.bg(bg);
        }
        if s.modifiers.contains(Modifiers::BOLD) {
            style = style.add_modifier(ratatui::style::Modifier::BOLD);
        }
        if s.modifiers.contains(Modifiers::ITALIC) {
            style = style.add_modifier(ratatui::style::Modifier::ITALIC);
        }
        if s.modifiers.contains(Modifiers::STRIKETHROUGH) {
            style = style.add_modifier(ratatui::style::Modifier::CROSSED_OUT);
        }
        // Underline styles: ratatui supports UNDERLINED modifier + underline_color.
        // Wavy/dotted/dashed require terminal support and may not map 1:1.
        match s.underline {
            UnderlineStyle::None => {}
            _ => {
                style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                if let Some(uc) = s.underline_color {
                    style = style.underline_color(uc);
                }
            }
        }
        style
    }
}

// ---------------------------------------------------------------------------
// Grapheme — the atom of the formatter
// ---------------------------------------------------------------------------

/// One grapheme cluster laid out by the Format stage.
/// This is the unit that flows through Style into Render.
#[derive(Clone, Debug)]
pub struct Grapheme {
    /// Byte range within the materialized line buffer (empty for virtual content).
    ///
    /// Used by the highlight system (tree-sitter intervals are byte-native) and
    /// by the wrap-segment intersection check in the style stage.
    pub byte_range: Range<usize>,
    /// Absolute char offset from the start of the buffer.
    ///
    /// Populated by the format stage so the style stage can resolve selection
    /// head positions without any rope lookups. `usize::MAX` for purely virtual
    /// graphemes (inline inserts, newline indicators) that have no buffer char.
    pub char_offset: usize,
    /// Display column within the row (0-based, accounts for preceding widths).
    pub col: u16,
    /// Display width: 1 for ASCII/most Unicode, 2 for CJK, >1 for tabs.
    pub width: u8,
    /// What to render.
    pub content: CellContent,
    /// Indent depth at this column — used for indent guide rendering.
    pub indent_depth: u8,
}

/// What a grapheme cell displays.
#[derive(Copy, Clone, Debug)]
pub enum CellContent {
    /// A real grapheme cluster. The text is read from the rope via `byte_range`.
    /// Avoids copying grapheme strings during formatting.
    Grapheme,
    /// A substitution: whitespace indicator, tab fill character.
    Indicator(&'static str),
    /// The right-hand padding cell of a double-width character.
    WidthContinuation,
    /// Empty: tilde filler past EOF, or padding past end of line.
    Empty,
    /// An inline virtual decoration (inlay hint, ghost text).
    /// The string is owned by the provider and lives for the frame.
    Virtual(&'static str),
}

// ---------------------------------------------------------------------------
// Display Row
// ---------------------------------------------------------------------------

/// One horizontal row in the content area.
/// A single buffer line may produce multiple DisplayRows when wrapping.
#[derive(Clone, Debug)]
pub struct DisplayRow {
    /// What kind of content this row represents.
    pub kind: RowKind,
    /// Index range into the frame's `FrameScratch::graphemes` buffer.
    pub graphemes: Range<usize>,
}

/// Classifies a display row's origin.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// The first display row of a buffer line.
    LineStart { line_idx: usize },
    /// A continuation row produced by wrapping.
    Wrap { line_idx: usize, wrap_row: u16 },
    /// A virtual row injected by a provider (no buffer line).
    Virtual {
        provider_id: u16,
        anchor_line: usize,
    },
    /// A tilde filler row past end of buffer.
    Filler,
}

impl RowKind {
    /// Returns the buffer line index if this row corresponds to a real line.
    pub fn line_idx(self) -> Option<usize> {
        match self {
            RowKind::LineStart { line_idx } | RowKind::Wrap { line_idx, .. } => Some(line_idx),
            RowKind::Virtual { .. } | RowKind::Filler => None,
        }
    }

    pub fn is_wrapped_continuation(self) -> bool {
        matches!(self, RowKind::Wrap { .. })
    }

    pub fn is_virtual(self) -> bool {
        matches!(self, RowKind::Virtual { .. })
    }
}

// ---------------------------------------------------------------------------
// Selections & Cursor
// ---------------------------------------------------------------------------

/// An editor selection: an anchor and a head, both as absolute char offsets
/// from the start of the buffer.
///
/// Anchor == head is a single-character selection covering the char at that
/// index (the editor's inclusive selection invariant). The selection spans
/// [min(anchor, head), max(anchor, head)] inclusive.
///
/// Using char offsets avoids per-frame rope lookups at the editor→engine
/// boundary: the editor simply copies its char-offset selections directly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// Returns the selection range as (start, end) with start <= end.
    pub fn range(self) -> (usize, usize) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// True if this selection is collapsed (anchor == head, no range).
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.head
    }
}

/// Editor mode — determines cursor shape and highlight behavior.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum EditorMode {
    #[default]
    Normal,
    Insert,
    Select,
    Extend,
    Command,
    Search,
}

impl EditorMode {
    /// Whether the cursor should render as a bar (Insert/Command/Search/Select)
    /// or a block (Normal/Extend).
    pub fn cursor_is_bar(self) -> bool {
        matches!(
            self,
            EditorMode::Insert | EditorMode::Command | EditorMode::Search | EditorMode::Select
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_style_layer_fg_wins() {
        let base = ResolvedStyle {
            fg: Some(ratatui::style::Color::Red),
            ..Default::default()
        };
        let over = ResolvedStyle {
            fg: Some(ratatui::style::Color::Blue),
            ..Default::default()
        };
        assert_eq!(base.layer(over).fg, Some(ratatui::style::Color::Blue));
    }

    #[test]
    fn resolved_style_layer_preserves_base_when_over_is_none() {
        let base = ResolvedStyle {
            fg: Some(ratatui::style::Color::Red),
            ..Default::default()
        };
        let over = ResolvedStyle::default();
        assert_eq!(base.layer(over).fg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn resolved_style_layer_underline_none_preserves_base() {
        let base = ResolvedStyle {
            underline: UnderlineStyle::Wavy,
            ..Default::default()
        };
        let over = ResolvedStyle::default(); // underline = None
        assert_eq!(base.layer(over).underline, UnderlineStyle::Wavy);
    }

    #[test]
    fn resolved_style_layer_underline_over_wins() {
        let base = ResolvedStyle {
            underline: UnderlineStyle::Wavy,
            ..Default::default()
        };
        let over = ResolvedStyle {
            underline: UnderlineStyle::Solid,
            ..Default::default()
        };
        assert_eq!(base.layer(over).underline, UnderlineStyle::Solid);
    }

    #[test]
    fn resolved_style_layer_modifiers_empty_preserves_base() {
        let base = ResolvedStyle {
            modifiers: Modifiers::BOLD,
            ..Default::default()
        };
        let over = ResolvedStyle::default();
        assert_eq!(base.layer(over).modifiers, Modifiers::BOLD);
    }

    #[test]
    fn selection_range_ordered() {
        let sel = Selection {
            anchor: 42,
            head: 7,
        };
        let (start, end) = sel.range();
        assert!(start <= end);
        assert_eq!(start, 7);
        assert_eq!(end, 42);
    }

    #[test]
    fn row_kind_line_idx() {
        assert_eq!(RowKind::LineStart { line_idx: 7 }.line_idx(), Some(7));
        assert_eq!(
            RowKind::Wrap {
                line_idx: 7,
                wrap_row: 1
            }
            .line_idx(),
            Some(7)
        );
        assert_eq!(
            RowKind::Virtual {
                provider_id: 0,
                anchor_line: 7
            }
            .line_idx(),
            None
        );
        assert_eq!(RowKind::Filler.line_idx(), None);
    }

    #[test]
    fn resolved_style_layer_bg() {
        let base = ResolvedStyle {
            bg: Some(ratatui::style::Color::Red),
            ..Default::default()
        };
        let over = ResolvedStyle {
            bg: Some(ratatui::style::Color::Blue),
            ..Default::default()
        };
        assert_eq!(base.layer(over).bg, Some(ratatui::style::Color::Blue));
        // None over preserves base bg.
        assert_eq!(
            base.layer(ResolvedStyle::default()).bg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[test]
    fn resolved_style_layer_underline_color() {
        let base = ResolvedStyle {
            underline_color: Some(ratatui::style::Color::Green),
            ..Default::default()
        };
        let over = ResolvedStyle {
            underline_color: Some(ratatui::style::Color::Red),
            ..Default::default()
        };
        assert_eq!(
            base.layer(over).underline_color,
            Some(ratatui::style::Color::Red)
        );
        assert_eq!(
            base.layer(ResolvedStyle::default()).underline_color,
            Some(ratatui::style::Color::Green)
        );
    }

    #[test]
    fn resolved_style_layer_modifiers_union() {
        let base = ResolvedStyle {
            modifiers: Modifiers::BOLD,
            ..Default::default()
        };
        let over = ResolvedStyle {
            modifiers: Modifiers::ITALIC,
            ..Default::default()
        };
        assert_eq!(
            base.layer(over).modifiers,
            Modifiers::BOLD | Modifiers::ITALIC
        );
    }

    #[test]
    fn resolved_style_to_ratatui_style() {
        let s = ResolvedStyle {
            fg: Some(ratatui::style::Color::Red),
            bg: Some(ratatui::style::Color::Blue),
            modifiers: Modifiers::BOLD | Modifiers::ITALIC | Modifiers::STRIKETHROUGH,
            underline: UnderlineStyle::Solid,
            underline_color: Some(ratatui::style::Color::Green),
        };
        let r: ratatui::style::Style = s.into();
        assert_eq!(r.fg, Some(ratatui::style::Color::Red));
        assert_eq!(r.bg, Some(ratatui::style::Color::Blue));
        assert!(r.add_modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(r.add_modifier.contains(ratatui::style::Modifier::ITALIC));
        assert!(
            r.add_modifier
                .contains(ratatui::style::Modifier::CROSSED_OUT)
        );
        assert!(
            r.add_modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }

    #[test]
    fn selection_range_anchor_equals_head() {
        let sel = Selection { anchor: 5, head: 5 };
        let (start, end) = sel.range();
        assert_eq!(start, 5);
        assert_eq!(end, 5);
    }

    #[test]
    fn selection_is_collapsed() {
        assert!(Selection { anchor: 0, head: 0 }.is_collapsed());
        assert!(!Selection { anchor: 0, head: 1 }.is_collapsed());
    }

    #[test]
    fn editor_mode_cursor_is_bar() {
        assert!(!EditorMode::Normal.cursor_is_bar());
        assert!(!EditorMode::Extend.cursor_is_bar());
        assert!(EditorMode::Insert.cursor_is_bar());
        assert!(EditorMode::Command.cursor_is_bar());
        assert!(EditorMode::Search.cursor_is_bar());
        assert!(EditorMode::Select.cursor_is_bar());
    }

    #[test]
    fn row_kind_predicates() {
        assert!(
            RowKind::Wrap {
                line_idx: 0,
                wrap_row: 1
            }
            .is_wrapped_continuation()
        );
        assert!(!RowKind::LineStart { line_idx: 0 }.is_wrapped_continuation());
        assert!(
            RowKind::Virtual {
                provider_id: 0,
                anchor_line: 0
            }
            .is_virtual()
        );
        assert!(!RowKind::Filler.is_virtual());
    }
}
