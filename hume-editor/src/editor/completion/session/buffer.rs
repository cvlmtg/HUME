//! The Insert-mode completion session: [`BufferSession`]. Every text change
//! on its buffer is carried into it by `PositionStores` ([`BufferSession::carry`]);
//! what a change means for the open menu is decided by
//! [`BufferSession::reconcile`] against the live text and cursor, unlike
//! [`super::MinibufSession`], which dismisses on any key but Tab.

use std::ops::Range;

use hume_editing::changeset::{Assoc, ChangeSet, PosMapCursor};
use hume_editing::edit::TextChange;
use hume_editing::text::{BufferText, TextVersion};
use hume_editing::tracked::Tracked;
use hume_editing::word::{CharClass, WordChars};
use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::lsp::wire_range_to_chars;

use super::super::item::CompletionItem;
use super::super::registry::{BufferSourceId, SourceRegistry};
use super::slots::{Invocation, SlotSet, SlotTokens};

/// The coordinate system one invocation's answer was computed in: the rope
/// at invoke time (an O(1) clone, since ropey is structurally shared) and every
/// change carried since, composed. A server's wire `textEdit` range is
/// computed against the document as it stood at the *request*, which is
/// this snapshot; `accept` decodes against `rope` and maps forward through
/// `cs_since` rather than approximating drift as a scalar shift.
pub(super) struct DocSnapshot {
    pub(super) rope: ropey::Rope,
    pub(super) cs_since: ChangeSet,
}

/// Where a `Buffer` invocation's token starts in live coordinates. The token
/// ends at the cursor, so only its start is stored.
pub(in crate::editor) struct BufferSpan {
    pub(super) doc: DocSnapshot,
    /// Fixed at invoke time by the source's token class and mapped
    /// `Assoc::Before` through every carried change (text inserted exactly
    /// at the start belongs to the token).
    pub(super) start: CharOffset,
    /// The character before `start` was deleted by a carried change: the
    /// edit reached past the token, so the answer no longer applies.
    crossed: bool,
    /// The distinct edit-range starts of this answer's items, ascending and
    /// carried like `start`; an item's `token_group` indexes it. The server
    /// says where an item's token starts, which need not be where the
    /// source's own token rule puts `start`.
    starts: Vec<CharOffset>,
}

impl BufferSpan {
    /// Groups `items` by the start of their own edit range: decodes each
    /// against the request-time snapshot, then maps the distinct starts
    /// through the changes carried since the request, so they are live like
    /// `start`. An item whose range cannot be decoded keeps no group and is
    /// filtered against `start`; accepting it reports the problem.
    fn assign_token_groups(&mut self, items: &mut [CompletionItem], may_resolve: bool) {
        let BufferSpan { doc, starts, .. } = self;
        let item_starts: Vec<Option<CharOffset>> = items
            .iter()
            .map(|item| {
                let te = item.text_edit.as_ref()?;
                let encoding = item.wire_encoding(may_resolve, "").ok()?;
                Some(wire_range_to_chars(&doc.rope, &te.range, encoding).start)
            })
            .collect();
        let mut distinct: Vec<CharOffset> = item_starts.iter().flatten().copied().collect();
        distinct.sort_unstable();
        distinct.dedup();
        for (item, start) in items.iter_mut().zip(&item_starts) {
            item.token_group = start
                .and_then(|s| distinct.binary_search(&s).ok())
                .map(|g| g as u32);
        }
        doc.cs_since.map_positions(&mut distinct, Assoc::Before);
        *starts = distinct;
    }
}

impl Invocation<BufferSpan> {
    /// A `Buffer`-target invocation whose token starts at `start`.
    pub(in crate::editor) fn buffer(rope: ropey::Rope, start: CharOffset) -> Self {
        let cs_since = ChangeSet::identity(rope.len_chars());
        Self::new(BufferSpan {
            doc: DocSnapshot { rope, cs_since },
            start,
            crossed: false,
            starts: Vec::new(),
        })
    }

    /// The seeded filter text for this invocation (`text[start..head]`),
    /// handed to a Steel source as its `prefix` argument.
    pub(in crate::editor) fn prefix(&self, text: &BufferText, head: CharOffset) -> String {
        token_text(text, self.span.start, head)
    }

    /// Composes `cs` into this invocation's snapshot and maps its token
    /// start. Makes no decision about what the change means; see
    /// [`BufferSession::reconcile`].
    fn carry(&mut self, cs: &ChangeSet) {
        let BufferSpan {
            doc,
            start,
            crossed,
            starts,
        } = &mut self.span;
        // One cursor over the non-decreasing pair `[start-1, start]`:
        // `anchor_deleted` on `start-1` answers "was the token's start
        // character itself deleted?" (a Backspace at the token's own
        // start) directly, rather than inferring it from two positions
        // mapping to the same spot.
        let mut cursor = PosMapCursor::new(cs.ops());
        if *start > CharOffset::new(0)
            && cursor
                .map_anchor(start.retreat(1), Assoc::Before)
                .anchor_deleted
        {
            *crossed = true;
        }
        *start = cursor.map_anchor(*start, Assoc::Before).pos;
        cs.map_positions(starts, Assoc::Before);
        // `compose` takes `self` by value; swap the accumulated changeset
        // out (rather than cloning its whole op list, including every
        // typed-text `Insert` op, just to feed it in) and compose in place.
        let prev = std::mem::replace(&mut doc.cs_since, ChangeSet::identity(0));
        doc.cs_since = prev.compose(cs.clone());
    }

    /// Whether the token `start..head` is still the one this invocation
    /// answered for: not crossed, the cursor still at or past its start, and
    /// every character between them in the source's token class.
    fn holds(&self, text: &BufferText, head: CharOffset, chars: WordChars<'_>) -> bool {
        let BufferSpan { start, crossed, .. } = &self.span;
        !*crossed
            && *start <= head
            && text
                .slice(ExclusiveRange::new(*start, head))
                .chars()
                .all(|c| chars.classify(c) == CharClass::Word)
    }
}

/// The document a `Buffer` session's tokens are read against when ranking
/// the live text and the primary cursor's head.
pub(in crate::editor) struct LiveDoc<'a> {
    pub(in crate::editor) text: &'a BufferText,
    pub(in crate::editor) head: CharOffset,
}

/// What [`BufferSession::reconcile`] found.
pub(in crate::editor) enum Reconciled {
    /// The session cannot survive: its text was replaced.
    Dismiss,
    /// The text and cursor are the ones the session last saw and no answer
    /// was dropped.
    Unchanged,
    /// The text or cursor moved, or an answer was dropped, so the ranking is
    /// stale. `text_changed` also asks for the sources that want a fresh
    /// call ([`BufferSession::sources_to_reinvoke`]) to be called again.
    Changed { text_changed: bool },
}

/// The Insert-mode completion session: one per open menu, on one buffer.
pub(in crate::editor) struct BufferSession {
    bid: BufferId,
    /// Pane the session began in: `accept` only proceeds while this pane is
    /// still focused. A completion resolved against a pane the user has
    /// since navigated away from has no well-defined live cursor to land at,
    /// and `PaneBufferState`'s own `ensure` would otherwise silently
    /// fabricate one (see `accept`'s pane precondition).
    pub(super) pane_id: PaneId,
    /// Text version and primary head as of the last [`Self::reconcile`]:
    /// `accept` rejects if the buffer changed since.
    pub(super) version: TextVersion,
    head: CharOffset,
    /// The buffer's text was replaced with no change to carry
    /// ([`Self::forget`]): no position in the session means anything now.
    replaced: bool,
    /// Whether an explicit `Trigger::Explicit` (Ctrl-Space) has touched
    /// this session, as opposed to only ever a trigger char. Gates the
    /// settle-time "no completions" report (`orchestrate.rs`'s
    /// `contribute`): silent for a session the user never explicitly asked
    /// anything of, reported once they did and got nothing. `false` at
    /// open; [`Self::mark_explicit_trigger`] sets it, never unset.
    explicit: bool,
    /// The leftmost live token start among the slots [`Self::rank`] just
    /// gave at least one ranked candidate, folded into that same per-slot
    /// pass rather than recomputed by a second walk over the ranked list on
    /// every render frame, snapped to the cluster holding it. `None` with
    /// nothing ranked; reads as absent once the text moves past the ranking.
    menu_anchor: Option<Tracked<ClusterStart>>,
    /// `(slot, item)` pairs a lower-priority slot's plain item (see
    /// [`CompletionItem::is_plain`]) is hidden because a strictly-higher-
    /// priority slot already shows an item with the same `filter_text`,
    /// rebuilt by [`Self::recompute_dedup`] only when the shown item *set*
    /// changes (an answer lands, a slot is dropped), not every keystroke;
    /// [`Self::rank`] only ever reads it.
    dedup_hidden: rustc_hash::FxHashSet<(u32, u32)>,
    core: SlotSet<BufferSourceId, BufferSpan>,
}

impl BufferSession {
    /// A session on `bid`, shown in `pane_id`, opened with the primary
    /// cursor at `head` in the text at `version`.
    pub(in crate::editor) fn open(
        bid: BufferId,
        pane_id: PaneId,
        version: TextVersion,
        head: CharOffset,
    ) -> Self {
        Self {
            bid,
            pane_id,
            version,
            head,
            replaced: false,
            explicit: false,
            menu_anchor: None,
            dedup_hidden: rustc_hash::FxHashSet::default(),
            core: SlotSet::new(),
        }
    }

    pub(in crate::editor) fn bid(&self) -> BufferId {
        self.bid
    }

    pub(in crate::editor) fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    /// Whether an explicit `Trigger::Explicit` (Ctrl-Space) has touched
    /// this session; see [`Self::explicit`]'s doc.
    pub(in crate::editor) fn is_explicit(&self) -> bool {
        self.explicit
    }

    /// Records that an explicit trigger touched this session. Called by
    /// `trigger_buffer_completion` on both the fresh-open and the
    /// reused-session path, so a session a trigger char opened still
    /// reports once the user follows up with Ctrl-Space.
    pub(in crate::editor) fn mark_explicit_trigger(&mut self) {
        self.explicit = true;
    }

    // ── Sources in and out ───────────────────────────────────────────────────

    /// Records a fresh call of a source, superseding any still in flight
    /// for it.
    pub(in crate::editor) fn invoke(
        &mut self,
        source: BufferSourceId,
        invocation: Invocation<BufferSpan>,
    ) -> u64 {
        self.core.invoke(source, invocation)
    }

    /// Lands an answer for invocation `id`, grouping its items by their own
    /// edit-range start. Then, if it landed, rebuilds [`Self::dedup_hidden`]
    /// against the item set as it now stands.
    pub(in crate::editor) fn contribute(
        &mut self,
        sources: &SourceRegistry,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        let landed = self
            .core
            .contribute(id, items, incomplete, |source, span, items| {
                let may_resolve = sources.buffer_get(source).resolve;
                span.assign_token_groups(items, may_resolve);
            });
        if landed {
            self.recompute_dedup(sources);
        }
        landed
    }

    /// Whether any slot still awaits an answer. Test-only: production reads
    /// [`Self::is_spent`] instead, which already folds this in.
    #[cfg(test)]
    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.core.is_pending()
    }

    /// Whether any source has answered with at least one item: a session
    /// can be `!is_pending()` yet still have nothing to show. Test-only:
    /// production reads [`Self::is_spent`] instead, which already folds
    /// this in.
    #[cfg(test)]
    pub(super) fn has_live_sources(&self) -> bool {
        self.core.is_live()
    }

    /// Whether the session has nothing left to show and nothing on its way
    /// (every source answered empty, or the cursor typed out of every
    /// token).
    pub(in crate::editor) fn is_spent(&self) -> bool {
        self.core.is_spent()
    }

    /// Clears every still-`inflight` invocation, leaving any existing
    /// `shown` answer untouched. Returns whether anything was actually
    /// cleared.
    ///
    /// The recovery path for a Steel call batch that failed before any of
    /// its queued sources could reach `completion-emit!`: without this, a
    /// source that raises (or one that simply never answers) leaves its
    /// slot permanently pending, since nothing else ever tells this session
    /// the call didn't happen. `sources_to_reinvoke` calls it again on
    /// every subsequent edit, so a broken source would otherwise error on
    /// every keystroke instead of just once.
    ///
    /// Safe regardless of *why* the batch failed, or whether it even
    /// touched completion at all: a source that was merely slow gets asked
    /// again through the ordinary edit-driven reinvocation path, or a
    /// fresh trigger; one that's genuinely broken simply stops being asked
    /// until then, rather than erroring forever.
    pub(in crate::editor) fn drop_stalled_invocations(&mut self) -> bool {
        self.core.drop_stalled()
    }

    /// The sources to call again after an edit: those that flagged their
    /// latest answer `isIncomplete`, and those still pending (their
    /// in-flight call saw an older document, so it is superseded rather
    /// than waited for).
    pub(in crate::editor) fn sources_to_reinvoke(&self) -> Vec<BufferSourceId> {
        self.core.sources_to_reinvoke()
    }

    // ── Edits ────────────────────────────────────────────────────────────────

    /// Carries every invocation's snapshot and token start through `change`
    /// when it landed on this session's buffer: every keystroke, not just
    /// ones at the primary cursor (a keystroke at a cursor *before* the
    /// primary shifts every token). Called by `PositionStores::carry` for
    /// every text change, so no edit path can skip it.
    pub(in crate::editor) fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        if buffer != self.bid || self.replaced {
            return;
        }
        let cs = change.changes();
        for slot in self.core.slots_mut() {
            for inv in [&mut slot.inflight, &mut slot.shown].into_iter().flatten() {
                inv.carry(cs);
            }
        }
    }

    /// Marks the session dead: `buffer`'s text was replaced with no change
    /// to carry the session's positions through.
    pub(in crate::editor) fn forget(&mut self, buffer: BufferId) {
        if buffer == self.bid {
            self.replaced = true;
        }
    }

    /// Decides what the buffer's current `text` and primary `head` mean for
    /// the open menu, drops the answer of any slot whose token they have
    /// left, and records the state it saw. Idempotent: the same state
    /// reconciled twice reports [`Reconciled::Unchanged`] the second time.
    /// `word_chars` is this buffer's; each slot extends it with its own
    /// source's token chars.
    pub(in crate::editor) fn reconcile(
        &mut self,
        sources: &SourceRegistry,
        text: &BufferText,
        head: CharOffset,
        word_chars: &str,
    ) -> Reconciled {
        if self.replaced {
            return Reconciled::Dismiss;
        }
        let text_changed = text.version() != self.version;
        let moved = text_changed || head != self.head;
        // Whether any slot's `shown` answer was actually dropped below:
        // the one thing that can change which items dedup compares against,
        // so `recompute_dedup` runs only then.
        let mut dropped = false;
        for slot in self.core.slots_mut() {
            let token_chars = sources.buffer_get(slot.source).token_chars_over(word_chars);
            if let Some(inv) = &slot.shown
                && !inv.holds(text, head, WordChars::new(&token_chars))
            {
                slot.shown = None;
                dropped = true;
            }
        }
        if dropped {
            self.recompute_dedup(sources);
        }
        self.version = text.version();
        self.head = head;
        if moved || dropped {
            Reconciled::Changed { text_changed }
        } else {
            Reconciled::Unchanged
        }
    }

    /// Rebuilds [`Self::dedup_hidden`] from the current shown items: a
    /// plain item ([`CompletionItem::is_plain`]) is hidden when a strictly-
    /// higher-priority slot's shown answer has an item with the same
    /// `filter_text`. An item carrying edits is never hidden: accepting it
    /// does something a duplicate-*looking* plain item from another source
    /// wouldn't, so it stays regardless of what else duplicates its label.
    /// Priority is a static, per-source fact, so this only ever needs to
    /// run when the shown item *set* changes ([`Self::contribute`] landing
    /// an answer, [`Self::reconcile`] dropping a slot), not on every
    /// keystroke, unlike scoring itself.
    fn recompute_dedup(&mut self, sources: &SourceRegistry) {
        self.dedup_hidden.clear();
        let slots = self.core.slots_mut();
        // The highest priority among every shown item (plain or not, any
        // slot) carrying a given `filter_text`: O(total items), not the
        // O(items²) an all-pairs scan across slots costs (a 20k-word
        // buffer-words slot against a 1k-item LSP slot is ~20M string
        // compares per landed answer with the naive version). A slot's own
        // priority never exceeds its own contribution to this map, so
        // below, a strict `>` against it already excludes comparing a slot
        // against itself; no separate index check needed.
        let mut max_priority: rustc_hash::FxHashMap<&str, i64> = rustc_hash::FxHashMap::default();
        for slot in slots.iter() {
            let Some(inv) = &slot.shown else { continue };
            let priority = sources.buffer_get(slot.source).priority;
            for item in inv.items() {
                max_priority
                    .entry(item.filter_text.as_str())
                    .and_modify(|p| *p = (*p).max(priority))
                    .or_insert(priority);
            }
        }
        for (s, slot) in slots.iter().enumerate() {
            let Some(inv) = &slot.shown else { continue };
            let priority = sources.buffer_get(slot.source).priority;
            for (i, item) in inv.items().iter().enumerate() {
                if !item.is_plain() {
                    continue;
                }
                let outranked = max_priority
                    .get(item.filter_text.as_str())
                    .is_some_and(|&p| p > priority);
                if outranked {
                    self.dedup_hidden.insert((s as u32, i as u32));
                }
            }
        }
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against its own slot's live token text
    /// (`live[start..head]`), with `live` supplying the document to read
    /// that token against, or `None` when the session's buffer isn't
    /// focused in any pane (`rerank_open_session`): nothing scores, the
    /// ranked list empties, and `menu_anchor` becomes `None`, same as if
    /// every slot's token had fallen out of range.
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry, live: Option<LiveDoc<'_>>) {
        let anchor = self
            .core
            .rank_with(sources, Some(&self.dedup_hidden), |inv| {
                let doc = live.as_ref()?;
                // An answer that landed since the last reconcile may start
                // after the cursor; its items are skipped rather than
                // sliced until the next reconcile drops it.
                let token = |start: CharOffset| {
                    (start <= doc.head).then(|| (token_text(doc.text, start, doc.head), start))
                };
                Some(SlotTokens {
                    base: token(inv.span.start),
                    groups: inv.span.starts.iter().map(|&start| token(start)).collect(),
                })
            });
        self.menu_anchor = anchor
            .zip(live)
            .map(|(start, doc)| Tracked::new(doc.text.snap(start), doc.text));
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    pub(in crate::editor) fn len(&self) -> usize {
        self.core.len()
    }

    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.core.is_empty()
    }

    pub(super) fn ranked(
        &self,
        idx: usize,
    ) -> Option<(BufferSourceId, &Invocation<BufferSpan>, &CompletionItem)> {
        self.core.ranked(idx)
    }

    /// Where the menu anchors: the leftmost live token start among the
    /// sources with a ranked candidate. Stable while cycling; moves only
    /// when ranking changes which sources contribute. `None` with nothing
    /// ranked. Computed once per [`Self::rank`] call, not per call to this
    /// accessor: both this and the per-frame render path need it. `None`
    /// too when `text`, the buffer's current text, is not the one the
    /// ranking read.
    pub(in crate::editor) fn menu_anchor(&self, text: &BufferText) -> Option<ClusterStart> {
        self.menu_anchor.as_ref()?.get(text).copied()
    }

    pub(in crate::editor) fn selected(&self) -> usize {
        self.core.selected()
    }

    pub(in crate::editor) fn step_selection(&mut self, forward: bool) -> bool {
        self.core.step_selection(forward)
    }

    pub(in crate::editor) fn top(
        &self,
        n: usize,
        sources: &SourceRegistry,
    ) -> Vec<serde_json::Value> {
        self.core.top(n, sources)
    }

    /// Row content for the candidates at `range` in ranked order: the only
    /// way rows leave this session, and `range` is `menu_window`'s own
    /// window (`hume_ui::popup`), so a frame formats at most `MAX_MENU_ROWS`
    /// rows and never the whole ranked list.
    pub(in crate::editor) fn rows_in(&self, range: Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        self.core.rows_in(range)
    }
}

/// `text[start..head]`: a `Buffer` invocation's token text in the live
/// buffer, for [`Invocation::prefix`]'s Steel-facing seed and
/// [`BufferSession::rank`]'s scoring pass. Both callers already know
/// `start <= head` before calling this.
fn token_text(text: &BufferText, start: CharOffset, head: CharOffset) -> String {
    text.slice(ExclusiveRange::new(start, head)).to_string()
}
