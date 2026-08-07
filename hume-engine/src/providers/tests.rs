use super::*;

struct DummyHighlight {
    tier: HighlightTier,
}

impl DecorationSource for DummyHighlight {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }
    fn decorations_for_line(&self, _: usize, out: &mut Vec<Decoration>) {
        out.push(Decoration::Highlight {
            byte_start: 0,
            byte_end: 1,
            scope: ScopeId(0),
            tier: self.tier,
        });
    }
}

/// A `VIRTUAL_LINE`-kind source, distinguishable from `DummyHighlight` by
/// declared kind — used to prove `ProviderSet::decoration_sources` filters
/// by kind rather than returning every registered source.
struct DummyVirtualLine;

impl DecorationSource for DummyVirtualLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, _: usize, _: &mut Vec<Decoration>) {}
}

struct DummyGutter;

impl GutterColumn for DummyGutter {
    fn width(&self, _: usize) -> u8 {
        0
    }
    fn render_row_cells(&self, _: crate::types::RowKind, _: &GutterRowCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(ScopeId(0))]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── GutterCellContent::from_number ─────────────────────────────────

fn num_str(n: usize) -> String {
    GutterCell {
        content: GutterCellContent::from_number(n),
        scope: ScopeId(0),
    }
    .as_str()
    .to_owned()
}

#[test]
fn from_number_zero() {
    assert_eq!(num_str(0), "0");
}

#[test]
fn from_number_small() {
    assert_eq!(num_str(1), "1");
    assert_eq!(num_str(42), "42");
    assert_eq!(num_str(999), "999");
}

#[test]
fn from_number_large() {
    assert_eq!(num_str(9_999_999), "9999999");
    assert_eq!(num_str(10_000_000), "10000000");
}

#[test]
fn gutter_cell_text_and_blank() {
    let s = GutterCell {
        content: GutterCellContent::Text(Cow::Borrowed("abc")),
        scope: ScopeId(0),
    };
    assert_eq!(s.as_str(), "abc");
    let b = GutterCell::blank(ScopeId(0));
    assert_eq!(b.as_str(), " ");
}

// ── sync_line_number_style ───────────────────────────────────────────

#[test]
fn sync_line_number_style_updates_line_number_column() {
    use crate::builtins::line_number::{LineNumberColumn, LineNumberStyle};
    let mut set = ProviderSet::new();
    set.add_gutter_column(Box::new(LineNumberColumn::with_style(
        LineNumberStyle::Hybrid,
        ScopeId(0),
        ScopeId(1),
    )));
    set.sync_line_number_style(LineNumberStyle::Relative);
    let col = set.gutter_columns[0]
        .1
        .as_any_mut()
        .downcast_mut::<LineNumberColumn>()
        .unwrap();
    assert_eq!(col.style, LineNumberStyle::Relative);
}

#[test]
fn sync_line_number_style_skips_non_line_number_columns() {
    use crate::builtins::line_number::LineNumberStyle;
    let mut set = ProviderSet::new();
    set.add_gutter_column(Box::new(DummyGutter));
    // Should not panic — DummyGutter doesn't downcast to LineNumberColumn.
    set.sync_line_number_style(LineNumberStyle::Absolute);
}

#[test]
fn sync_line_number_style_no_op_when_empty() {
    use crate::builtins::line_number::LineNumberStyle;
    let mut set = ProviderSet::new();
    set.sync_line_number_style(LineNumberStyle::Hybrid);
}

#[test]
fn sync_sign_column_width_updates_registered_sign_columns() {
    let mut set = ProviderSet::new();
    set.add_gutter_column(Box::new(SignColumn::new(ScopeId(0))));
    set.sync_sign_column_width(0);
    let col = set.gutter_columns[0]
        .1
        .as_any_mut()
        .downcast_mut::<SignColumn>()
        .unwrap();
    assert_eq!(col.width(0), 0);
}

#[test]
fn sync_sign_column_width_skips_non_sign_columns() {
    let mut set = ProviderSet::new();
    set.add_gutter_column(Box::new(DummyGutter));
    // Should not panic — DummyGutter doesn't downcast to SignColumn.
    set.sync_sign_column_width(0);
}

// ── ProviderSet ──────────────────────────────────────────────────────

#[test]
fn provider_set_ids_are_sequential_and_unique_across_types() {
    let mut set = ProviderSet::new();
    let id0 = set.add_decoration_source(Box::new(DummyHighlight {
        tier: HighlightTier::Syntax,
    }));
    let id1 = set.add_gutter_column(Box::new(DummyGutter));
    let id2 = set.add_decoration_source(Box::new(DummyHighlight {
        tier: HighlightTier::Diagnostic,
    }));
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
}

#[test]
fn decoration_sources_filters_by_kind() {
    let mut set = ProviderSet::new();
    set.add_decoration_source(Box::new(DummyHighlight {
        tier: HighlightTier::Syntax,
    }));
    set.add_decoration_source(Box::new(DummyVirtualLine));

    let highlight_kinds: Vec<_> = set
        .decoration_sources(DecorationKinds::HIGHLIGHT)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        highlight_kinds,
        vec![0],
        "only the HIGHLIGHT-kind source (id 0) matches"
    );

    let virtual_line_kinds: Vec<_> = set
        .decoration_sources(DecorationKinds::VIRTUAL_LINE)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        virtual_line_kinds,
        vec![1],
        "only the VIRTUAL_LINE-kind source (id 1) matches"
    );

    let paint_kinds: Vec<_> = set
        .decoration_sources(DecorationKinds::PAINT)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        paint_kinds,
        vec![0],
        "PAINT includes HIGHLIGHT but not VIRTUAL_LINE"
    );
}
