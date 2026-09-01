//! `CompletionSession::accept` — the churn hotspot: applies the selected
//! candidate's `textEdit` (or a synthesized token-replacement fallback) at
//! every cursor, then best-effort `completionItem/resolve`.

use hume_editing::changeset::Assoc;

use super::CompletionSession;
use super::item::{StoredCompletionItem, parse_additional_text_edits_lenient};
use crate::editor::event::EditorEvent;
use crate::editor::lsp::{LspCallback, LspState, edits, introspect, wire_range_to_chars};
use crate::editor::{EditorState, Severity};
use hume_ops::edit::{replace_around_cursors, replace_span_around_cursors, word_start_before};

/// How `CompletionSession::accept` derives the per-cursor deletion span: a
/// uniform `(back, forward)` pair when the server sent a `textEdit` (safe
/// everywhere per the LSP containment guarantee on the server's own range,
/// applied via [`replace_around_cursors`]), or each cursor's own preceding
/// identifier token when it didn't (no such guarantee exists for a
/// synthesized range, so it must be computed per cursor via
/// [`replace_span_around_cursors`] — see `accept`'s `None` arm for how the
/// fields here become that per-cursor `start_of` closure).
#[derive(Clone, Copy)]
enum ReplaceSpan {
    Uniform {
        back: usize,
        forward: usize,
    },
    TokenBefore {
        /// The session's own primary cursor — identifiable by its head
        /// position since `SelectionSet` heads are unique — gets `anchor`
        /// as its token start rather than `head - typed`; see the field
        /// docs on those two for why they can diverge.
        primary_head: usize,
        /// `CompletionSession::anchor()` — tracked independently of live
        /// buffer content, so it stays correct even when `self.filter` was
        /// narrowed via `completion-update-filter!` without a matching real
        /// edit (the primary's head then doesn't reflect `typed` chars at
        /// all, so `head - typed` would be wrong for it specifically).
        anchor: usize,
        /// Chars this session has logically consumed since it began — the
        /// same at every cursor, since multi-cursor Insert types
        /// identically everywhere. Skipped before each *non-primary*
        /// cursor's own backward token scan, so the scan only ever looks at
        /// that cursor's own pre-session content.
        typed: usize,
        forward: usize,
    },
}

impl CompletionSession {
    /// Applies `filtered[idx]`'s `textEdit` (falling back to `insertText`
    /// over each cursor's own identifier token when absent) at *every*
    /// cursor in the session's pane, as if the completion had been typed at
    /// each — a conforming server's completion range always contains the
    /// request position (LSP spec, `completion.rs`'s `text_edit` doc), so
    /// the primary's own edit, re-expressed as a char count behind/ahead of
    /// its live head, is the same span typing would have consumed at any
    /// cursor. `additionalTextEdits` have no cursor of their own and are
    /// applied once, document-wide. Both land as one undo step — gen-checked
    /// against `generation_at_begin`.
    ///
    /// If the item lacks `additionalTextEdits` entirely (not just an empty
    /// array — see [`StoredCompletionItem::has_additional_text_edits`]) and
    /// the server advertises `completionProvider.resolveProvider`, sends
    /// `completionItem/resolve` and applies whatever it returns once the
    /// response lands (via the ordinary `LspCallback`/`stale_check`
    /// machinery every other `lsp-request` uses — dropped silently if the
    /// buffer has moved past `generation_at_begin`'s successor by then,
    /// same staleness discipline as any other LSP response).
    pub(crate) fn accept(
        &self,
        state: &mut EditorState,
        lsp: &mut LspState,
        idx: usize,
    ) -> Result<(), String> {
        let &item_idx = self
            .filtered
            .get(idx)
            .ok_or_else(|| "completion-accept!: index out of range".to_string())?;
        let item = &self.items[item_idx as usize];
        edits::checked_buffer(state, self.bid, Some(self.generation_at_begin))?;
        let encoding = introspect::encoding_for_buffer(state, lsp, self.bid);

        // The session's pane/buffer pairing may no longer be live — a pane
        // switch (nothing dismisses the session on one), or the Steel
        // `completion-accept!` builtin firing from a different pane than
        // `begin()` resolved. `pane_state::ensure`'s fallback (fabricate a
        // fresh cursor at char 0 for a pane that never showed this buffer)
        // is right for "a background buffer with no selection state yet",
        // not for "this session's own point of reference is gone" — so this
        // errors instead of silently landing the edit at the top of the file.
        if state.focused_pane_id != self.pane_id {
            return Err("completion-accept!: the session's pane is no longer focused".to_string());
        }
        let pid = self.pane_id;
        let head_now = {
            let pbs = state.panes.buffer_state(pid, self.bid).ok_or_else(|| {
                "completion-accept!: buffer is no longer shown in the session's pane".to_string()
            })?;
            // The "as if typed at each cursor" model has no meaning for a
            // real selection — typing over one is a different edit than
            // completing at it, and `replace_*_cursors` force-collapses
            // every selection it touches, which would silently discard a
            // real selection set.
            if !pbs.selections.iter_sorted().all(|s| s.is_collapsed()) {
                return Err("completion-accept!: selections must be collapsed".to_string());
            }
            pbs.selections.primary().head()
        };

        let (span, new_text) = match &item.text_edit {
            Some(te) => {
                let rope_at_begin = &self.rope_at_begin;
                let (start_b, end_b) = wire_range_to_chars(rope_at_begin, &te.range, encoding);
                if end_b < start_b {
                    return Err(format!(
                        "text edit has a reversed range (end {end_b} before start {start_b})"
                    ));
                }
                // Decoded once against the frozen request-time snapshot
                // above, then mapped forward through every edit this
                // session actually observed via `observe_edit` (`Assoc::
                // Before` on the start so it stays pinned to the token even
                // if an observed insertion landed exactly there;
                // `Assoc::After` on the end so an observed insertion at or
                // inside the range extends it rather than being left
                // stranded next to the completion text) — exact position
                // tracking through the intervening keystrokes, not a
                // scalar-drift guess. Two single-position maps, not
                // `map_ranges`: that helper hardcodes both ends to *shrink*
                // on a boundary insertion, which is the wrong association
                // for the end here.
                let mut start_pos = [start_b];
                self.cs_since_begin
                    .map_positions(&mut start_pos, Assoc::Before);
                let start_now = start_pos[0];
                let mut end_pos = [end_b];
                self.cs_since_begin
                    .map_positions(&mut end_pos, Assoc::After);
                // `self.filter` can narrow independent of any edit this
                // session observed — `completion-update-filter!` sets it
                // directly, without touching the buffer (used by
                // programmatic/scripted callers, and by tests). Extending
                // (never shrinking) to cover it here catches that case too,
                // on top of whatever `cs_since_begin` mapped from real edits.
                let end_now = end_pos[0].max(self.anchor() + self.filter.chars().count());
                // The delta model below rests entirely on this containment:
                // a conforming server's completion range always contains
                // the request position (LSP spec). An off-spec server, or a
                // cursor that has since moved outside the range (e.g. an
                // arrow key the completion menu deliberately lets through),
                // breaks that assumption — erroring here, buffer untouched,
                // is safer than silently clamping to some other span.
                if !(start_now <= head_now && head_now <= end_now) {
                    return Err(
                        "completion-accept!: textEdit range does not contain the cursor"
                            .to_string(),
                    );
                }
                (
                    ReplaceSpan::Uniform {
                        back: head_now - start_now,
                        forward: end_now - head_now,
                    },
                    te.new_text.clone(),
                )
            }
            // No server-provided range: replace each cursor's own preceding
            // identifier token rather than just the anchor..cursor span —
            // any prefix typed *before* triggering completion (e.g. "fo"
            // before the popup opened) is otherwise left untouched,
            // duplicating it ahead of `insert_text`. See `ReplaceSpan::
            // TokenBefore`'s field docs for why the primary and the other
            // cursors need different treatment here.
            None => {
                let typed = self.filter.chars().count();
                let anchor = self.anchor();
                let forward = (anchor + typed).saturating_sub(head_now);
                (
                    ReplaceSpan::TokenBefore {
                        primary_head: head_now,
                        anchor,
                        typed,
                        forward,
                    },
                    item.insert_text.clone(),
                )
            }
        };

        // Captured before any edit lands — a resolve response (if one ends
        // up sent below) is computed against this exact pre-accept document,
        // and its wire positions must be decoded against it, not whatever
        // the buffer holds once the response actually arrives.
        let rope_pre = state.buffers.get(self.bid).text().rope().clone();

        // Decoded and mapped here (pure — no mutation yet) so an overlap
        // with the main edit's own range (checked just below) can be caught
        // before either lands.
        let additional_char_edits = if item.additional_text_edits.is_empty() {
            Vec::new()
        } else {
            edits::build_edits_from_earlier_document(
                &self.rope_at_begin,
                &self.cs_since_begin,
                encoding,
                &item.additional_text_edits,
            )?
        };
        // Scoped to the server-range case: only there does the main edit
        // have a single, well-defined [start, end) to check against — the
        // token-replacement fallback has no server-provided range to
        // overlap in the first place.
        if let ReplaceSpan::Uniform { back, forward } = span {
            let (start_now, end_now) = (head_now - back, head_now + forward);
            // The half-open overlap test alone (`s < end_now && start_now <
            // e`) misses a *zero-width* additional edit sitting exactly at
            // `end_now`: it inserts before the cursor edit lands, so
            // `translate_in_place`'s `Assoc::After` on selection heads
            // (`hume-editing/src/selection/mod.rs`) walks the live head past
            // the inserted text — the cursor edit's `back` chars then eat
            // that inserted text instead of the span the server asked for.
            // An insertion at `start_now` is safe (it shifts the whole span
            // uniformly ahead of the edit) and stays excluded.
            let overlaps = additional_char_edits
                .iter()
                .any(|&(s, e, _)| (s < end_now && start_now < e) || (s == e && s == end_now));
            if overlaps {
                return Err("completion-accept!: textEdit overlaps additionalTextEdits".to_string());
            }
        }

        // Insert mode already has a group open (composing this accept into
        // the ongoing session); a Steel-triggered accept outside Insert mode
        // does not, so open one here — both edits below then land as one
        // undo step regardless of caller.
        let opened_group = state.panes.state[pid][self.bid].edit_group.is_none();
        if opened_group {
            crate::editor::doc_ops::begin_edit_group(
                &state.buffers,
                &mut state.panes.state,
                pid,
                self.bid,
            );
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
        let cs_additional = match edits::commit_char_edits(state, self.bid, additional_char_edits) {
            Ok(cs) => cs,
            Err(e) => {
                if opened_group {
                    crate::editor::doc_ops::commit_edit_group(
                        &mut state.buffers,
                        &mut state.panes.state,
                        pid,
                        self.bid,
                    );
                }
                return Err(e);
            }
        };

        // `primary_head`/`anchor` were captured before `additionalTextEdits`
        // landed — the closure below compares them against live heads read
        // *after* `commit_char_edits` above already shifted every selection
        // across those edits (`apply_doc_edit_grouped` → `translate_in_place`).
        // Left unmapped, an additional edit ahead of the cursor (e.g. an
        // auto-inserted import line) would make `head == primary_head` never
        // match the real primary, or — worse — spuriously match a different
        // cursor that happened to remap onto the stale value.
        //
        // Both map with `Assoc::After`, *not* the `Before` `anchor()` itself
        // uses for `cs_since_begin` — that association is specific to real
        // typed content (a char landing exactly at the anchor extends the
        // token leftward-inclusive). `cs_additional` is a foreign, unrelated
        // document edit (e.g. an auto-inserted import), not typed content;
        // an edit landing exactly at the anchor should carry it forward
        // exactly like any other live cursor position would, so the
        // completion's own text still lands where the user's token actually
        // was — after the inserted text, never spliced inside it.
        let span = match span {
            ReplaceSpan::TokenBefore {
                primary_head,
                anchor,
                typed,
                forward,
            } => {
                let (primary_head, anchor) = match &cs_additional {
                    Some(cs_a) => {
                        let mut ph = [primary_head];
                        cs_a.map_positions(&mut ph, Assoc::After);
                        let mut a = [anchor];
                        cs_a.map_positions(&mut a, Assoc::After);
                        (ph[0], a[0])
                    }
                    None => (primary_head, anchor),
                };
                ReplaceSpan::TokenBefore {
                    primary_head,
                    anchor,
                    typed,
                    forward,
                }
            }
            uniform => uniform,
        };

        let cs_cursors = match span {
            ReplaceSpan::Uniform { back, forward } => {
                crate::editor::doc_ops::apply_doc_edit_grouped(
                    &mut state.buffers,
                    &state.config.decorations,
                    &mut state.panes.state,
                    &mut state.panes.jumps,
                    pid,
                    self.bid,
                    move |b, s| replace_around_cursors(b, s, back, forward, &new_text),
                )
            }
            ReplaceSpan::TokenBefore {
                primary_head,
                anchor,
                typed,
                forward,
            } => {
                let word_chars = crate::editor::commands::word_chars_owned(
                    state.buffers.get(self.bid),
                    &state.settings,
                );
                crate::editor::doc_ops::apply_doc_edit_grouped(
                    &mut state.buffers,
                    &state.config.decorations,
                    &mut state.panes.state,
                    &mut state.panes.jumps,
                    pid,
                    self.bid,
                    move |b, s| {
                        let chars = hume_editing::word::WordChars::new(&word_chars);
                        replace_span_around_cursors(
                            b,
                            s,
                            move |text, head| {
                                if head == primary_head {
                                    word_start_before(text, anchor, chars)
                                } else {
                                    word_start_before(text, head.saturating_sub(typed), chars)
                                }
                            },
                            forward,
                            &new_text,
                        )
                    },
                )
            }
        };

        if opened_group {
            crate::editor::doc_ops::commit_edit_group(
                &mut state.buffers,
                &mut state.panes.state,
                pid,
                self.bid,
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
        // parse (e.g. `command`); Rust now owns additionalTextEdits/resolve.
        state.queue_event(EditorEvent::OnCompletionAccept {
            buffer: self.bid,
            item: item.raw.clone(),
        });

        if !item.has_additional_text_edits {
            self.maybe_send_resolve(state, lsp, item, rope_pre, accept_cs, encoding);
        }
        Ok(())
    }

    /// Sends `completionItem/resolve` when the server advertised
    /// `completionProvider.resolveProvider` — best-effort: a resolution
    /// error, timeout, or a server that's gone by send time only logs, it
    /// never fails the accept that already landed.
    fn maybe_send_resolve(
        &self,
        state: &mut EditorState,
        lsp: &mut LspState,
        item: &StoredCompletionItem,
        rope_pre: ropey::Rope,
        accept_cs: hume_editing::changeset::ChangeSet,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) {
        let Some(server_id) = state.buffers.try_get(self.bid).and_then(|b| b.lsp_server) else {
            return;
        };
        let resolve_provider = lsp
            .servers
            .get(&server_id)
            .and_then(|e| e.capabilities_json.as_ref())
            .and_then(|caps| caps.get("completionProvider"))
            .and_then(|cp| cp.get("resolveProvider"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !resolve_provider {
            return;
        }

        // Same discipline `lsp-request` itself uses (bridge.rs): a request
        // minted here must not reach the wire ahead of the didChange
        // describing the edit `accept` just applied.
        crate::editor::lsp::sync::flush_lsp_pending_changes(state, lsp);
        let bid = self.bid;
        let timeout_ms = state.settings.lsp_request_timeout_ms as u64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let meta = hume_lsp::client::RequestMeta {
            method: "completionItem/resolve".to_string(),
            allow_stale: false,
            deadline,
        };
        let gen_after = state.buffers.get(bid).text_gen;
        let Some(id) =
            lsp.send_request(server_id, "completionItem/resolve", item.raw.clone(), meta)
        else {
            return; // server gone between the capability check and now
        };
        let callback: LspCallback = Box::new(move |editor, outcome| match outcome {
            hume_lsp::client::Outcome::Ok(resolved) => {
                let resolved_edits = parse_additional_text_edits_lenient(&resolved);
                let result = edits::build_edits_from_earlier_document(
                    &rope_pre,
                    &accept_cs,
                    encoding,
                    &resolved_edits,
                )
                .and_then(|char_edits| {
                    edits::commit_char_edits(&mut editor.state, bid, char_edits)
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
        lsp.register_callback(server_id, id, Some((bid, gen_after)), callback);
    }
}
