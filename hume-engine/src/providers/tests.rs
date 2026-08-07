use super::*;

struct DummyHighlight {
    tier: HighlightTier,
}

impl HighlightSource for DummyHighlight {
    fn tier(&self) -> HighlightTier {
        self.tier
    }
    fn highlights_for_line(
        &self,
        _: usize,
        _: &SourceContext,
        _: &mut Vec<(usize, usize, ScopeId)>,
    ) {
    }
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

/// Distinguishable from `DummyGutter` by width — used to prove `remove`
/// takes down the right provider and leaves the other untouched.
struct OtherGutter;

impl GutterColumn for OtherGutter {
    fn width(&self, _: usize) -> u8 {
        5
    }
    fn render_row_cells(&self, _: crate::types::RowKind, _: &GutterRowCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(ScopeId(1))]
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
    let id0 = set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::Syntax,
    }));
    let id1 = set.add_gutter_column(Box::new(DummyGutter));
    let id2 = set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::Diagnostic,
    }));
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
}

#[test]
fn provider_set_highlight_sorted_by_tier() {
    let mut set = ProviderSet::new();
    set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::BracketMatch,
    }));
    set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::Syntax,
    }));
    set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::Diagnostic,
    }));

    let tiers: Vec<_> = set.highlights.iter().map(|(_, h)| h.tier()).collect();
    assert_eq!(
        tiers,
        vec![
            HighlightTier::Syntax,
            HighlightTier::Diagnostic,
            HighlightTier::BracketMatch,
        ]
    );
}

// ── Provider unregistration (G3) ─────────────────────────────────────

#[test]
fn remove_by_id_takes_down_only_that_provider() {
    let mut set = ProviderSet::new();
    let id0 = set.add_gutter_column(Box::new(DummyGutter)); // width 0
    set.add_gutter_column(Box::new(OtherGutter)); // width 5

    assert!(set.remove(id0));

    let widths: Vec<u8> = set.gutter_columns().map(|c| c.width(0)).collect();
    assert_eq!(
        widths,
        vec![5],
        "only OtherGutter (width 5) remains; render order reflects it alone"
    );
}

#[test]
fn remove_unknown_id_is_a_no_op() {
    let mut set = ProviderSet::new();
    set.add_gutter_column(Box::new(DummyGutter));

    assert!(!set.remove(999), "unknown id must return false");
    assert_eq!(
        set.gutter_columns().count(),
        1,
        "removing an unknown id must not touch existing providers"
    );
}

#[test]
fn remove_across_provider_types_only_touches_the_matching_list() {
    // Ids are shared across all five lists' allocator — removing a
    // gutter-column id must not accidentally hit a highlight source
    // that happens to share the same numeric id space at a different
    // index.
    let mut set = ProviderSet::new();
    let highlight_id = set.add_highlight_source(Box::new(DummyHighlight {
        tier: HighlightTier::Syntax,
    }));
    let gutter_id = set.add_gutter_column(Box::new(DummyGutter));

    assert!(set.remove(gutter_id));
    assert_eq!(set.gutter_columns().count(), 0);
    assert_eq!(
        set.highlights.len(),
        1,
        "removing the gutter column must not touch the highlight source"
    );
    let _ = highlight_id;
}
