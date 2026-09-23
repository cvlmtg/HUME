//! The `:` command-line completion session — [`MinibufSession`]. Always
//! cycle-and-apply (Tab/Shift-Tab move the selection *and* immediately
//! splice the newly-selected candidate into the minibuffer): the popup's
//! own key handler (`input_stack/completion.rs`) dismisses on any other
//! key, unlike [`super::BufferSession`], which refilters in place as the
//! user types.

use std::ops::Range;

use super::super::item::CompletionItem;
use super::super::registry::{MinibufSourceId, SourceRegistry};
use super::slots::{Invocation, SlotSet};

/// Byte range in [`MinibufSession`]'s own `input` that a `Minibuf`
/// invocation answered for. Never moves — nothing can edit the `:` line
/// while a session is open without dismissing it. `pub(in crate::editor)`:
/// `orchestrate.rs` names `Invocation<MinibufSpan>` at every invoke call
/// site.
pub(in crate::editor) struct MinibufSpan {
    bytes: Range<usize>,
}

impl Invocation<MinibufSpan> {
    /// A `Minibuf`-target invocation. `bytes` is the whitespace-delimited
    /// argument the cursor is in — every minibuffer source's token rule,
    /// except a `NativeDelegated` one, which computes its own span
    /// synchronously before this is minted (`orchestrate.rs`'s
    /// `invoke_minibuf_source`).
    pub(in crate::editor) fn minibuf(bytes: Range<usize>) -> Self {
        Self::new(MinibufSpan { bytes })
    }
}

/// The `:` command-line completion session: the input every source saw,
/// restored verbatim before each cycle-apply (`orchestrate.rs`), so
/// applying candidate *k* over a slot's span is idempotent in these
/// coordinates — no "what did the previous candidate leave behind"
/// bookkeeping, and two sources with different spans coexist by
/// construction. No cross-source dedup (unlike `BufferSession`) — a
/// `Minibuf` session invokes exactly one source, so there is never a
/// second slot to dedup against.
pub(in crate::editor) struct MinibufSession {
    input: String,
    cursor: usize,
    /// [`super::BufferSession`]'s `menu_anchor` counterpart, a byte offset
    /// into `input`.
    menu_anchor: Option<usize>,
    core: SlotSet<MinibufSourceId, MinibufSpan>,
}

impl MinibufSession {
    /// A session over the `:` line's `input`, cursor at byte `cursor`.
    pub(in crate::editor) fn open(input: String, cursor: usize) -> Self {
        Self {
            input,
            cursor,
            menu_anchor: None,
            core: SlotSet::new(),
        }
    }

    pub(in crate::editor) fn input(&self) -> &str {
        &self.input
    }

    // ── Sources in and out ───────────────────────────────────────────────────

    pub(in crate::editor) fn invoke(
        &mut self,
        source: MinibufSourceId,
        invocation: Invocation<MinibufSpan>,
    ) -> u64 {
        self.core.invoke(source, invocation)
    }

    pub(in crate::editor) fn contribute(
        &mut self,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        self.core.contribute(id, items, incomplete)
    }

    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.core.is_pending()
    }

    /// See `BufferSession::drop_stalled_invocations`'s own doc — identical
    /// recovery, applied to this target's own slots.
    pub(in crate::editor) fn drop_stalled_invocations(&mut self) -> bool {
        self.core.drop_stalled()
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against `input[start..cursor]` — see
    /// `SlotSet::rank_with`'s own doc for the shared rank key. No `dedup`
    /// argument: this target has none.
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry) {
        let Self {
            input,
            cursor,
            menu_anchor,
            core,
        } = self;
        *menu_anchor = core.rank_with(sources, None, |inv| {
            let start = inv.span.bytes.start;
            Some((input[start..*cursor].to_owned(), start))
        });
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    pub(in crate::editor) fn len(&self) -> usize {
        self.core.len()
    }

    pub(in crate::editor) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        self.core.selected_item(idx)
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

    pub(in crate::editor) fn rows_in(&self, range: Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        self.core.rows_in(range)
    }

    /// Where the menu anchors — see `BufferSession::menu_anchor_char`'s
    /// own doc, a byte offset into [`Self::input`] here instead of a
    /// `CharOffset`.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        self.menu_anchor
    }

    /// The `:` line span the ranked candidate at `idx` replaces, with its
    /// `insert_text` — `None` for an unranked `idx`.
    pub(in crate::editor) fn selected_apply(&self, idx: usize) -> Option<(Range<usize>, &str)> {
        let (_, inv, item) = self.core.ranked(idx)?;
        Some((inv.span.bytes.clone(), item.insert_text()))
    }
}
