//! The `:` command-line completion session: [`MinibufSession`]. Tab and
//! Shift-Tab pick a row and immediately splice that candidate into the
//! minibuffer, with no separate accept step; before any row is picked, Tab
//! splices the candidates' common prefix instead. The popup's own key
//! handler (`input_stack/completion.rs`) dismisses on any other key, unlike
//! [`super::BufferSession`], which refilters in place as the user types.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::super::item::CompletionItem;
use super::super::registry::{MinibufSourceId, SourceRegistry};
use super::slots::{Invocation, SlotSet, SlotTokens};

/// Byte range in [`MinibufSession`]'s own `input` that a `Minibuf`
/// invocation answered for. Never moves: nothing can edit the `:` line
/// while a session is open without dismissing it. `pub(in crate::editor)`:
/// `orchestrate.rs` names `Invocation<MinibufSpan>` at every invoke call
/// site.
pub(in crate::editor) struct MinibufSpan {
    bytes: Range<usize>,
}

impl Invocation<MinibufSpan> {
    /// A `Minibuf`-target invocation. `bytes` is the whitespace-delimited
    /// argument the cursor is in, every minibuffer source's token rule,
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
/// coordinates: no "what did the previous candidate leave behind"
/// bookkeeping, and two sources with different spans coexist by
/// construction. No cross-source dedup (unlike `BufferSession`): a
/// `Minibuf` session invokes exactly one source, so there is never a
/// second slot to dedup against.
pub(in crate::editor) struct MinibufSession {
    input: String,
    cursor: usize,
    /// [`super::BufferSession`]'s `menu_anchor` counterpart, a byte offset
    /// into `input`.
    menu_anchor: Option<usize>,
    /// Whether the user (or the eager policy) has picked a row since the
    /// last [`Self::rank`]. Until then nothing is highlighted and Tab
    /// proposes the candidates' common prefix instead of a candidate.
    picked: bool,
    core: SlotSet<MinibufSourceId, MinibufSpan>,
}

/// The longest run of whole grapheme clusters `a` and `b` start with.
fn common_prefix<'a>(a: &'a str, b: &str) -> &'a str {
    let len = a
        .grapheme_indices(true)
        .zip(b.graphemes(true))
        .take_while(|((_, x), y)| x == y)
        .last()
        .map_or(0, |((i, x), _)| i + x.len());
    &a[..len]
}

impl MinibufSession {
    /// A session over the `:` line's `input`, cursor at byte `cursor`.
    pub(in crate::editor) fn open(input: String, cursor: usize) -> Self {
        Self {
            input,
            cursor,
            menu_anchor: None,
            picked: false,
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
        self.core.contribute(id, items, incomplete, |_, _, _| {})
    }

    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.core.is_pending()
    }

    /// See `BufferSession::drop_stalled_invocations`'s own doc; identical
    /// recovery, applied to this target's own slots.
    pub(in crate::editor) fn drop_stalled_invocations(&mut self) -> bool {
        self.core.drop_stalled()
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against `input[start..cursor]`; see
    /// `SlotSet::rank_with`'s own doc for the shared rank key. No `dedup`
    /// argument: this target has none.
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry) {
        let Self {
            input,
            cursor,
            menu_anchor,
            picked,
            core,
        } = self;
        *picked = false;
        *menu_anchor = core.rank_with(sources, None, |inv| {
            let start = inv.span.bytes.start;
            Some(SlotTokens::single(input[start..*cursor].to_owned(), start))
        });
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    pub(in crate::editor) fn len(&self) -> usize {
        self.core.len()
    }

    pub(in crate::editor) fn selected_item(&self, idx: usize) -> Option<&CompletionItem> {
        self.core.selected_item(idx)
    }

    /// The highlighted ranked row, `None` until one is picked.
    pub(in crate::editor) fn selected(&self) -> Option<usize> {
        self.picked.then(|| self.core.selected())
    }

    /// Moves the selection one row, wrapping. The first step after a rank
    /// picks a row instead: row 0 going forward, the last row going back.
    /// `false` on an empty list.
    pub(in crate::editor) fn step_selection(&mut self, forward: bool) -> bool {
        if self.picked {
            return self.core.step_selection(forward);
        }
        if self.core.is_empty() {
            return false;
        }
        self.picked = true;
        forward || self.core.step_selection(false)
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

    /// Where the menu anchors; see `BufferSession::menu_anchor`'s
    /// own doc, a byte offset into [`Self::input`] here instead of a
    /// `CharOffset`.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        self.menu_anchor
    }

    /// The edit Tab applies: the picked candidate over its span, or with
    /// nothing picked, the candidates' common prefix over the span. `None`
    /// when there is nothing to apply, including a common prefix that does
    /// not extend what is already typed (a fuzzy match can share less than
    /// the typed text). The typed text is the whole token, so with the
    /// cursor mid-token the prefix rarely extends it and the first Tab only
    /// opens the popup.
    pub(in crate::editor) fn proposed_apply(&self) -> Option<(Range<usize>, Cow<'_, str>)> {
        if self.picked {
            let (_, inv, item) = self.core.ranked(self.core.selected())?;
            return Some((inv.span.bytes.clone(), Cow::Borrowed(item.insert_text())));
        }
        let (_, inv, first) = self.core.ranked(0)?;
        let span = inv.span.bytes.clone();
        let prefix = (1..self.core.len())
            .filter_map(|i| self.core.selected_item(i))
            .fold(first.insert_text(), |p, item| {
                common_prefix(p, item.insert_text())
            });
        let typed = &self.input[span.clone()];
        let extends = prefix.len() > typed.len() && super::prefix_matches(prefix, typed, false);
        extends.then_some((span, Cow::Borrowed(prefix)))
    }
}
