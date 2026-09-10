use super::*;
use crate::providers::DecorationSource;
use crate::theme::ScopeRegistry;
use crate::types::ScopeId;
use hume_rope::column::ByteCol;

fn bc(n: usize) -> ByteCol {
    ByteCol::new(n)
}

fn make_scope_ids(names: &[&'static str]) -> (ScopeRegistry, Vec<ScopeId>) {
    let mut reg = ScopeRegistry::new();
    let ids = names.iter().map(|&n| reg.intern(n)).collect();
    (reg, ids)
}

#[test]
fn interval_cursor_basic() {
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(bc(2), bc(5), kw), (bc(7), bc(9), fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(bc(0)), None);
    assert_eq!(cursor.scope_at(bc(2)), Some(kw));
    assert_eq!(cursor.scope_at(bc(4)), Some(kw));
    assert_eq!(cursor.scope_at(bc(5)), None);
    assert_eq!(cursor.scope_at(bc(7)), Some(fn_));
    assert_eq!(cursor.scope_at(bc(9)), None);
}

#[test]
fn interval_cursor_empty() {
    let mut cursor = IntervalCursor::<'_>::new(&[]);
    assert_eq!(cursor.scope_at(bc(0)), None);
    assert_eq!(cursor.scope_at(bc(100)), None);
}

/// Emits spans at two different tiers — proves tier is data on
/// `Decoration::Highlight`, not a per-provider property.
struct TwoTierSource(ScopeId);

impl DecorationSource for TwoTierSource {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }
    fn decorations_for_line(
        &self,
        _line_idx: hume_rope::line::ContentLine,
        out: &mut Vec<Decoration>,
    ) {
        out.push(Decoration::Highlight {
            byte_start: bc(0),
            byte_end: bc(1),
            scope: self.0,
            tier: HighlightTier::Syntax,
        });
        out.push(Decoration::Highlight {
            byte_start: bc(2),
            byte_end: bc(3),
            scope: self.0,
            tier: HighlightTier::Diagnostic,
        });
    }
}

#[test]
fn rebuild_line_decorations_buckets_one_sources_spans_by_tier() {
    let (_reg, ids) = make_scope_ids(&["kw"]);
    let scope = ids[0];

    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(TwoTierSource(scope)));
    let rope = ropey::Rope::from_str("abcdef\n");
    let mut scratch = StyleScratch::new();

    rebuild_line_decorations(
        hume_rope::line::ContentLine::new(0),
        None,
        &providers,
        &rope,
        &mut scratch,
    );

    assert_eq!(
        scratch.tier_bufs.0[HighlightTier::Syntax as usize],
        vec![(bc(0), bc(1), scope)],
        "the Syntax-tier span lands in the Syntax bucket"
    );
    assert_eq!(
        scratch.tier_bufs.0[HighlightTier::Diagnostic as usize],
        vec![(bc(2), bc(3), scope)],
        "the Diagnostic-tier span from the same source lands in a different bucket"
    );
}

#[test]
fn interval_cursor_adjacent_intervals() {
    // (2,5) and (5,8) are adjacent — byte 5 must match the second.
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(bc(2), bc(5), kw), (bc(5), bc(8), fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(bc(4)), Some(kw));
    assert_eq!(cursor.scope_at(bc(5)), Some(fn_));
    assert_eq!(cursor.scope_at(bc(7)), Some(fn_));
    assert_eq!(cursor.scope_at(bc(8)), None);
}
