use super::StyleScratch;
use crate::providers::{Decoration, DecorationKinds, HighlightTier, ProviderSet, SyntaxSpans};
use crate::theme::Theme;
use crate::types::{ResolvedStyle, ScopeId};

// ── Interval cursor ────────────────────────────────────────────────────────────

/// Walks a sorted, non-overlapping slice of `(byte_start, byte_end, ScopeId)`
/// intervals in order. Queries must be monotonically non-decreasing.
///
/// `pub(crate)`: also used by `rows::segment_virtual_row` to resolve
/// per-grapheme scopes for virtual lines, the same interval shape as
/// `Decoration::Highlight`/`SyntaxSpans`.
pub(crate) struct IntervalCursor<'a> {
    intervals: &'a [(usize, usize, ScopeId)],
    pos: usize,
}

impl<'a> IntervalCursor<'a> {
    pub(crate) fn new(intervals: &'a [(usize, usize, ScopeId)]) -> Self {
        Self { intervals, pos: 0 }
    }

    /// Return the scope id active at `byte_offset`, or `None`.
    /// Advances the internal cursor forward; never goes backward.
    pub(crate) fn scope_at(&mut self, byte_offset: usize) -> Option<ScopeId> {
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
/// Indexed by `HighlightTier as usize`; see `HighlightStack`.
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

// ── rebuild_line_decorations ─────────────────────────────────────────────────

/// Gather highlight intervals from the syntax source and every `PAINT`-kind
/// `DecorationSource` for one buffer line, returning the line's background
/// tint (`Decoration::LineBg`), if any provider requested one.
///
/// Must be called once per buffer line before calling [`super::style_row`] for
/// that line's display rows. Clears and re-fills `tier_bufs`.
///
/// `syntax` is the buffer's syntax span source (if a language is
/// configured). Its spans for this line are merged into the `Syntax`
/// tier bucket before any per-pane provider-based sources.
pub(crate) fn rebuild_line_decorations(
    line_idx: usize,
    syntax: Option<&dyn SyntaxSpans>,
    providers: &ProviderSet,
    rope: &ropey::Rope,
    scratch: &mut StyleScratch,
) -> Option<ScopeId> {
    scratch.tier_bufs.clear();
    scratch.syntax_spans.clear();
    if let Some(syntax) = syntax {
        syntax.spans_for_line(line_idx, rope, &mut scratch.syntax_spans);
        for &interval in scratch.syntax_spans.iter() {
            scratch.tier_bufs.push(HighlightTier::Syntax, interval);
        }
        scratch.syntax_spans.clear();
    }
    scratch.decorations.clear();
    for (_, provider) in providers.decoration_sources(DecorationKinds::PAINT) {
        provider.decorations_for_line(line_idx, &mut scratch.decorations);
    }
    // Last LineBg wins if more than one PAINT-kind provider tints the same
    // line — the editor bridge already collapses multi-source ties to one
    // record per line (`update_line_bg_providers`), so this only matters for
    // a hypothetical second engine-side provider.
    let mut tint = None;
    for d in scratch.decorations.drain(..) {
        match d {
            Decoration::Highlight {
                byte_start,
                byte_end,
                scope,
                tier,
            } => scratch.tier_bufs.push(tier, (byte_start, byte_end, scope)),
            Decoration::LineBg(scope) => tint = Some(scope),
            Decoration::VirtualLine(_) | Decoration::Inline(_) => {
                // A provider that declared PAINT but emitted a
                // VIRTUAL_LINE/INLINE kind is a provider bug — ignored, not
                // a panic, same posture `rows::RowMap` takes for the
                // reverse case.
            }
        }
    }
    scratch.tier_bufs.sort_all();
    tint
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
