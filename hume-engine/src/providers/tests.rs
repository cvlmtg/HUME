use super::*;

struct DummyHighlight {
    tier: HighlightTier,
}

impl DecorationSource for DummyHighlight {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }
    fn decorations_for_line(&self, _: hume_rope::line::ContentLine, out: &mut Vec<Decoration>) {
        out.push(Decoration::Highlight {
            byte_start: hume_rope::column::ByteCol::new(0),
            byte_end: hume_rope::column::ByteCol::new(1),
            scope: ScopeId(0),
            tier: self.tier,
        });
    }
}

/// A `VIRTUAL_LINE`-kind source, distinguishable from `DummyHighlight` by
/// declared kind. Used to prove `ProviderSet::decoration_sources` filters
/// by kind rather than returning every registered source.
struct DummyVirtualLine;

impl DecorationSource for DummyVirtualLine {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, _: hume_rope::line::ContentLine, _: &mut Vec<Decoration>) {}
}

struct DummyGutter;

impl GutterColumn for DummyGutter {
    fn width(&self, _: hume_rope::line::RopeyLine, _: crate::providers::GutterInputs) -> u8 {
        0
    }
    fn render_cells(&self, _: crate::types::DisplayLineKind, _: &GutterCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(ScopeId(0))]
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
