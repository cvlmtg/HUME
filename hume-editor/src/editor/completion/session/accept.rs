//! `BufferSession::accept` — the churn hotspot: applies the selected
//! candidate's `textEdit` (or a synthesized token-replacement fallback) at
//! every cursor, then best-effort `completionItem/resolve`.

use hume_editing::changeset::Assoc;
use hume_engine::pipeline::EngineView;
use hume_rope::offset::CharOffset;

use hume_lsp::completion_item::parse_additional_text_edits_lenient;

use super::super::item::CompletionItem;
use super::buffer::{BufferSession, BufferSpan};
use super::contains_cursor;
use crate::editor::event::EditorEvent;
use crate::editor::lsp::{
    LspCallback, LspState, ResponseAnchor, edits, introspect, wire_range_to_chars,
};
use crate::editor::{EditorState, Severity};
use hume_ops::edit::{replace_around_cursors, replace_span_around_cursors, word_start_before};

impl BufferSession {
    /// Applies the ranked candidate at `idx`'s `textEdit` (falling back to `insertText`
    /// over its own source's live token span when absent) at *every* cursor
    /// in the session's pane, as if the completion had been typed at each —
    /// a conforming server's completion range always contains the request
    /// position (LSP spec, `item.rs`'s `CompletionItem` doc), so the
    /// primary's own edit, re-expressed as a char count behind/ahead of its
    /// live head, is the same span typing would have consumed at any
    /// cursor. `additionalTextEdits` have no cursor of their own and are
    /// applied once, document-wide. Both land as one undo step — gen-checked
    /// against the last edit this session observed.
    ///
    /// Every position here is decoded against the *selected item's own*
    /// invocation snapshot and mapped through the edits observed since it —
    /// a second source, or the same source re-invoked against a later
    /// document, has its own snapshot, so no contributor's positions are
    /// ever read against another's document.
    ///
    /// If the accepted item's own source declared `#:resolve #t`
    /// (`BufferSourceEntry::resolve` — a claim that its items are wire
    /// items from this buffer's attached server, not merely that a server
    /// happens to be attached) and the item lacks `additionalTextEdits`
    /// entirely (not just an empty array — see [`CompletionItem::
    /// has_additional_text_edits`]) and the server advertises
    /// `completionProvider.resolveProvider`, sends `completionItem/resolve`
    /// and applies whatever it returns once the response lands (via the
    /// ordinary `LspCallback`/`stale_check` machinery every other
    /// `lsp-request` uses — dropped silently if the buffer has moved past
    /// the accept's own generation by then, same staleness discipline as
    /// any other LSP response). Without the source's own flag, an item from
    /// a source that merely *shares* an LSP-attached buffer
    /// (`core:buffer-words`) would otherwise send the server an item it
    /// never produced — the server has no way to recognize it (most key a
    /// resolve lookup off an opaque `data` field only their own items
    /// carry), so the response is unspecified best case and a spurious
    /// error every accept worst case.
    pub(in crate::editor) fn accept(
        &self,
        state: &mut EditorState,
        view: &EngineView,
        lsp: &mut LspState,
        idx: usize,
    ) -> Result<(), String> {
        let (source, invocation, item) = self
            .ranked(idx)
            .ok_or_else(|| "completion-accept!: index out of range".to_string())?;
        let BufferSpan { doc, live } = &invocation.span;
        let bid = self.bid();
        // Copied out now — a plain `bool`, so there's no reason to keep the
        // registry borrow (or `source`) alive across the `&mut state` uses
        // below just to read it again at the bottom.
        let may_resolve = state.config.completion_sources.buffer_get(source).resolve;
        edits::checked_buffer(state, bid, Some(self.generation))?;
        // A source that hasn't declared `#:resolve` isn't claiming to be
        // genuine LSP-server-origin (that flag's own doc), so its own
        // `textEdit`/`additionalTextEdits` have no wire encoding to honor.
        // `Utf32` (LSP 3.17's own third `PositionEncodingKind` — `character`
        // counts chars, never code units) is the spec-defined choice for
        // "no server involved." A `#:resolve` item reads its encoding off
        // its own `raw` response tag instead of the buffer's *currently*
        // attached server — the response may have been produced by a server
        // since restarted or detached, which could negotiate differently.
        let encoding = if may_resolve {
            match &item.raw {
                Some(raw) => raw.position_encoding("completion-accept!")?,
                // Not assumed impossible (same discipline as
                // `maybe_send_resolve`'s own `raw` check below) — a
                // #:resolve source's items are always real LSP items in
                // practice, but this function's contract doesn't get to
                // lean on "in practice."
                None => {
                    return Err(
                        "completion-accept!: a #:resolve source's item has no wire payload to \
                         decode positions from"
                            .to_string(),
                    );
                }
            }
        } else {
            hume_rope::position_encoding::PositionEncoding::Utf32
        };

        // The session's pane may no longer be live — the Steel
        // `completion-accept!` builtin firing from a different pane than the
        // session opened in. `pane_state::ensure`'s fallback (fabricate a
        // fresh cursor at char 0 for a pane that never showed this buffer)
        // is right for "a background buffer with no selection state yet",
        // not for "this session's own point of reference is gone" — so this
        // errors instead of silently landing the edit at the top of the file.
        if state.focus.id() != self.pane_id {
            return Err("completion-accept!: the session's pane is no longer focused".to_string());
        }
        // Focus alone doesn't prove the pane still *shows* this buffer in
        // general — but a session's own pane can only ever leave its buffer
        // via `switch_pane_to_buffer`, whose teardown (`focus::
        // end_focus_sessions`, run before its own `buffer_id` write) already
        // removes this session's `BufferCompletionLayer` first, since it
        // always sits above the `InsertLayer` that teardown truncates. Kept
        // as a real `Err`, not a debug-only assert: if that ordering ever
        // breaks (a new close/switch path, a plugin-driven pane retarget
        // that bypasses `switch_pane_to_buffer`), a release build must
        // refuse rather than silently edit a buffer this pane no longer
        // shows — the same discipline every other guard in this function
        // already follows.
        if view.panes.get(self.pane_id).map(|p| p.buffer_id) != Some(bid) {
            return Err(
                "completion-accept!: the session's pane no longer shows its buffer".to_string(),
            );
        }
        let pid = self.pane_id;
        let (head_now, heads_now) = {
            let pbs = state.panes.buffer_state(pid, bid).ok_or_else(|| {
                "completion-accept!: buffer is no longer shown in the session's pane".to_string()
            })?;
            // The "as if typed at each cursor" model has no meaning for a
            // real selection — typing over one is a different edit than
            // completing at it, and `replace_*_cursors` force-collapses
            // every selection it touches, which would silently discard a
            // real selection set.
            if !pbs.selections().all_collapsed() {
                return Err("completion-accept!: selections must be collapsed".to_string());
            }
            // Every cursor's own head, not just the primary's — the overlap
            // check below (span is uniform, but each cursor's own live head
            // differs) needs every one of them to catch an `additionalTextEdits`
            // insertion landing inside a *non-primary* cursor's own span.
            let heads_now: Vec<CharOffset> =
                pbs.selections().iter_sorted().map(|s| s.head()).collect();
            (pbs.selections().primary().head(), heads_now)
        };

        // Both arms below produce a `(start_now, end_now)` pair in today's
        // live coordinates and a label for the containment error — the
        // containment check and the `(back, forward)` distance it licenses
        // are then shared by both, just after the match.
        let (start_now, end_now, new_text, what) = match &item.text_edit {
            Some(te) => {
                let range = wire_range_to_chars(&doc.rope, &te.range, encoding);
                let (start_b, end_b) = (range.start, range.end);
                if end_b < start_b {
                    return Err(format!(
                        "text edit has a reversed range (end {end_b:?} before start {start_b:?})"
                    ));
                }
                // Decoded once against the frozen request-time snapshot
                // above, then mapped forward through every edit this
                // invocation actually observed (`Assoc::Before` on the start
                // so it stays pinned to the token even if an observed
                // insertion landed exactly there; `Assoc::After` on the end
                // so an observed insertion at or inside the range extends it
                // rather than being left stranded next to the completion
                // text) — exact position tracking through the intervening
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
            // No server-provided range: replace this source's own live
            // token uniformly at every cursor, same as the `textEdit` arm
            // just above — any prefix typed *before* triggering completion
            // (e.g. "fo" before the popup opened) is otherwise left
            // untouched, duplicating it ahead of `insert_text`. The token is
            // the source's own declared span (`registry.rs`'s token rule),
            // tracked through every keystroke since; there is no well-defined
            // *per-cursor* token independent of it to fall back to, so this
            // is the one span every cursor gets.
            None => (
                live.start,
                live.end,
                item.insert_text.clone(),
                "insertText token",
            ),
        };
        // The delta model below rests entirely on this containment: a
        // conforming server's completion range always contains the request
        // position (LSP spec), and the `insertText` fallback's token rests
        // on the same guarantee via the session's own tracking (`observe_
        // edit` drops a slot the cursor has left). A cursor that has since
        // moved outside the span by a path the session never saw breaks
        // that assumption — erroring here, buffer untouched, is safer than
        // silently computing a span from a stale reference point. This also
        // keeps the `chars_since` calls below from tripping their inversion
        // assert.
        if !contains_cursor(&(start_now..end_now), head_now) {
            return Err(format!(
                "completion-accept!: {what} does not contain the cursor"
            ));
        }
        let (back, forward) = (
            head_now.chars_since(start_now),
            end_now.chars_since(head_now),
        );

        // Captured before any edit lands — a resolve response (if one ends
        // up sent below) is computed against this exact pre-accept document,
        // and its wire positions must be decoded against it, not whatever
        // the buffer holds once the response actually arrives.
        let rope_pre = state.buffers.get(bid).text().rope().clone();

        // Decoded and mapped here (pure — no mutation yet) so an overlap
        // with the main edit's own range (checked just below) can be caught
        // before either lands.
        let additional_char_edits = edits::build_edits_from_earlier_document(
            &doc.rope,
            &doc.cs_since,
            encoding,
            &item.additional_text_edits,
        )?;
        // Each cursor's own `back` distance — computed once, now, against
        // the pre-edit document, and applied as a plain char *count* from
        // here on rather than a live re-scan: `additionalTextEdits` land
        // document-wide before the per-cursor replacement (just below), and
        // a `start_of` that re-scanned `text` fresh at that later point
        // would risk wandering into text that very additionalTextEdits
        // insertion just added, rather than staying immune to the shift the
        // way a fixed count is (see `replace_span_around_cursors`'s own
        // doc). A server-provided `textEdit` reuses the same `back` at
        // every cursor (LSP spec: the server's range always contains the
        // request position); the `insertText` fallback has no such
        // guarantee for a non-primary cursor, so each gets its own
        // `word_start_before` scan from its own head instead.
        let per_cursor_back: Vec<usize> = match &item.text_edit {
            Some(_) => heads_now.iter().map(|_| back).collect(),
            None => {
                let live_text = state.buffers.get(bid).text();
                let word_chars = crate::editor::commands::effective_word_chars(
                    state.buffers.get(bid),
                    &state.settings,
                );
                heads_now
                    .iter()
                    .map(|&head| head.chars_since(word_start_before(live_text, head, word_chars)))
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
                    // 0` — the only case where the two coincide, and the only
                    // one this can reach: an edit strictly ahead of `head` leaves
                    // `head` itself untouched by `Assoc::After`, so it can never
                    // land inside this cursor's own retreat): the header inserts
                    // before the cursor edit lands, so `translate_in_place`'s
                    // `Assoc::After` on selection heads
                    // (`hume-editing/src/selection/mod.rs`) walks the live head
                    // past the inserted text — the cursor edit's own retreat
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

        // Insert mode already has a group open on this exact (pane, buffer)
        // (composing this accept into the ongoing session); a Steel-
        // triggered accept outside Insert mode does not, so open one here —
        // both edits below then land as one undo step regardless of caller.
        // Checked by kind and owner, not `state.active_session.is_none()`:
        // a session open on a *different* (pane, buffer), or a non-Insert
        // one here (e.g. a Paste session from an unrelated `p` keypress),
        // is a real conflict — the fallible `begin_edit_group` call below
        // reports it as an `Err` rather than this function silently
        // treating "some session is open" as "the group I need is open."
        let opened_group = !state
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_insert_at(pid, bid));
        if opened_group {
            crate::editor::doc_ops::begin_edit_group(
                &state.buffers,
                &mut state.panes.state,
                &mut state.active_session,
                pid,
                bid,
            )
            .map_err(|e| e.message().to_owned())?;
        }

        // additionalTextEdits have no cursor of their own — document-level,
        // applied first so the cursor edit below reads live selections
        // already shifted across them, not the pre-edit positions.
        //
        // Validation (overlap/reversed-range checks) already ran above, so
        // a rejected batch here means the *in-batch* overlap check inside
        // `commit_char_edits` fired — the buffer is still untouched, but a
        // group opened just above would otherwise leak, still open and
        // empty, for the next edit to wrongly compose into. Commit it (a
        // no-op: `commit_edit_group` skips recording when nothing was ever
        // composed in) before propagating the error.
        // `commit_char_edits` is a no-op `Ok(None)` for an empty batch, so no
        // separate `is_empty()` branch is needed here.
        let cs_additional =
            match edits::commit_char_edits(state, &view.panes, pid, bid, additional_char_edits) {
                Ok(cs) => cs,
                Err(e) => {
                    if opened_group {
                        crate::editor::doc_ops::commit_edit_group(
                            &mut state.buffers,
                            &state.panes.state,
                            &mut state.active_session,
                        );
                    }
                    return Err(e);
                }
            };

        // `forward` chars ahead of each cursor's own *live* head, plus
        // `per_cursor_back[i]` chars behind it, travels forward through
        // `additionalTextEdits` for free: `apply_doc_edit_grouped` below
        // reads selections `commit_char_edits` above already shifted across
        // those edits (`translate_in_place`, `Assoc::After` on heads), and
        // `per_cursor_back` is a fixed count from *before* either edit
        // landed, immune to that shift the way a live re-scan of the
        // (already-shifted) text wouldn't be (see `per_cursor_back`'s own
        // doc, and `replace_span_around_cursors`'s) — so `back`/`forward`
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
        //
        // A server-provided `textEdit` (`per_cursor_back` uniformly
        // `back`) goes through the simpler, dedicated `replace_around_
        // cursors` rather than re-deriving the same uniform count through
        // `replace_span_around_cursors`'s more general per-cursor form.
        let cs_cursors = match &item.text_edit {
            Some(_) => crate::editor::doc_ops::apply_doc_edit_grouped(
                &mut state.buffers,
                &state.config.decorations,
                &mut state.panes.state,
                &mut state.panes.jumps,
                &mut state.active_session,
                pid,
                bid,
                move |b, s| replace_around_cursors(b, s, back, forward, &new_text),
            ),
            None => crate::editor::doc_ops::apply_doc_edit_grouped(
                &mut state.buffers,
                &state.config.decorations,
                &mut state.panes.state,
                &mut state.panes.jumps,
                &mut state.active_session,
                pid,
                bid,
                move |b, s| {
                    replace_span_around_cursors(
                        b,
                        s,
                        move |_text, i, head| head.retreat_saturating(per_cursor_back[i]),
                        forward,
                        &new_text,
                    )
                },
            ),
        };
        // Marks this dispatch (or completion accept — see `accept_completion_
        // selection`'s own doc) interactive: whatever it and the rest of the
        // Insert-key binding that called this do from here collapses into
        // one net edit, replayed directly, never by re-running any of it
        // (see `InsertInput::Result`'s own doc, and `DotCapture`'s).
        state.mark_dot_interactive();

        if opened_group {
            crate::editor::doc_ops::commit_edit_group(
                &mut state.buffers,
                &state.panes.state,
                &mut state.active_session,
            );
        }

        // The full pre-accept-document → post-accept-document transform —
        // `maybe_send_resolve` needs it composed, not just the cursor edit's
        // own half, to map a resolve response's positions forward correctly.
        let accept_cs = match cs_additional {
            Some(cs_a) => cs_a.compose(cs_cursors),
            None => cs_cursors,
        };

        // Fire on-completion-accept with the raw (pristine) item after the
        // edit lands — an extension point for anything this store doesn't
        // parse (e.g. `command`); Rust owns additionalTextEdits/resolve. A
        // label-only item (`plain()`'s own constructor, `core:buffer-words`'
        // bare-string answers) has no real wire payload to hand over —
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
            self.maybe_send_resolve(state, lsp, item, rope_pre, accept_cs, encoding);
        }
        Ok(())
    }

    /// Sends `completionItem/resolve` when the server advertised
    /// `completionProvider.resolveProvider` — best-effort: a resolution error,
    /// timeout, or a server that's gone by send time only logs, it never fails
    /// the accept that already landed. `self`'s own pane/buffer (already
    /// validated by `accept`, this method's only caller) rides along in the
    /// response callback's own closure rather than being re-read from live focus
    /// when the response arrives, so a focus change while the request is in
    /// flight can't redirect the resolved edit to the wrong pane.
    fn maybe_send_resolve(
        &self,
        state: &mut EditorState,
        lsp: &mut LspState,
        item: &CompletionItem,
        rope_pre: ropey::Rope,
        accept_cs: hume_editing::changeset::ChangeSet,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) {
        let (pid, bid) = (self.pane_id, self.bid());
        let Some(server_id) = state.buffers.try_get(bid).and_then(|b| b.lsp_server) else {
            return;
        };
        if !introspect::completion_resolve_provider(lsp, server_id) {
            return;
        }
        // A `#:resolve #t` source's items are always real LSP items in
        // practice, so `raw` is always `Some` here — but resolving is
        // best-effort by this function's own contract, so a plain item (no
        // wire payload to resolve against) is skipped rather than assumed
        // impossible.
        let Some(raw) = &item.raw else {
            return;
        };

        // Same discipline `lsp-request` itself uses (bridge.rs): a request
        // minted here must not reach the wire ahead of the didChange
        // describing the edit `accept` just applied.
        crate::editor::lsp::sync::flush_lsp_pending_changes(state, lsp);
        let timeout_ms = state.settings.lsp_request_timeout_ms as u64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let meta = hume_lsp::client::RequestMeta {
            method: "completionItem/resolve".to_string(),
            deadline,
        };
        let gen_after = state.buffers.get(bid).text_gen;
        let Some(id) = lsp.send_request(
            server_id,
            "completionItem/resolve",
            raw.value().clone(),
            meta,
        ) else {
            return; // server gone between the capability check and now
        };
        let callback: LspCallback = Box::new(move |editor, _server_id, outcome| match outcome {
            hume_lsp::client::Outcome::Ok(resolved) => {
                let resolved_edits = parse_additional_text_edits_lenient(&resolved);
                let result = edits::build_edits_from_earlier_document(
                    &rope_pre,
                    &accept_cs,
                    encoding,
                    &resolved_edits,
                )
                .and_then(|char_edits| {
                    edits::commit_char_edits(
                        &mut editor.state,
                        &editor.view.panes,
                        pid,
                        bid,
                        char_edits,
                    )
                });
                if let Err(e) = result {
                    editor.report(Severity::Error, format!("lsp completion resolve: {e}"));
                }
            }
            hume_lsp::client::Outcome::Err(e) => {
                editor.report(
                    Severity::Error,
                    format!("lsp completion resolve: {} ({})", e.message, e.code),
                );
            }
            hume_lsp::client::Outcome::TimedOut => {
                editor.report(
                    Severity::Error,
                    "lsp completion resolve: timeout".to_string(),
                );
            }
        });
        let anchor = ResponseAnchor {
            bid,
            text_gen: gen_after,
            allow_stale: false,
            require_focus: None,
        };
        lsp.register_callback(server_id, id, anchor, callback);
    }
}
