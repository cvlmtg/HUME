//! `BufferSession::accept`, the churn hotspot: applies the selected
//! candidate's `textEdit` (or a synthesized token-replacement fallback) at
//! every cursor, then best-effort `completionItem/resolve`.

use hume_editing::changeset::Assoc;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_rope::offset::CharOffset;

use hume_lsp::completion_item::parse_additional_text_edits_lenient;

use super::super::item::CompletionItem;
use super::buffer::{BufferSession, BufferSpan};
use super::contains_cursor;
use crate::editor::event::EditorEvent;
use crate::editor::lsp::{RustDelivery, RustResponder, edits};
use crate::editor::replay::Interactive;
use crate::editor::{EditorState, Severity};
use hume_lsp::position::checked_lsp_range_to_chars;
use hume_ops::edit::replace_span_around_cursors;

impl BufferSession {
    /// Applies the ranked candidate at `idx`'s `textEdit` (falling back to `insertText`
    /// over its own source's token when absent) at *every* cursor
    /// in the session's pane, as if the completion had been typed at each.
    /// A conforming server's completion range always contains the request
    /// position (LSP spec, `item.rs`'s `CompletionItem` doc), so the
    /// primary's own edit, re-expressed as a char count behind/ahead of its
    /// live head, is the same span typing would have consumed at any
    /// cursor. `additionalTextEdits` have no cursor of their own and are
    /// applied once, document-wide. Both land as one undo step, gen-checked
    /// against the last state this session reconciled.
    ///
    /// Every position here is decoded against the *selected item's own*
    /// invocation snapshot and mapped through the changes carried since it.
    /// A second source, or the same source re-invoked against a later
    /// document, has its own snapshot, so no contributor's positions are
    /// ever read against another's document.
    ///
    /// If the accepted item's own source declared `#:resolve #t`
    /// (`BufferSourceEntry::resolve`, a claim that its items are wire
    /// items from this buffer's attached server, not merely that a server
    /// happens to be attached) and the item lacks `additionalTextEdits`
    /// entirely (not just an empty array; see [`CompletionItem::
    /// has_additional_text_edits`]) and the server advertises
    /// `completionProvider.resolveProvider`, sends `completionItem/resolve`
    /// and applies whatever it returns once the response lands (via the
    /// same delivery and anchor machinery every other
    /// `lsp-request!` uses, dropped silently if the buffer has moved past
    /// the accept's own generation by then, same staleness discipline as
    /// any other LSP response). A selection-time resolve that already
    /// answered for this item against this same text is applied at once
    /// instead of asking again. Without the source's own flag, an item from
    /// a source that merely *shares* an LSP-attached buffer
    /// (`core:buffer-words`) would otherwise send the server an item it
    /// never produced. The server has no way to recognize it (most key a
    /// resolve lookup off an opaque `data` field only their own items
    /// carry), so the response is unspecified best case and a spurious
    /// error every accept worst case.
    pub(in crate::editor) fn accept(
        &self,
        state: &mut EditorState,
        view: &EngineView,
        idx: usize,
        _input: Interactive,
    ) -> Result<(), String> {
        // The session was taken off the stack, leaving the Insert layer it
        // completes in, whose session the edits compose into.
        let insert = state
            .input
            .insert_session()
            .expect("a buffer completion session is accepted over its Insert session");
        let (pid, bid) = (insert.pane(), insert.buffer());
        let AcceptPlan {
            item,
            may_resolve,
            encoding,
            rope_pre,
            edits,
        } = self.plan(state, (pid, bid), idx)?;
        let accept_cs = self.apply(state, view, (pid, bid), edits)?;

        // Fire on-completion-accept with the raw (pristine) item after the
        // edit lands: an extension point for anything this store doesn't
        // parse (e.g. `command`); Rust owns additionalTextEdits/resolve. A
        // label-only item (`plain()`'s own constructor, `core:buffer-words`'
        // bare-string answers) has no real wire payload to hand over, and
        // `{"label": …}` is a more useful hook payload than a bare `null`.
        let hook_item = match &item.raw {
            Some(raw) => raw.clone(),
            None => {
                hume_scripting::json::JsonHandle::new(serde_json::json!({"label": &item.label}))
            }
        };
        state.queue_event(EditorEvent::OnCompletionAccept {
            target: hume_scripting::PaneHandle::with_pane(bid, pid),
            item: hook_item,
        });

        if may_resolve && !item.has_additional_text_edits {
            let base = ResolveBase {
                pid,
                bid,
                rope_pre,
                accept_cs,
                encoding,
            };
            match item.resolved_reply_for(self.version) {
                Some(reply) => base.apply(state, view, reply),
                None => base.request(state, item),
            }
        }
        Ok(())
    }

    /// Every check and computation `accept` needs, against an `EditorState`
    /// it cannot change: an accept that fails any check leaves the buffer
    /// untouched.
    fn plan<'a>(
        &'a self,
        state: &EditorState,
        (pid, bid): (PaneId, BufferId),
        idx: usize,
    ) -> Result<AcceptPlan<'a>, String> {
        let (source, invocation, item) = self
            .ranked(idx)
            .ok_or_else(|| "completion-accept!: index out of range".to_string())?;
        let BufferSpan { doc, start, .. } = &invocation.span;
        let may_resolve = state.config.completion_sources.buffer_get(source).resolve;
        let buf = edits::checked_buffer(state, bid, None)?;
        if buf.text().version() != self.version {
            return Err(
                "completion-accept!: buffer changed since the completion was computed".to_string(),
            );
        }
        let encoding = item.wire_encoding(may_resolve, "completion-accept!")?;

        let (head_now, heads_now) = {
            let pbs = state.panes.entries.seeded(pid, bid);
            // The "as if typed at each cursor" model has no meaning for a
            // real selection. Typing over one is a different edit than
            // completing at it, and `replace_*_cursors` force-collapses
            // every selection it touches, which would silently discard a
            // real selection set.
            let sels = pbs.view(state.buffers.get(bid).text());
            if !sels.all_cursors() {
                return Err("completion-accept!: selections must be collapsed".to_string());
            }
            // Every cursor's own head, not just the primary's: the overlap
            // check below (span is uniform, but each cursor's own live head
            // differs) needs every one of them to catch an `additionalTextEdits`
            // insertion landing inside a *non-primary* cursor's own span.
            let heads_now: Vec<CharOffset> = sels.iter().map(|s| s.head().offset()).collect();
            (sels.primary().head(), heads_now)
        };

        // Both arms below produce a `(start_now, end_now)` pair in today's
        // live coordinates and a label for the containment error. The
        // containment check and the `(back, forward)` distance it licenses
        // are then shared by both, just after the match.
        let (start_now, end_now, new_text, what) = match &item.text_edit {
            Some(te) => {
                let range = checked_lsp_range_to_chars(&doc.rope, &te.range, encoding)
                    .ok_or_else(|| edits::past_document_end(&te.range, &doc.rope))?;
                let (start_b, end_b) = (range.start, range.end);
                if end_b < start_b {
                    return Err(format!(
                        "text edit has a reversed range (end {end_b:?} before start {start_b:?})"
                    ));
                }
                // Decoded once against the frozen request-time snapshot
                // above, then mapped forward through every edit this
                // invocation carried (`Assoc::Before` on the start
                // so it stays pinned to the token even if a carried
                // insertion landed exactly there; `Assoc::After` on the end
                // so a carried insertion at or inside the range extends it
                // rather than being left stranded next to the completion
                // text): exact position tracking through the intervening
                // keystrokes, not a scalar-drift guess. Two single-position
                // maps, not `map_ranges`: that helper hardcodes both ends to
                // *shrink* on a boundary insertion, which is the wrong
                // association for the end here.
                let mut start_pos = [start_b];
                doc.cs_since.map_positions(&mut start_pos, Assoc::Before);
                let mut end_pos = [end_b];
                doc.cs_since.map_positions(&mut end_pos, Assoc::After);
                (
                    start_pos[0],
                    end_pos[0],
                    te.new_text.clone(),
                    "textEdit range",
                )
            }
            // No server-provided range: replace the token from this
            // source's carried start to the cursor, as if the completion had
            // been typed at each cursor. Any prefix typed *before*
            // triggering completion (e.g. "fo" before the popup opened) is
            // otherwise left untouched, duplicating it ahead of
            // `insert_text`.
            None => (
                *start,
                head_now.offset(),
                item.insert_text.clone(),
                "insertText token",
            ),
        };
        // The delta model below rests entirely on this containment: a
        // conforming server's completion range always contains the request
        // position (LSP spec), and the `insertText` fallback's span ends at
        // the cursor, so it holds unless the cursor sits before the carried
        // start (`reconcile` drops such a slot). A cursor outside a server's
        // span breaks that assumption. Erroring here, buffer untouched, is
        // safer than quietly computing a span from a stale reference
        // point. This also keeps the `chars_since` calls below from
        // tripping their inversion assert.
        if !contains_cursor(&(start_now..end_now), head_now.offset()) {
            return Err(format!(
                "completion-accept!: {what} does not contain the cursor"
            ));
        }
        let (back, forward) = (
            head_now.offset().chars_since(start_now),
            end_now.chars_since(head_now.offset()),
        );

        // Captured before any edit lands: a resolve response (if `accept`
        // ends up sending one) is computed against this exact pre-accept document,
        // and its wire positions must be decoded against it, not whatever
        // the buffer holds once the response actually arrives.
        let rope_pre = state.buffers.get(bid).text().rope().clone();

        // Decoded and mapped here (pure, no mutation yet) so an overlap
        // with the main edit's own range (checked just below) can be caught
        // before either lands.
        let additional_char_edits = edits::build_edits_from_earlier_document(
            &doc.rope,
            &doc.cs_since,
            encoding,
            &item.additional_text_edits,
        )?;
        // Each cursor's own `back` distance, computed once, now, against
        // the pre-edit document, and applied as a plain char *count* from
        // here on rather than a live re-scan: `additionalTextEdits` land
        // document-wide before the per-cursor replacement (in `apply`), and
        // a `start_of` that re-scanned `text` fresh at that later point
        // would risk wandering into text that very additionalTextEdits
        // insertion just added, rather than staying immune to the shift the
        // way a fixed count is (see `replace_span_around_cursors`'s own
        // doc). A server-provided `textEdit` reuses the same `back` at
        // every cursor (LSP spec: the server's range always contains the
        // request position); the `insertText` fallback has no such
        // guarantee for a non-primary cursor, so each gets its own
        // token-start scan from its own head instead.
        let per_cursor_back: Vec<usize> = match &item.text_edit {
            Some(_) => heads_now.iter().map(|_| back).collect(),
            None => {
                let live_text = state.buffers.get(bid).text();
                let base = state.buffers.get(bid).overrides.word_chars(&state.settings);
                let entry = state.config.completion_sources.buffer_get(source);
                heads_now
                    .iter()
                    .map(|&head| head.chars_since(entry.token_start(live_text, head, base)))
                    .collect()
            }
        };
        // Applied at *every* cursor, so this check runs per cursor too, not
        // just the primary's, and protects the `insertText` fallback as
        // well as a server-provided `textEdit` range.
        let overlaps = heads_now
            .iter()
            .zip(&per_cursor_back)
            .any(|(&head, &back)| {
                let start_now = head.retreat_saturating(back);
                let end_now = head.shift(forward as isize);
                additional_char_edits.iter().any(|(r, _)| {
                    // The half-open overlap test alone (`s < end_now && start_now
                    // < e`) misses a *zero-width* additional edit sitting
                    // exactly at `head` (equivalently `end_now` when `forward ==
                    // 0`, the only case where the two coincide, and the only
                    // one this can reach: an edit strictly ahead of `head` leaves
                    // `head` itself untouched by `Assoc::After`, so it can never
                    // land inside this cursor's own retreat): the header inserts
                    // before the cursor edit lands, so carrying the selections
                    // through it (`SelectionSet::translate`, `Assoc::After` on
                    // heads) walks the live head
                    // past the inserted text. The cursor edit's own retreat
                    // then eats that inserted text instead of the span the
                    // server asked for. Guarded on `start_now < head`: a
                    // zero-width completion span (a pure insert, or an
                    // `insertText`-fallback cursor with no preceding word char
                    // at all) never retreats into anything, so flagging it here
                    // would reject the ordinary "accept immediately after the
                    // trigger char" shape whenever a source also sends an
                    // `additionalTextEdits` insertion at that same point. An
                    // insertion at `start_now` is safe (it shifts the whole span
                    // uniformly ahead of the edit) and stays excluded.
                    (r.start < end_now && start_now < r.end)
                        || (start_now < head && r.start == r.end && r.start == head)
                })
            });
        if overlaps {
            return Err(
                "completion-accept!: replacement span overlaps additionalTextEdits".to_string(),
            );
        }

        Ok(AcceptPlan {
            item,
            may_resolve,
            encoding,
            rope_pre,
            edits: AcceptEdits {
                additional: additional_char_edits,
                per_cursor_back,
                forward,
                new_text,
            },
        })
    }

    /// Lands `edits` in the open Insert session's group: `additionalTextEdits`
    /// first, then the per-cursor replacement. Returns the composed
    /// pre-accept → post-accept transform.
    fn apply(
        &self,
        state: &mut EditorState,
        view: &EngineView,
        (pid, bid): (PaneId, BufferId),
        edits: AcceptEdits<'_>,
    ) -> Result<hume_editing::changeset::ChangeSet, String> {
        let AcceptEdits {
            additional,
            per_cursor_back,
            forward,
            new_text,
        } = edits;
        // additionalTextEdits have no cursor of their own (document-level),
        // applied first so the cursor edit below reads live selections
        // already shifted across them, not the pre-edit positions.
        //
        // Validation (overlap/reversed-range checks) already ran in `plan`,
        // so a rejected batch here means the *in-batch* overlap check inside
        // `commit_char_edits` fired, with the buffer still untouched.
        // `commit_char_edits` is a no-op `Ok(None)` for an empty batch, so no
        // separate `is_empty()` branch is needed here.
        let cs_additional = edits::commit_char_edits(state, &view.panes, pid, bid, additional)?;

        // `forward` chars ahead of each cursor's own *live* head, plus
        // `per_cursor_back[i]` chars behind it, travels forward through
        // `additionalTextEdits` for free: `apply_doc_edit_grouped` below
        // reads selections `commit_char_edits` above already shifted across
        // those edits (`SelectionSet::translate`, `Assoc::After` on heads), and
        // `per_cursor_back` is a fixed count from *before* either edit
        // landed, immune to that shift the way a live re-scan of the
        // (already-shifted) text wouldn't be (see `per_cursor_back`'s own
        // doc, and `replace_span_around_cursors`'s), so `back`/`forward`
        // chars behind/ahead of the live head is already the right span at
        // every cursor.
        //
        // This is also the edit that grows an open Insert session's
        // typed-run selection to cover the whole replacement, not just what
        // was keyed since: if the accepted item's `textEdit` replaces text
        // typed before the Insert session began (e.g. `A` mid-identifier,
        // type one char, then accept), the selected typed run on Esc grows
        // to cover the whole replacement. That's intended, not a
        // pin-tracking bug: the accept's own edit rewrote that whole span,
        // so every character in it was written by this session, and
        // selecting the freshly completed token is the useful outcome.
        let (buffers, mut stores) = state.edit_parts();
        let cs_cursors = crate::editor::doc_ops::apply_doc_edit_grouped(
            buffers,
            &mut stores,
            pid,
            bid,
            move |s| {
                replace_span_around_cursors(
                    s,
                    move |_text, i, head| head.retreat_saturating(per_cursor_back[i]),
                    forward,
                    &new_text,
                )
            },
        )
        .map_err(|e| e.message().to_string())?;

        // The full pre-accept-document → post-accept-document transform:
        // `ResolveBase::apply` needs it composed, not just the cursor edit's
        // own half, to map a resolve response's positions forward correctly.
        Ok(match cs_additional {
            Some(cs_a) => cs_a.compose(cs_cursors),
            None => cs_cursors,
        })
    }
}

/// The accept a resolve answer's `additionalTextEdits` land on top of: the
/// pre-accept document their positions decode against, and the accept's own
/// change to map them through.
struct ResolveBase {
    pid: PaneId,
    bid: BufferId,
    rope_pre: ropey::Rope,
    accept_cs: hume_editing::changeset::ChangeSet,
    encoding: hume_rope::position_encoding::PositionEncoding,
}

impl ResolveBase {
    /// Sends `completionItem/resolve` to the server the item came from, when
    /// that server is still attached to the buffer, running, and advertised
    /// `completionProvider.resolveProvider`, and applies the answer's
    /// `additionalTextEdits` on top of `self`. Best-effort: a resolution
    /// error, timeout, or a server that's gone by send time only logs, it
    /// never fails the accept that already landed. `self` rides along in the
    /// response callback rather than being re-read from live focus when the
    /// response arrives, so a focus change while the request is in flight
    /// can't redirect the resolved edit to the wrong pane.
    fn request(self, state: &mut EditorState, item: &CompletionItem) {
        // A `#:resolve #t` source's items are always real LSP items in
        // practice, so the target is always there, but resolving is
        // best-effort by this function's own contract, so a plain item (no
        // wire payload to resolve against) is skipped rather than assumed
        // impossible.
        let Some((server_id, raw)) = item.resolve_target() else {
            return;
        };
        let bid = self.bid;
        let respond: RustResponder = Box::new(move |state, view, result| match result {
            Ok(resolved) => self.apply(state, view, &resolved),
            Err(message) => {
                state.report(
                    Severity::Error,
                    format!("lsp completion resolve: {message}"),
                );
            }
        });
        state.lsp_request_json(
            bid,
            server_id,
            "completionItem/resolve",
            raw.value().clone(),
            RustDelivery::default(),
            respond,
        );
    }

    fn apply(&self, state: &mut EditorState, view: &EngineView, resolved: &serde_json::Value) {
        let resolved_edits = parse_additional_text_edits_lenient(resolved);
        let result = edits::build_edits_from_earlier_document(
            &self.rope_pre,
            &self.accept_cs,
            self.encoding,
            &resolved_edits,
        )
        .and_then(|char_edits| {
            edits::commit_char_edits(state, &view.panes, self.pid, self.bid, char_edits)
        });
        if let Err(e) = result {
            state.report(Severity::Error, format!("lsp completion resolve: {e}"));
        }
    }
}

/// What [`BufferSession::plan`] hands `accept`: the item and everything
/// computed from it before any edit lands.
struct AcceptPlan<'a> {
    item: &'a CompletionItem,
    may_resolve: bool,
    encoding: hume_rope::position_encoding::PositionEncoding,
    /// The pre-accept document a resolve response's positions decode
    /// against.
    rope_pre: ropey::Rope,
    edits: AcceptEdits<'a>,
}

/// The edits [`BufferSession::apply`] lands.
struct AcceptEdits<'a> {
    additional: Vec<(hume_rope::offset::ExclusiveRange<CharOffset>, &'a str)>,
    /// Chars behind each cursor's head the replacement starts at, by
    /// selection index.
    per_cursor_back: Vec<usize>,
    forward: usize,
    new_text: String,
}
