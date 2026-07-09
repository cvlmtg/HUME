use std::any::Any;
use std::borrow::Cow;

use crate::providers::{GutterCell, GutterCellContent, GutterColumn, GutterRowCtx, ProviderId};
use crate::types::{RowKind, ScopeId};

/// Default configured width of a `SignColumn`: one sign cell plus one column
/// of right-padding, matching every other gutter column's separator
/// convention (see `render::compose_gutter`).
const DEFAULT_WIDTH: u8 = 2;

/// One sign a `SignSource` renders for a buffer line (diagnostic marker, git
/// change indicator, breakpoint, bookmark, ...).
#[derive(Clone, Debug)]
pub struct Sign {
    /// 1–2 cells wide; wider is truncated to the column width by
    /// `render::compose_gutter`, same as any other gutter cell text.
    pub text: Cow<'static, str>,
    /// Already-interned — same contract as `HighlightSource`/`InlineInsert`:
    /// intern at provider-construction time, before the first render.
    pub scope: ScopeId,
    /// Higher wins when multiple sources fire for the same line. Ties
    /// resolve to the later-registered source (registration order in
    /// `SignColumn::sources`).
    pub priority: i16,
}

/// A source of signs for buffer lines (diagnostics, git status, breakpoints,
/// bookmarks, ...). Multiple sources can share one `SignColumn`, which keeps
/// only the highest-priority `Sign` per line — this is what lets several
/// features merge into one narrow gutter column instead of each burning a
/// column of width.
pub trait SignSource {
    /// Sign for one buffer line, or `None`. Called per `LineStart` row per
    /// frame — implementations should be cheap lookups into their own state
    /// (same contract as `VirtualLineSource`).
    fn sign_for_line(&self, line_idx: usize, ctx: &GutterRowCtx) -> Option<Sign>;
}

/// Built-in gutter column that merges signs from multiple `SignSource`s,
/// keeping only the highest-priority one per line.
///
/// Registered like any other `GutterColumn` via
/// `ProviderSet::add_gutter_column`, which returns the column's own
/// `ProviderId`. Sign sources are added separately, after registration, by
/// finding the column and downcasting to `SignColumn` — the same pattern
/// `ProviderSet::sync_line_number_style` already uses to reach a
/// `LineNumberColumn` post-registration (`as_any_mut().downcast_mut::<SignColumn>()`).
pub struct SignColumn {
    sources: Vec<(ProviderId, Box<dyn SignSource>)>,
    width: u8,
    next_id: ProviderId,
}

impl Default for SignColumn {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            width: DEFAULT_WIDTH,
            next_id: 0,
        }
    }
}

impl SignColumn {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_width(width: u8) -> Self {
        Self {
            width,
            ..Self::default()
        }
    }

    /// Register a sign source. Mints its own `ProviderId` — `SignColumn` is
    /// self-contained rather than reusing the pane's `ProviderSet` allocator,
    /// since a sign source isn't one of `ProviderSet`'s five provider kinds.
    pub fn add_source(&mut self, source: Box<dyn SignSource>) -> ProviderId {
        let id = self.next_id;
        debug_assert!(self.next_id < ProviderId::MAX, "SignColumn id overflow");
        self.next_id += 1;
        self.sources.push((id, source));
        id
    }

    /// Remove the sign source registered under `id`. Returns `true` if one
    /// was removed.
    pub fn remove_source(&mut self, id: ProviderId) -> bool {
        let before = self.sources.len();
        self.sources.retain(|(pid, _)| *pid != id);
        before != self.sources.len()
    }

    /// Set the configured column width. Unlike the sources themselves, width
    /// is not derived from them automatically — a caller wanting the column
    /// to collapse to `0` when no source would fire for the current buffer
    /// (or grow back to the default when one would) sets it explicitly, per
    /// frame, via the same post-registration downcast `sync_line_number_style`
    /// uses. Two calls with the same width are a cheap no-op either way.
    pub fn set_width(&mut self, width: u8) {
        self.width = width;
    }
}

impl GutterColumn for SignColumn {
    fn width(&self, _last_line_idx: usize) -> u8 {
        // Configured, not per-frame-recomputed: signs must not resize the
        // gutter frame to frame (unlike LineNumberColumn's whole-file-max
        // rule, which legitimately grows with the file).
        self.width
    }

    fn render_row(&self, kind: RowKind, ctx: &GutterRowCtx) -> GutterCell {
        let RowKind::LineStart { line_idx } = kind else {
            // Wrap/Virtual/Filler rows never carry a sign.
            return GutterCell::blank(crate::types::Scope("ui.linenr"));
        };
        let winner = self
            .sources
            .iter()
            .filter_map(|(_, src)| src.sign_for_line(line_idx, ctx))
            // max_by_key keeps the *last* maximum on ties, i.e. the
            // later-registered source wins — matches the documented
            // tie-break rule.
            .max_by_key(|sign| sign.priority);
        match winner {
            Some(sign) => GutterCell {
                content: GutterCellContent::Text(sign.text),
                scope: sign.scope.into(),
            },
            None => GutterCell::blank(crate::types::Scope("ui.linenr")),
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ScopeRegistry, Theme};
    use crate::types::EditorMode;

    fn ctx(rope: &ropey::Rope) -> GutterRowCtx<'_> {
        GutterRowCtx {
            mode: EditorMode::Normal,
            primary_head_line: 0,
            rope,
            tree: None,
        }
    }

    struct FixedSign {
        line: usize,
        sign: Sign,
    }

    impl SignSource for FixedSign {
        fn sign_for_line(&self, line_idx: usize, _ctx: &GutterRowCtx) -> Option<Sign> {
            (line_idx == self.line).then(|| self.sign.clone())
        }
    }

    #[test]
    fn higher_priority_sign_wins_on_the_same_line() {
        let mut registry = ScopeRegistry::new();
        let diag_scope = registry.intern("diagnostic");
        let git_scope = registry.intern("git");

        let mut col = SignColumn::new();
        col.add_source(Box::new(FixedSign {
            line: 3,
            sign: Sign {
                text: "!".into(),
                scope: diag_scope,
                priority: 10,
            },
        }));
        col.add_source(Box::new(FixedSign {
            line: 3,
            sign: Sign {
                text: "+".into(),
                scope: git_scope,
                priority: 5,
            },
        }));

        let rope = ropey::Rope::new();
        let cell = col.render_row(RowKind::LineStart { line_idx: 3 }, &ctx(&rope));
        assert_eq!(cell.as_str(), "!", "priority 10 beats priority 5");
        assert_eq!(cell.scope, crate::providers::GutterScope::Id(diag_scope));
    }

    #[test]
    fn removing_the_winner_reveals_the_next_highest() {
        let mut registry = ScopeRegistry::new();
        let diag_scope = registry.intern("diagnostic");
        let git_scope = registry.intern("git");

        let mut col = SignColumn::new();
        let winner_id = col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "!".into(),
                scope: diag_scope,
                priority: 10,
            },
        }));
        col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "+".into(),
                scope: git_scope,
                priority: 5,
            },
        }));

        assert!(col.remove_source(winner_id));

        let rope = ropey::Rope::new();
        let cell = col.render_row(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
        assert_eq!(
            cell.as_str(),
            "+",
            "with the priority-10 sign gone, 5 shows"
        );
    }

    #[test]
    fn no_source_fires_renders_blank() {
        let col = SignColumn::new();
        let rope = ropey::Rope::new();
        let cell = col.render_row(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
        assert_eq!(cell.as_str(), " ");
    }

    #[test]
    fn sign_absent_on_wrap_virtual_and_filler_rows() {
        let mut registry = ScopeRegistry::new();
        let scope = registry.intern("diagnostic");
        let mut col = SignColumn::new();
        col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "!".into(),
                scope,
                priority: 1,
            },
        }));
        let rope = ropey::Rope::new();

        for kind in [
            RowKind::Wrap {
                line_idx: 0,
                wrap_row: 1,
            },
            RowKind::Virtual {
                provider_id: 0,
                anchor_line: 0,
            },
            RowKind::Filler,
        ] {
            let cell = col.render_row(kind, &ctx(&rope));
            assert_eq!(cell.as_str(), " ", "{kind:?} must not show a sign");
        }
    }

    #[test]
    fn width_is_configured_not_recomputed_per_frame() {
        let col = SignColumn::with_width(3);
        assert_eq!(col.width(0), 3);
        assert_eq!(col.width(999_999), 3, "stable regardless of file size");
    }

    #[test]
    fn set_width_overrides_the_configured_width() {
        let mut col = SignColumn::with_width(2);
        col.set_width(0);
        assert_eq!(col.width(0), 0, "collapsed to zero when no signs exist");
        col.set_width(2);
        assert_eq!(col.width(0), 2, "restored once a sign exists again");
    }

    #[test]
    fn sign_text_truncates_to_column_width_end_to_end() {
        // Full compose path (not just SignColumn::render_row in isolation):
        // a 3-glyph sign in a width-2 column must come out clipped by
        // `render::compose_gutter` (B8's gutter clipping), same as every
        // other gutter column — SignColumn adds no truncation of its own.
        let mut registry = ScopeRegistry::new();
        let scope = registry.intern("diagnostic");
        let mut col = SignColumn::with_width(2); // usable = 1 cell
        col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "▶▶▶".into(),
                scope,
                priority: 1,
            },
        }));

        let graphemes = vec![crate::types::Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        }];
        let rows = [crate::types::DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes: 0..1,
        }];
        let styles = vec![crate::types::ResolvedStyle::default()];
        let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![(0, Box::new(col))];
        let visible = crate::layout::VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 6,
            gutter_width: 2,
            last_line_idx: 0,
        };
        let viewport = crate::pane::ViewportState::new(8, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 1,
        };
        let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
        let mut theme = Theme::default();
        theme.bake(&registry);
        let rope = ropey::Rope::from_str("x\n");
        let col_widths = vec![2u16];
        let compose_ctx = crate::render::ComposeCtx {
            gutter_columns: &gutter_columns,
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
            rope: &rope,
            tree: None,
        };
        let mut canvas = crate::render::PaneCanvas::new(&mut buf, None);
        crate::render::compose_row(
            &rows[0],
            &graphemes,
            &styles,
            "x",
            "",
            0,
            &col_widths,
            &compose_ctx,
            &mut canvas,
            None,
        );

        let sym = |x: u16| {
            buf.cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol()
                .to_string()
        };
        assert_eq!(sym(0), "▶", "only the first glyph of the sign fits");
        assert_eq!(sym(1), " ", "separator cell, not a straggler glyph");
        // Content area starts at x=2 (gutter_width) — must show the real
        // grapheme 'x', never a spillover from the sign text.
        assert_eq!(sym(2), "x");
    }

    /// A width-0 `SignColumn` (the auto-collapse state `set_width(0)`
    /// produces when no sign exists for the pane's buffer) must render as if
    /// it weren't registered at all — the gutter composer still iterates it,
    /// but must not shift or corrupt whatever renders in the next column.
    #[test]
    fn zero_width_sign_column_leaves_the_next_column_untouched() {
        let mut registry = ScopeRegistry::new();
        let scope = registry.intern("ui.linenr");
        let empty_col = SignColumn::with_width(0); // no sources — width collapsed
        let mut content_col = SignColumn::with_width(2);
        content_col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "!".into(),
                scope,
                priority: 1,
            },
        }));

        let graphemes = vec![crate::types::Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            col: 0,
            width: 1,
            content: crate::types::CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        }];
        let rows = [crate::types::DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes: 0..1,
        }];
        let styles = vec![crate::types::ResolvedStyle::default()];
        let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> =
            vec![(0, Box::new(empty_col)), (1, Box::new(content_col))];
        let visible = crate::layout::VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 6,
            gutter_width: 2, // 0 (empty_col) + 2 (content_col)
            last_line_idx: 0,
        };
        let viewport = crate::pane::ViewportState::new(8, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 1,
        };
        let mut buf = ratatui::buffer::Buffer::empty(pane_rect);
        let mut theme = Theme::default();
        theme.bake(&registry);
        let rope = ropey::Rope::from_str("x\n");
        let col_widths = vec![0u16, 2u16];
        let compose_ctx = crate::render::ComposeCtx {
            gutter_columns: &gutter_columns,
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
            rope: &rope,
            tree: None,
        };
        let mut canvas = crate::render::PaneCanvas::new(&mut buf, None);
        crate::render::compose_row(
            &rows[0],
            &graphemes,
            &styles,
            "x",
            "",
            0,
            &col_widths,
            &compose_ctx,
            &mut canvas,
            None,
        );

        let sym = |x: u16| {
            buf.cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol()
                .to_string()
        };
        assert_eq!(
            sym(0),
            "!",
            "the width-2 column's sign starts right at x=0, unaffected by the width-0 column ahead of it"
        );
        assert_eq!(sym(1), " ", "separator cell");
        assert_eq!(sym(2), "x", "content starts exactly at gutter_width=2");
    }

    #[test]
    fn sign_scope_resolves_via_baked_theme() {
        let mut registry = ScopeRegistry::new();
        let scope_id = registry.intern("diagnostic.error");
        let mut styles_map = std::collections::HashMap::new();
        styles_map.insert(
            "diagnostic.error",
            crate::types::ResolvedStyle {
                fg: Some(ratatui::style::Color::Red),
                ..Default::default()
            },
        );
        let mut theme = Theme::new(styles_map, crate::types::ResolvedStyle::default());
        theme.bake(&registry);

        let mut col = SignColumn::new();
        col.add_source(Box::new(FixedSign {
            line: 0,
            sign: Sign {
                text: "!".into(),
                scope: scope_id,
                priority: 1,
            },
        }));
        let rope = ropey::Rope::new();
        let cell = col.render_row(RowKind::LineStart { line_idx: 0 }, &ctx(&rope));
        let crate::providers::GutterScope::Id(id) = cell.scope else {
            panic!("expected an interned ScopeId, got {:?}", cell.scope);
        };
        assert_eq!(theme.resolve(id).fg, Some(ratatui::style::Color::Red));
    }
}
