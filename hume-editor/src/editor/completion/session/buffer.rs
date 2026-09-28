//! The Insert-mode completion session: [`BufferSession`]. Refilters the
//! open menu in place as the user types (`observe_edit`), unlike
//! [`super::MinibufSession`], which dismisses on any key but Tab.

use std::ops::Range;

use hume_editing::changeset::{Assoc, ChangeSet, PosMapCursor};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::super::item::CompletionItem;
use super::super::registry::{BufferSourceId, SourceRegistry};
use super::contains_cursor;
use super::slots::{Invocation, SlotSet};
use crate::editor::EditorState;

/// The coordinate system one invocation's answer was computed in: the rope
/// at invoke time (an O(1) clone, since ropey is structurally shared) and every
/// edit observed since, composed. A server's wire `textEdit` range is
/// computed against the document as it stood at the *request*, which is
/// this snapshot; `accept` decodes against `rope` and maps forward through
/// `cs_since` rather than approximating drift as a scalar shift.
pub(super) struct DocSnapshot {
    pub(super) rope: ropey::Rope,
    pub(super) cs_since: ChangeSet,
}

/// The token in live coordinates that a `Buffer` invocation answered for,
/// fixed at invoke time (the source's token rule: the word before the
/// cursor) and tracked through every edit since via [`Invocation::observe`].
pub(in crate::editor) struct BufferSpan {
    pub(super) doc: DocSnapshot,
    /// `start` mapped `Assoc::Before` through every observed edit (text
    /// inserted exactly at the token's start belongs to the token), `end`
    /// mapped `Assoc::After` (text typed at the token's end extends it).
    pub(super) live: Range<CharOffset>,
}

impl Invocation<BufferSpan> {
    /// A `Buffer`-target invocation. `live` is the word before the cursor
    /// (every buffer source's token rule), so `live.end` *is* the cursor at
    /// invoke time; there is no separate "head" to track alongside it.
    pub(in crate::editor) fn buffer(rope: ropey::Rope, live: Range<CharOffset>) -> Self {
        let cs_since = ChangeSet::identity(rope.len_chars());
        Self::new(BufferSpan {
            doc: DocSnapshot { rope, cs_since },
            live,
        })
    }

    /// The seeded filter text for this invocation (`text[live.start ..
    /// live.end]` as of the invoke), handed to a Steel source as its
    /// `prefix` argument.
    pub(in crate::editor) fn prefix(&self, text: &BufferText) -> String {
        token_text(text, self.span.live.start, self.span.live.end)
    }

    /// Composes `cs` into this invocation's snapshot and remaps its live
    /// span. Returns `false` when the edit crossed the token's start; see
    /// [`BufferSession::observe_edit`]. `text`/`chars` are the *live*
    /// (post-`cs`) document and this buffer's word-chars, needed only to
    /// classify a newly-included end-of-token slice; see `end`'s own
    /// comment below.
    fn observe(&mut self, cs: &ChangeSet, text: &BufferText, chars: WordChars<'_>) -> bool {
        let BufferSpan { doc, live: range } = &mut self.span;
        // One cursor over the non-decreasing sequence `[start-1, start,
        // end]`: `map_anchor`'s `anchor_deleted` on `start-1` answers "was
        // the token's start character itself deleted?" (a Backspace at the
        // token's own start) directly, rather than inferring it from two
        // positions mapping to the same spot.
        let mut cursor = PosMapCursor::new(cs.ops());
        let crossed = if range.start > CharOffset::new(0) {
            cursor
                .map_anchor(range.start.retreat(1), Assoc::Before)
                .anchor_deleted
        } else {
            false
        };
        let start = cursor.map_anchor(range.start, Assoc::Before).pos;
        // Two calls on the same, already-visited position is safe:
        // `PosMapCursor::map_anchor` only advances its own state past an
        // op once a *later* position is queried, so re-querying `range.end`
        // immediately after `start` (itself `<= range.end`) never violates
        // the cursor's own non-decreasing-queries contract. The two only
        // ever disagree when an `Insert` op sits exactly at `range.end`:
        // `Assoc::Before` stops short of it, `Assoc::After` extends past it.
        let end_before = cursor.map(range.end, Assoc::Before);
        let end_after = cursor.map(range.end, Assoc::After);
        // An insertion exactly at the *non-empty* token's end only extends
        // the tracked span when every newly-included char is itself
        // word-class, matching the token's own definition ("the word
        // before the cursor"). Unconditionally taking `end_after` let an
        // auto-paired bracket (or any other non-word char) landing there
        // silently join the token: `accept`'s containment check then
        // trivially succeeds once the live head sits exactly at the grown
        // boundary, masking the "cursor left the token" case this span
        // exists to detect. An *empty* token (`range` was already
        // zero-width: no word typed yet, matching every candidate) has no
        // word for a non-word char to violate, so it stays "live" (chasing
        // the cursor) unconditionally until something more definite ends
        // it (a start-side Backspace, an out-of-band cursor jump), the
        // same as any other source not yet narrowed by typing.
        let end = if range.start == range.end
            || (end_after > end_before
                && token_text(text, end_before, end_after)
                    .chars()
                    .all(|c| chars.classify(c) == CharClass::Word))
        {
            end_after
        } else {
            end_before
        };
        *range = start..end;
        // `compose` takes `self` by value; swap the accumulated changeset
        // out (rather than cloning its whole op list, including every
        // typed-text `Insert` op, just to feed it in) and compose in place.
        let prev = std::mem::replace(&mut doc.cs_since, ChangeSet::identity(0));
        doc.cs_since = prev.compose(cs.clone());
        !crossed
    }

    /// Whether `head` is still inside this invocation's live token.
    fn contains(&self, head: CharOffset) -> bool {
        contains_cursor(&self.span.live, head)
    }
}

/// The document a `Buffer` session's tokens are read against when ranking
/// the live text and the primary cursor's head.
pub(in crate::editor) struct LiveDoc<'a> {
    pub(in crate::editor) text: &'a BufferText,
    pub(in crate::editor) head: CharOffset,
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
    /// Buffer generation as of the last edit this session observed via
    /// [`Self::observe_edit`]. `accept` rejects if the buffer changed by
    /// any other path since.
    pub(super) generation: u64,
    /// The buffer's length as of that same edit: what the next observed
    /// `ChangeSet`'s `len_before` must equal, or an edit reached the buffer
    /// through a path this session never saw.
    len: usize,
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
    /// every render frame. `None` with nothing ranked.
    menu_anchor: Option<CharOffset>,
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
    /// A session on `bid`, shown in `pane_id`, whose text is `len` chars at
    /// generation `generation`: the state the first [`Self::observe_edit`]
    /// checks against.
    pub(in crate::editor) fn open(
        bid: BufferId,
        pane_id: PaneId,
        generation: u64,
        len: usize,
    ) -> Self {
        Self {
            bid,
            pane_id,
            generation,
            len,
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

    /// Whether this session's pane/buffer/generation still match live state
    /// (`Editor::dismiss_invalid_completion`'s settle-time check). A coarse
    /// yes/no, unlike `accept`'s own preconditions (`checked_buffer`, the
    /// focus and pane-shows-buffer checks in `accept.rs`), which stay
    /// separate because they each need their own distinct Steel-facing
    /// error message; this one only ever feeds a silent background dismiss.
    pub(in crate::editor) fn still_valid(&self, state: &EditorState, view: &EngineView) -> bool {
        state.focus.id() == self.pane_id
            && view.panes.get(self.pane_id).map(|p| p.buffer_id) == Some(self.bid)
            && state.buffers.try_get(self.bid).map(|b| b.text_gen) == Some(self.generation)
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

    /// Lands an answer for invocation `id`. Then, if it landed, rebuilds
    /// [`Self::dedup_hidden`] against the item set as it now stands.
    pub(in crate::editor) fn contribute(
        &mut self,
        sources: &SourceRegistry,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        let landed = self.core.contribute(id, items, incomplete);
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

    /// Records an Insert-mode edit that landed on this session's buffer:
    /// every keystroke, not just ones at the primary cursor (a keystroke at
    /// a cursor *before* the primary shifts every token). Composes `cs`
    /// into each invocation's `cs_since`, remaps each live span, and drops
    /// the answer of any slot whose token the cursor has left: `head`
    /// outside `[start, end]`, or the character *before* the token deleted
    /// (a Backspace at the token's start).
    ///
    /// Returns `false` (session untouched) when `cs` wasn't produced
    /// against this session's own tracked document length: an edit reached
    /// the buffer through a path this session never observed, which
    /// `ChangeSet::compose` would otherwise turn into a hard panic (its
    /// `len_before`/`len_after` check is a release `assert_eq!`). The caller
    /// must dismiss the session in that case. `text_gen` is the buffer's
    /// generation *after* `cs` landed. `text`/`chars` are the live
    /// (post-`cs`) document and this buffer's word-chars, threaded through
    /// to [`Invocation::observe`], which needs them only to classify a
    /// newly-included end-of-token slice.
    pub(in crate::editor) fn observe_edit(
        &mut self,
        sources: &SourceRegistry,
        cs: &ChangeSet,
        text_gen: u64,
        head: CharOffset,
        text: &BufferText,
        chars: WordChars<'_>,
    ) -> bool {
        if cs.len_before() != self.len {
            return false;
        }
        self.len = cs.len_after();
        self.generation = text_gen;
        // Whether any slot's `shown` answer was actually dropped below:
        // the one thing that can change which items dedup compares against,
        // so `recompute_dedup` runs only then, not on every edit.
        let mut dropped = false;
        for slot in self.core.slots_mut() {
            if let Some(inv) = &mut slot.inflight {
                inv.observe(cs, text, chars);
            }
            if let Some(inv) = &mut slot.shown
                && !inv.observe(cs, text, chars)
            {
                slot.shown = None;
                dropped = true;
            }
            if let Some(inv) = &slot.shown
                && !inv.contains(head)
            {
                slot.shown = None;
                dropped = true;
            }
        }
        if dropped {
            self.recompute_dedup(sources);
        }
        true
    }

    /// Rebuilds [`Self::dedup_hidden`] from the current shown items: a
    /// plain item ([`CompletionItem::is_plain`]) is hidden when a strictly-
    /// higher-priority slot's shown answer has an item with the same
    /// `filter_text`. An item carrying edits is never hidden: accepting it
    /// does something a duplicate-*looking* plain item from another source
    /// wouldn't, so it stays regardless of what else duplicates its label.
    /// Priority is a static, per-source fact, so this only ever needs to
    /// run when the shown item *set* changes ([`Self::contribute`] landing
    /// an answer, [`Self::observe_edit`] dropping a slot), not on every
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
        self.menu_anchor = self
            .core
            .rank_with(sources, Some(&self.dedup_hidden), |inv| {
                let doc = live.as_ref()?;
                let start = inv.span.live.start;
                // A shown slot always contains `head` after `observe_edit` (see
                // its doc), so an inverted range here can only mean an edit
                // this session never saw, skipped rather than sliced, until
                // the settle-time validity check dismisses the session.
                (start <= doc.head).then(|| (token_text(doc.text, start, doc.head), start))
            });
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
    /// accessor: both this and the per-frame render path need it.
    pub(in crate::editor) fn menu_anchor_char(&self) -> Option<CharOffset> {
        self.menu_anchor
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

/// `text[start..head]`: a `Buffer` invocation's token text, in whatever
/// document `text` is (the invocation's own snapshot for
/// [`Invocation::prefix`]'s Steel-facing seed, the live buffer for
/// [`BufferSession::rank`]'s scoring pass). Both callers already know
/// `start <= head` before calling this.
fn token_text(text: &BufferText, start: CharOffset, head: CharOffset) -> String {
    text.slice(ExclusiveRange::new(start, head)).to_string()
}
