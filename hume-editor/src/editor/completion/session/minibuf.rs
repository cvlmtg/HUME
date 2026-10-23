//! The minibuffer completion session (the `:` and `|` lines):
//! [`MinibufSession`]. Tab and Shift-Tab pick a row and immediately splice
//! that candidate into the minibuffer, with no separate accept step; before
//! any row is picked, Tab splices the candidates' common prefix instead. A
//! source may answer in parts (`#:more`): each part replaces the last, and
//! a picked candidate stays picked across them.
//! The popup's own key handler (`input_stack/completion.rs`) dismisses on
//! any other key, unlike [`super::BufferSession`], which refilters in place
//! as the user types.

use std::borrow::Cow;
use std::ops::Range;

use hume_rope::grapheme::common_str_prefix;
use hume_scripting::SteelCall;
use hume_scripting::host::{AnswerEnd, EmittedWord, FirstTab};

use super::super::item::{CandidateKey, CompletionItem};
use super::super::registry::{MinibufSourceId, SourceRegistry};
use super::MatchKind;
use super::slots::{Invocation, SlotSet, SlotTokens};

/// Byte range in [`MinibufSession`]'s own `input` that a `Minibuf`
/// invocation answered for. Never moves: nothing can edit the line while
/// a session is open without dismissing it. `pub(in crate::editor)`:
/// `orchestrate.rs` names `Invocation<MinibufSpan>` at every invoke call
/// site.
pub(in crate::editor) struct MinibufSpan {
    bytes: Range<usize>,
    /// What the first Tab puts over `bytes` while no row is picked.
    prefix: FirstTab,
}

impl Invocation<MinibufSpan> {
    /// A `Minibuf`-target invocation. `bytes` is the whitespace-delimited
    /// argument the cursor is in, every minibuffer source's token rule,
    /// except a `NativeDelegated` one, which computes its own span
    /// synchronously before this is minted (`orchestrate.rs`'s
    /// `invoke_minibuf_source`), and a delegated Steel one, whose answer
    /// may name its span ([`MinibufSession::emit`]).
    pub(in crate::editor) fn minibuf(bytes: Range<usize>) -> Self {
        Self::new(MinibufSpan {
            bytes,
            prefix: FirstTab::Common,
        })
    }
}

/// The minibuffer completion session: the input every source saw,
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
    /// The picked candidate, when a further part of the answer it was
    /// picked from has landed since the last [`Self::rank`], which picks it
    /// again wherever it ranks.
    kept_pick: Option<CandidateKey>,
    core: SlotSet<MinibufSourceId, MinibufSpan>,
}

impl MinibufSession {
    /// A session over a minibuffer line's `input`, cursor at byte `cursor`.
    pub(in crate::editor) fn open(input: String, cursor: usize) -> Self {
        Self {
            input,
            cursor,
            menu_anchor: None,
            picked: false,
            kept_pick: None,
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

    /// Lands a native source's answer for invocation `id` over the span the
    /// invocation was minted with.
    pub(in crate::editor) fn contribute(
        &mut self,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        self.core
            .contribute(id, items, incomplete, AnswerEnd::Final, |_, _, _| {})
    }

    /// Lands a Steel source's answer for invocation `id`, completing the
    /// word it names: `word.span`, a char range of [`Self::input`] (`None`
    /// keeps the invocation's own), and `word.prefix`, what the first Tab
    /// puts over it. A [`FirstTab::Named`] text has a syntax the session
    /// cannot read, so it is applied as given: the typed text and the
    /// prefix need not share a display form (`dir/a` and `'dir/a b` read as
    /// the same path start in PowerShell). `Ok(false)` when no slot still
    /// owes an answer to `id`. `Err` for a word from a source the editor matches for
    /// (it filters that source's items, so the set a prefix describes is
    /// not the source's), and for a span that does not hold the cursor
    /// within the line. `end` says whether more of the answer follows.
    pub(in crate::editor) fn emit(
        &mut self,
        registry: &SourceRegistry,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
        end: AnswerEnd,
        word: EmittedWord,
    ) -> Result<bool, String> {
        let kept = self.streamed_pick(id);
        let named = if word == EmittedWord::default() {
            None
        } else {
            let Some(source) = self.core.source_of(id) else {
                return Ok(false);
            };
            if registry.minibuf_get(source).match_kind != MatchKind::Delegated {
                return Err(
                    "completion-emit!: #:span and #:prefix need a source registered with \
                     #:match 'delegated"
                        .to_string(),
                );
            }
            let EmittedWord { span, prefix } = word;
            Some((
                span.map(|chars| self.span_bytes(chars)).transpose()?,
                prefix,
            ))
        };
        let landed = self
            .core
            .contribute(id, items, incomplete, end, |_, minted, _| {
                if let Some((bytes, prefix)) = named {
                    if let Some(bytes) = bytes {
                        minted.bytes = bytes;
                    }
                    minted.prefix = prefix;
                }
            });
        if landed {
            self.kept_pick = kept;
        }
        Ok(landed)
    }

    /// The picked candidate, when an answer to `id` continues the answer it
    /// was picked from.
    fn streamed_pick(&self, id: u64) -> Option<CandidateKey> {
        if !self.picked || !self.core.continues_stream(id) {
            return None;
        }
        Some(
            self.core
                .selected_item(self.core.selected())?
                .candidate_key(),
        )
    }

    /// Char range `chars` of [`Self::input`] as bytes, `Err` unless it
    /// holds the cursor within the line.
    fn span_bytes(&self, chars: Range<usize>) -> Result<Range<usize>, String> {
        let input = &self.input;
        let byte_at = |n: usize| {
            input
                .char_indices()
                .map(|(i, _)| i)
                .chain([input.len()])
                .nth(n)
        };
        let cursor = input[..self.cursor].chars().count();
        match (byte_at(chars.start), byte_at(chars.end)) {
            (Some(start), Some(end)) if chars.start <= cursor && cursor <= chars.end => {
                Ok(start..end)
            }
            _ => Err(format!(
                "completion-emit!: #:span from char {} to {} must hold the cursor ({cursor}) \
                 within the line's {} chars",
                chars.start,
                chars.end,
                input.chars().count()
            )),
        }
    }

    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.core.is_pending()
    }

    /// See `BufferSession::end_request`.
    pub(in crate::editor) fn end_request(&mut self, id: u64) -> bool {
        self.core.end_request(id)
    }

    /// See `BufferSession::take_cancels`.
    pub(in crate::editor) fn take_cancels(&mut self, sources: &SourceRegistry) -> Vec<SteelCall> {
        self.core.take_cancels(sources)
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    /// Re-scores every shown item against `input[start..cursor]`; see
    /// `SlotSet::rank_with`'s own doc for the shared rank key. No `dedup`
    /// argument: this target has none. Nothing is picked afterwards except
    /// a pick kept across the parts of an answer whose candidate still ranks.
    pub(in crate::editor) fn rank(&mut self, sources: &SourceRegistry) {
        let Self {
            input,
            cursor,
            menu_anchor,
            picked,
            kept_pick,
            core,
        } = self;
        *menu_anchor = core.rank_with(sources, None, |inv| {
            let start = inv.span.bytes.start;
            Some(SlotTokens::single(input[start..*cursor].to_owned(), start))
        });
        *picked = kept_pick
            .take()
            .is_some_and(|key| core.select_where(|item| key.matches(item)));
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

    /// Rows always carry `detail`: the minibuffer has no documentation box.
    pub(in crate::editor) fn rows_in(&self, range: Range<usize>) -> Vec<hume_ui::popup::MenuRow> {
        self.core.rows_in(range, true)
    }

    /// Where the menu anchors; see `BufferSession::menu_anchor`'s
    /// own doc, a byte offset into [`Self::input`] here instead of a
    /// `CharOffset`.
    pub(in crate::editor) fn menu_anchor_byte(&self) -> Option<usize> {
        self.menu_anchor
    }

    /// The edit Tab applies: the picked candidate over its span, or with
    /// nothing picked, what the answer's [`FirstTab`] says over the span.
    /// `None` when there is nothing to apply: a [`FirstTab::Nothing`], a
    /// named prefix with the cursor before the span's end (the span's text
    /// after the cursor is not something the source read), or a common
    /// prefix that does not extend what is already typed (a fuzzy match can
    /// share less than the typed text). The typed text is the whole token,
    /// so with the cursor mid-token the common prefix rarely extends it and
    /// the first Tab only opens the popup.
    pub(in crate::editor) fn proposed_apply(&self) -> Option<(Range<usize>, Cow<'_, str>)> {
        if self.picked {
            let (_, inv, item) = self.core.ranked(self.core.selected())?;
            return Some((inv.span.bytes.clone(), Cow::Borrowed(item.insert_text())));
        }
        let (_, inv, first) = self.core.ranked(0)?;
        let span = inv.span.bytes.clone();
        match &inv.span.prefix {
            FirstTab::Nothing => None,
            FirstTab::Named(prefix) => {
                (self.cursor == span.end).then_some((span, Cow::Borrowed(prefix.as_str())))
            }
            FirstTab::Common => {
                let typed = &self.input[span.clone()];
                let prefix = (1..self.core.len())
                    .filter_map(|i| self.core.selected_item(i))
                    .fold(first.insert_text(), |p, item| {
                        common_str_prefix(p, item.insert_text())
                    });
                let extends =
                    prefix.len() > typed.len() && super::prefix_matches(prefix, typed, false);
                extends.then_some((span, Cow::Borrowed(prefix)))
            }
        }
    }
}

impl super::EndingSession for MinibufSession {
    fn end_requests(&mut self, sources: &SourceRegistry) -> Vec<SteelCall> {
        self.core.end_requests(sources)
    }
}
