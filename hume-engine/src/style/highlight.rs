use crate::builtins::tree_sitter_hl::TreeSitterHighlighter;
use crate::providers::{HighlightSource, HighlightTier, SourceContext};
use crate::theme::Theme;
use crate::types::{ResolvedStyle, ScopeId};
use super::StyleScratch;

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

/// Aggregated highlight intervals for one buffer line, one cursor per tier.
/// Built once before iterating graphemes, queried per grapheme in O(1) amortised.
pub(super) struct HighlightStack<'a> {
    syntax: IntervalCursor<'a>,
    search: IntervalCursor<'a>,
    diagnostic: IntervalCursor<'a>,
    bracket: IntervalCursor<'a>,
}

impl<'a> HighlightStack<'a> {
    pub(super) fn new(tiers: &'a TierBufs) -> Self {
        Self {
            syntax: IntervalCursor::new(&tiers.syntax),
            search: IntervalCursor::new(&tiers.search),
            diagnostic: IntervalCursor::new(&tiers.diagnostic),
            bracket: IntervalCursor::new(&tiers.bracket),
        }
    }

    /// Layer all active highlight tiers at `byte_offset` into `base`.
    ///
    /// Each `theme.resolve(id)` call is an O(1) `Vec` index into the baked
    /// style array — no hashing on the per-grapheme hot path.
    pub(super) fn layer_at(
        &mut self,
        byte_offset: usize,
        mut base: ResolvedStyle,
        theme: &Theme,
    ) -> ResolvedStyle {
        // Syntax (lowest)
        if let Some(id) = self.syntax.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Search match
        if let Some(id) = self.search.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Diagnostic
        if let Some(id) = self.diagnostic.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
        }
        // Bracket match (highest highlight)
        if let Some(id) = self.bracket.scope_at(byte_offset) {
            base = base.layer(theme.resolve(id));
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
#[derive(Default)]
pub struct TierBufs {
    syntax: Vec<(usize, usize, ScopeId)>,
    search: Vec<(usize, usize, ScopeId)>,
    diagnostic: Vec<(usize, usize, ScopeId)>,
    bracket: Vec<(usize, usize, ScopeId)>,
}

impl TierBufs {
    pub fn clear(&mut self) {
        self.syntax.clear();
        self.search.clear();
        self.diagnostic.clear();
        self.bracket.clear();
    }

    fn push(&mut self, tier: HighlightTier, interval: (usize, usize, ScopeId)) {
        match tier {
            HighlightTier::Syntax => self.syntax.push(interval),
            HighlightTier::SearchMatch => self.search.push(interval),
            HighlightTier::Diagnostic => self.diagnostic.push(interval),
            HighlightTier::BracketMatch => self.bracket.push(interval),
        }
    }

    fn sort_all(&mut self) {
        self.syntax.sort_by_key(|i| i.0);
        self.search.sort_by_key(|i| i.0);
        self.diagnostic.sort_by_key(|i| i.0);
        self.bracket.sort_by_key(|i| i.0);
    }
}

// ── rebuild_tier_bufs ─────────────────────────────────────────────────────────

/// Gather highlight intervals from all providers for one buffer line.
///
/// Must be called once per buffer line before calling [`super::style_row`] for
/// that line's display rows. Clears and re-fills `tier_bufs` and `raw_highlights`.
///
/// `syntax` is the buffer-level tree-sitter highlighter (if a language is
/// configured). It runs first into the `Syntax` tier bucket before any
/// per-pane provider-based sources.
pub(crate) fn rebuild_tier_bufs(
    line_idx: usize,
    syntax: Option<&TreeSitterHighlighter>,
    providers: &[Box<dyn HighlightSource>],
    rope: &ropey::Rope,
    tree: Option<&tree_sitter::Tree>,
    scratch: &mut StyleScratch,
) {
    scratch.tier_bufs.clear();
    scratch.highlights.clear();
    let ctx = SourceContext {
        rope,
        tree,
        line_start_byte: rope.line_to_byte(line_idx),
    };
    if let Some(hl) = syntax {
        hl.highlights_for_line(line_idx, &ctx, &mut scratch.highlights);
        for &interval in scratch.highlights.iter() {
            scratch.tier_bufs.push(HighlightTier::Syntax, interval);
        }
        scratch.highlights.clear();
    }
    for provider in providers {
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
