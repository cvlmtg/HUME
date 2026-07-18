use super::StyleScratch;
use crate::builtins::tree_sitter_hl::layer_highlights_for_line;
use crate::providers::{HighlightSource, HighlightTier, ProviderId, SourceContext};
use crate::syntax_layers::SyntaxLayers;
use crate::theme::Theme;
use crate::types::{ResolvedStyle, ScopeId};

// ── Interval cursor ────────────────────────────────────────────────────────────

/// Walks a sorted, non-overlapping slice of `(byte_start, byte_end, ScopeId)`
/// intervals in order. Queries must be monotonically non-decreasing.
struct IntervalCursor<'a> {
    intervals: &'a [(usize, usize, ScopeId)],
    pos: usize,
}

impl<'a> IntervalCursor<'a> {
    fn new(intervals: &'a [(usize, usize, ScopeId)]) -> Self {
        Self { intervals, pos: 0 }
    }

    /// Return the scope id active at `byte_offset`, or `None`.
    /// Advances the internal cursor forward; never goes backward.
    fn scope_at(&mut self, byte_offset: usize) -> Option<ScopeId> {
        // Skip intervals that have already ended.
        while self.pos < self.intervals.len() && self.intervals[self.pos].1 <= byte_offset {
            self.pos += 1;
        }
        // Check if the current interval covers `byte_offset`.
        if self.pos < self.intervals.len() {
            let (start, end, id) = self.intervals[self.pos];
            if start <= byte_offset && byte_offset < end {
                return Some(id);
            }
        }
        None
    }
}

// ── Highlight stack ────────────────────────────────────────────────────────────

/// Number of [`HighlightTier`] variants — the tier arrays in [`HighlightStack`]
/// and [`TierBufs`] are indexed by `tier as usize`, so this must track the
/// enum exactly (see `HighlightTier`'s doc for the discriminant assignments).
const TIER_COUNT: usize = 5;

/// Aggregated highlight intervals for one buffer line, one cursor per tier.
/// Built once before iterating graphemes, queried per grapheme in O(1) amortised.
///
/// Indexed by `HighlightTier as usize`, so iterating `tiers` in order visits
/// tiers from lowest to highest layering priority (see [`HighlightTier`]).
pub(super) struct HighlightStack<'a> {
    tiers: [IntervalCursor<'a>; TIER_COUNT],
}

impl<'a> HighlightStack<'a> {
    pub(super) fn new(tiers: &'a TierBufs) -> Self {
        Self {
            tiers: tiers
                .0
                .each_ref()
                .map(|intervals| IntervalCursor::new(intervals)),
        }
    }

    /// Layer all active highlight tiers at `byte_offset` into `base`, lowest
    /// priority first (`Syntax`) through highest (`BracketMatch`).
    ///
    /// Each `theme.resolve(id)` call is an O(1) `Vec` index into the baked
    /// style array — no hashing on the per-grapheme hot path.
    pub(super) fn layer_at(
        &mut self,
        byte_offset: usize,
        mut base: ResolvedStyle,
        theme: &Theme,
    ) -> ResolvedStyle {
        for cursor in &mut self.tiers {
            if let Some(id) = cursor.scope_at(byte_offset) {
                base = base.layer(theme.resolve(id));
            }
        }
        base
    }
}

// ── TierBufs ──────────────────────────────────────────────────────────────────

/// Scratch buffer holding sorted highlight intervals split by tier.
/// Owned by `FrameScratch` so capacity is retained across frames.
///
/// Each interval is `(byte_start, byte_end, ScopeId)` — the `ScopeId` maps to
/// a pre-baked [`ResolvedStyle`] via an O(1) `Vec` index in [`Theme::resolve`].
/// Indexed by `HighlightTier as usize`; see [`HighlightStack`].
#[derive(Default)]
pub struct TierBufs([Vec<(usize, usize, ScopeId)>; TIER_COUNT]);

impl TierBufs {
    pub fn clear(&mut self) {
        for buf in &mut self.0 {
            buf.clear();
        }
    }

    fn push(&mut self, tier: HighlightTier, interval: (usize, usize, ScopeId)) {
        self.0[tier as usize].push(interval);
    }

    fn sort_all(&mut self) {
        for buf in &mut self.0 {
            buf.sort_by_key(|i| i.0);
        }
    }
}

// ── rebuild_tier_bufs ─────────────────────────────────────────────────────────

/// Gather highlight intervals from all providers for one buffer line.
///
/// Must be called once per buffer line before calling [`super::style_row`] for
/// that line's display rows. Clears and re-fills `tier_bufs` and `raw_highlights`.
///
/// `syntax` is the buffer's tree-sitter syntax layers (if a language is
/// configured). Every layer covering this line is merged into the `Syntax`
/// tier bucket before any per-pane provider-based sources.
pub(crate) fn rebuild_tier_bufs(
    line_idx: usize,
    syntax: Option<&SyntaxLayers>,
    providers: &[(ProviderId, Box<dyn HighlightSource>)],
    rope: &ropey::Rope,
    scratch: &mut StyleScratch,
) {
    scratch.tier_bufs.clear();
    scratch.highlights.clear();
    if let Some(layers) = syntax {
        layer_highlights_for_line(
            layers,
            line_idx,
            rope,
            &mut scratch.ts_raw,
            &mut scratch.ts_stack,
            &mut scratch.ts_events,
            &mut scratch.highlights,
        );
        for &interval in scratch.highlights.iter() {
            scratch.tier_bufs.push(HighlightTier::Syntax, interval);
        }
        scratch.highlights.clear();
    }
    let ctx = SourceContext {
        rope,
        line_start_byte: rope.line_to_byte(line_idx),
    };
    for (_, provider) in providers {
        provider.highlights_for_line(line_idx, &ctx, &mut scratch.highlights);
        for &interval in scratch.highlights.iter() {
            scratch.tier_bufs.push(provider.tier(), interval);
        }
        scratch.highlights.clear();
    }
    scratch.tier_bufs.sort_all();
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
}
