use super::*;
use crate::providers::DecorationSource;
use crate::theme::ScopeRegistry;
use crate::types::ScopeId;

fn make_scope_ids(names: &[&'static str]) -> (ScopeRegistry, Vec<ScopeId>) {
    let mut reg = ScopeRegistry::new();
    let ids = names.iter().map(|&n| reg.intern(n)).collect();
    (reg, ids)
}

#[test]
fn interval_cursor_basic() {
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(2, 5, kw), (7, 9, fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(0), None);
    assert_eq!(cursor.scope_at(2), Some(kw));
    assert_eq!(cursor.scope_at(4), Some(kw));
    assert_eq!(cursor.scope_at(5), None);
    assert_eq!(cursor.scope_at(7), Some(fn_));
    assert_eq!(cursor.scope_at(9), None);
}

#[test]
fn interval_cursor_empty() {
    let mut cursor = IntervalCursor::<'_>::new(&[]);
    assert_eq!(cursor.scope_at(0), None);
    assert_eq!(cursor.scope_at(100), None);
}

/// Emits spans at two different tiers — proves tier is data on
/// `Decoration::Highlight`, not a per-provider property.
struct TwoTierSource(ScopeId);

impl DecorationSource for TwoTierSource {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::HIGHLIGHT
    }
    fn decorations_for_line(&self, _line_idx: usize, out: &mut Vec<Decoration>) {
        out.push(Decoration::Highlight {
            byte_start: 0,
            byte_end: 1,
            scope: self.0,
            tier: HighlightTier::Syntax,
        });
        out.push(Decoration::Highlight {
            byte_start: 2,
            byte_end: 3,
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

    rebuild_line_decorations(0, None, &providers, &rope, &mut scratch);

    assert_eq!(
        scratch.tier_bufs.0[HighlightTier::Syntax as usize],
        vec![(0, 1, scope)],
        "the Syntax-tier span lands in the Syntax bucket"
    );
    assert_eq!(
        scratch.tier_bufs.0[HighlightTier::Diagnostic as usize],
        vec![(2, 3, scope)],
        "the Diagnostic-tier span from the same source lands in a different bucket"
    );
}

#[test]
fn interval_cursor_adjacent_intervals() {
    // (2,5) and (5,8) are adjacent — byte 5 must match the second.
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(2, 5, kw), (5, 8, fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(4), Some(kw));
    assert_eq!(cursor.scope_at(5), Some(fn_));
    assert_eq!(cursor.scope_at(7), Some(fn_));
    assert_eq!(cursor.scope_at(8), None);
}
