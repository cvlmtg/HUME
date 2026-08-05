use hume_editing::changeset::ChangeSet;
use hume_editing::lines::leading_whitespace_end;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_scripting::SteelBufferId;
use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::super::dispatch::ArgSource;
use super::super::event::EditorEvent;
use super::super::keymap::WalkResult;
use super::super::registry::MappableCommand;
use super::super::replay::InsertInput;
use super::super::{Editor, Severity, commands, doc_ops};
use hume_ops::MotionMode;
use hume_ops::auto_pairs::{delete_pair, insert_pair_close};
use hume_ops::edit::{
    dedent_tab_backward, delete_char_backward, delete_char_forward, insert_char,
    insert_newline_indent, insert_tab,
};
use hume_ops::motion::cmd_move_right;

impl Editor {
    // ── Insert mode ───────────────────────────────────────────────────────────

    /// Applies a grouped edit on the focused (pane, buffer) and, if an LSP
    /// completion session is open on that same buffer, records the edit on
    /// it via `observe_edit` — the chokepoint every keystroke handler below
    /// that edits the focused buffer directly goes through, so no such call
    /// site needs its own record-or-not decision. (A cursor-motion or
    /// edit-command key that instead resolves through the insert trie is a
    /// separate case — `handle_insert`'s `WalkResult::Leaf` arm dismisses
    /// the session outright before reaching any of those, since none of them
    /// route back through here.) See `CompletionSession::observe_edit` for
    /// why every keystroke reaching this function needs recording, not just
    /// ones at the primary cursor.
    fn apply_insert_edit(
        &mut self,
        cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
    ) {
        let focused = self.state.focused_pane_id;
        let buf = self.focused_buffer_id();
        let cs = doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut self.state.panes.state,
            focused,
            buf,
            cmd,
        );
        // A session anchored to a different buffer than the one this edit
        // just landed on has nothing to record here — this can only happen
        // while a stale session (its buffer no longer focused) is still
        // open, since `apply_insert_edit` always edits the focused buffer.
        // `observe_edit`'s own length check would reject a mismatched
        // `ChangeSet` anyway, but checking `bid` up front documents why,
        // rather than relying on that as a coincidence.
        let stale = self
            .lsp
            .completion
            .as_mut()
            .is_some_and(|session| session.bid() == buf && !session.observe_edit(&cs));
        if stale {
            self.clear_completion_menu();
        }
    }

    pub(in super::super) fn handle_insert(&mut self, key: KeyEvent) {
        // ── LSP completion menu intercept ────────────────────────────────
        // Guarded early-return before the trie walk (not after) — Esc must
        // never reach the trie's exit-insert binding while a session is
        // open; Esc dismisses the *session*, staying in
        // Insert. Printable chars and Backspace-within-the-token are
        // deliberately NOT fully handled here — they fall through to the
        // normal body below, then get refiltered by the post-edit hook at
        // the end of this function.
        if self.lsp.completion.is_some() && self.handle_completion_key(key) {
            return;
        }

        // Walk the insert trie first: handles Esc, Ctrl+C, and arrow keys.
        // Regular characters (Char without CONTROL) and Backspace/Delete/Enter
        // are NOT in the insert trie — they're handled below.
        let trie_result = self.state.config.keymap.insert.walk(&[key]);
        match trie_result {
            WalkResult::Leaf(cmd) => {
                // Every key that resolves to a trie leaf is a cursor motion
                // or an edit command — Esc, arrows, Ctrl-W, any user-bound
                // insert key. None of them route through `apply_insert_edit`
                // (motions bypass it entirely; `MappableCommand::Edit` below
                // goes through `run_native_body` instead, which cannot hand
                // its `ChangeSet` back here — see that branch), so an open
                // completion session can't stay correctly anchored past one:
                // a motion moves the cursor off the token, and an edit
                // command mutates outside the one chokepoint that keeps the
                // session's anchor in sync. Dismiss unconditionally rather
                // than let either corrupt the session silently.
                self.clear_completion_menu();
                let Some(reg_cmd) = self
                    .state
                    .config
                    .registry
                    .get_mappable(cmd.name.as_ref())
                    .cloned()
                else {
                    self.report(Severity::Warning, format!("unknown command: {}", cmd.name));
                    return;
                };
                // Edit commands (e.g. Ctrl-W) must compose into the open insert-session
                // edit group. `run_native_body` routes through `apply_doc_edit_grouped`
                // when a group is open, so no special-casing is needed here.
                if let MappableCommand::Edit { .. } = reg_cmd {
                    commands::run_native_body(
                        &mut self.state,
                        &mut self.view,
                        reg_cmd,
                        Some(1),
                        false,
                    );
                    return;
                }
                // Any cursor-motion command (arrows, Home/End, …) invalidates
                // a pinned "typed run" — its anchor would otherwise select
                // across text the cursor jumped away from. exit-insert is the
                // finalizer itself and must not clear the pins it consumes.
                if cmd.name.as_ref() != "exit-insert" {
                    let pid = self.state.focused_pane_id;
                    let bid = self.focused_buffer_id();
                    self.state.panes.state[pid][bid].pinned_anchors = None;
                }
                self.execute_keymap_command(cmd.name, Some(1), false, ArgSource::Keymap);
                return;
            }
            WalkResult::NoMatch => {}
            // Interior / WaitChar can't arise in the insert trie (no multi-key
            // sequences, no wait-char bindings).
            WalkResult::Interior | WalkResult::WaitChar(_) => {}
        }

        // ── Dot-repeat recording ──────────────────────────────────────────────
        // Trie-matched keys (Esc, arrows) returned early above, so everything
        // reaching here is a text-modifying key — safe to record for replay.
        if let Some(ref mut session) = self.state.insert_session {
            session.keystrokes.push(InsertInput::Key(key));
        }

        // ── Character input ───────────────────────────────────────────────────
        let focused = self.state.focused_pane_id;
        let buf = self.focused_buffer_id();
        match key.code {
            KeyCode::Char(ch) if !key.modifiers.contains(Modifiers::CONTROL) => {
                // Typing real content cancels the "nothing typed since Enter"
                // state that gates the blank-line indent trim on exit — see
                // `EditorState::autoindent_pending`.
                self.state.autoindent_pending = false;
                let (ap_enabled, ap_pairs) =
                    self.doc().overrides.auto_pairs_ref(&self.state.settings);
                // `OnTriggerChar` only fires when `ch` actually landed in
                // the buffer — the two skip-close branches below just move
                // the cursor past an existing closer, inserting nothing.
                let mut inserted = true;
                if ap_enabled {
                    if let Some(pair) = ap_pairs.iter().find(|p| p.open == ch) {
                        let (open, close, symmetric) = (pair.open, pair.close, pair.is_symmetric());
                        if symmetric && self.should_skip_close(ch) {
                            // e.g. typing `"` when cursor already sits on `"`.
                            // NLL ends the `ap_pairs` borrow at its last use (the `find` above),
                            // so `&mut self.state.panes.state` here does not conflict with it.
                            doc_ops::apply_doc_motion(
                                &self.state.buffers,
                                &mut self.state.panes.state,
                                focused,
                                buf,
                                |b, s| cmd_move_right(b, s, 1, MotionMode::Move),
                            );
                            inserted = false;
                        } else if self.should_auto_pair(pair, ap_pairs) {
                            // Context is clear: insert open+close or wrap selection.
                            // NLL: `ap_pairs` last used in the condition above; borrow ends here.
                            self.apply_insert_edit(|b, s| insert_pair_close(b, s, open, close));
                        } else {
                            // Next char is a word char (or symmetric prev is word char):
                            // insert only the typed character.
                            self.apply_insert_edit(|b, s| insert_char(b, s, ch));
                        }
                    } else if ap_pairs.iter().any(|p| p.close == ch && !p.is_symmetric())
                        && self.should_skip_close(ch)
                    {
                        // Asymmetric close (e.g. `)`) when cursor is already on it.
                        doc_ops::apply_doc_motion(
                            &self.state.buffers,
                            &mut self.state.panes.state,
                            focused,
                            buf,
                            |b, s| cmd_move_right(b, s, 1, MotionMode::Move),
                        );
                        inserted = false;
                    } else {
                        self.apply_insert_edit(|b, s| insert_char(b, s, ch));
                    }
                } else {
                    self.apply_insert_edit(|b, s| insert_char(b, s, ch));
                }
                if inserted {
                    let language = self
                        .state
                        .buffers
                        .get(buf)
                        .language
                        .map(|id| self.state.config.languages.name_of(id).to_owned());
                    for source in self.state.trigger_sources_for(ch, language.as_deref()) {
                        self.queue_trigger_char(buf, ch, &source);
                    }
                }
            }

            // ── Tab ────────────────────────────────────────────────────────────
            // Governed by the `tab-style` setting: Hard inserts a literal `\t`,
            // Soft inserts spaces to the next tab stop (width from `tab-width`).
            KeyCode::Tab => {
                self.state.autoindent_pending = false;
                let style = self.doc().overrides.tab_style(&self.state.settings);
                let tw = self.doc().overrides.tab_width(&self.state.settings);
                self.apply_insert_edit(move |b, s| insert_tab(b, s, style, tw));
            }

            // ── Newline ───────────────────────────────────────────────────────
            // Auto-indent: copy the current line's leading whitespace onto the
            // new line. No smart indent (tree-sitter indent.scm is a separate
            // roadmap milestone).
            //
            // `trim_blank` (vim autoindent parity): only vacate a blank
            // line's whitespace if it was auto-inserted by *this* session's
            // own previous Enter — never on the first Enter that lands on a
            // pre-existing blank line. After this Enter, the new line's
            // indent (if any) is this session's own, so the next Enter/Esc
            // on it should trim.
            KeyCode::Enter => {
                let trim_blank = self.state.autoindent_pending;
                self.apply_insert_edit(move |b, s| insert_newline_indent(b, s, trim_blank));
                self.state.autoindent_pending = true;
            }

            // ── Delete ────────────────────────────────────────────────────────
            // Deliberately does NOT clear `autoindent_pending`: `:help
            // autoindent` names `<BS>` (alongside CTRL-D) as the one key that
            // doesn't cancel the "nothing typed on this line" state.
            KeyCode::Backspace => {
                let (ap_enabled, ap_pairs) =
                    self.doc().overrides.auto_pairs_ref(&self.state.settings);
                if self.should_dedent_backspace() {
                    let tw = self.doc().overrides.tab_width(&self.state.settings);
                    // Dedent: snap every cursor in leading whitespace back to
                    // the previous tab stop. All-or-nothing — if any cursor
                    // isn't in leading ws, the whole batch falls back.
                    self.apply_insert_edit(move |b, s| dedent_tab_backward(b, s, tw));
                } else if ap_enabled && self.is_between_pair(ap_pairs) {
                    self.apply_insert_edit(delete_pair);
                } else {
                    self.apply_insert_edit(delete_char_backward);
                }
            }
            KeyCode::Delete => {
                self.state.autoindent_pending = false;
                self.apply_insert_edit(delete_char_forward);
            }

            _ => {}
        }

        // Only reached for keys the completion pre-guard let fall through
        // (printable chars, Backspace within the token) — refilter using
        // the buffer's new anchor..cursor text now that the edit landed.
        if self.lsp.completion.is_some() {
            self.refilter_lsp_completion_after_edit(key);
        }
    }

    // ── LSP completion menu ─────────────────────────────────────────────

    /// The open completion session — every call site sits behind
    /// `handle_insert`'s `lsp.completion.is_some()` guard, so the session
    /// is always present here.
    fn open_completion_session(&self) -> &crate::editor::lsp::completion::CompletionSession {
        self.lsp
            .completion
            .as_ref()
            .expect("checked by handle_insert above")
    }

    /// Intercepts a key while an LSP completion session is open.
    /// Returns `true` if fully handled (skip the rest of `handle_insert`
    /// this call) — `false` if it should still fall through to normal
    /// Insert-mode dispatch (printable chars always do; Backspace always
    /// does, after this decides whether the session survives the deletion).
    fn handle_completion_key(&mut self, key: KeyEvent) -> bool {
        if self.open_completion_session().is_empty() {
            // Filtered to nothing by continued typing, or an `isIncomplete`
            // list awaiting an async re-request — either way no menu is
            // visibly shown, so nothing here should intercept a key.
            // Notably this covers Esc (falls through to the trie's
            // exit-insert leaf, which dismisses the session as a side
            // effect of leaving Insert — see `EditorState::set_mode` —
            // rather than needing a second Esc to actually leave Insert)
            // and Enter (inserts a newline instead of erroring on an
            // out-of-range `accept(0)`).
            return false;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.move_completion_selection(true);
                true
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.move_completion_selection(false);
                true
            }
            KeyCode::Enter => {
                self.accept_completion_selection();
                true
            }
            KeyCode::Escape => {
                self.clear_completion_menu();
                true
            }
            KeyCode::Backspace => {
                // The char Backspace is about to delete is the one right
                // before `head`. If `head` is already at (or before) the
                // anchor, that char lies *outside* the completed token —
                // crossing it, not just narrowing the filter.
                let head = self.current_selections().primary().head();
                if head <= self.open_completion_session().anchor() {
                    self.clear_completion_menu();
                }
                false
            }
            _ => false,
        }
    }

    /// Moves the completion menu's selection by one row. The popup scrolls
    /// to keep the selection visible, so the bound is the full ranked
    /// candidate list, not just the visible window.
    fn move_completion_selection(&mut self, forward: bool) {
        let Some(session) = self.lsp.completion.as_ref() else {
            return;
        };
        // `handle_completion_key`'s empty-session guard already returned
        // before dispatching here, so `n` is always positive.
        let n = session.len();
        let ui = self
            .lsp
            .completion_ui
            .get_or_insert(crate::editor::lsp::completion::CompletionMenuUi { selected: 0 });
        if forward {
            ui.selected = (ui.selected + 1) % n;
        } else {
            ui.selected = ui.selected.checked_sub(1).unwrap_or(n - 1);
        }
    }

    /// Accepts the currently-selected completion item through the same
    /// gen-checked edit path as `completion-accept!` — the session ends
    /// either way (success or failure), matching `EditorHostImpl`'s own
    /// completion_accept.
    fn accept_completion_selection(&mut self) {
        let selected = self.lsp.completion_ui.as_ref().map_or(0, |ui| ui.selected);
        let Some(session) = self.lsp.completion.take() else {
            return;
        };
        self.clear_completion_menu();
        if let Err(msg) = session.accept(&mut self.state, &mut self.lsp, selected) {
            self.report(Severity::Error, msg);
        }
    }

    /// Re-ranks the open completion session against the token text between
    /// its anchor and the current cursor — called after a printable char or
    /// Backspace has already landed in the buffer.
    fn refilter_lsp_completion_after_edit(&mut self, key: KeyEvent) {
        let is_char =
            matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(Modifiers::CONTROL);
        if !is_char && key.code != KeyCode::Backspace {
            return;
        }
        // Phase 1 — shared reads only: peek the anchor without taking the
        // session, so no put-back is ever needed.
        let Some(session) = self.lsp.completion.as_ref() else {
            return;
        };
        let anchor = session.anchor();
        let head = self.current_selections().primary().head();
        // Backspace crossing the anchor already dismissed the session in
        // `handle_completion_key`, before the edit ran. But `head` can still
        // land before `anchor` here — e.g. a Steel hook mutating selections
        // mid-session, or any other out-of-band cursor move that doesn't
        // route through `handle_insert`'s trie-leaf dismissal. Dismiss
        // rather than slice with an inverted or out-of-range span.
        let len = self.doc().text().len_chars();
        if head < anchor || head > len {
            self.clear_completion_menu();
            return;
        }
        let text = self.doc().text().slice(anchor..head).to_string();

        // Phase 2 — disjoint-field destructure (the `client_and_backend`/
        // `LspState` pattern): `update_filter` needs `&mut lsp.completion`
        // and `&state` at once, which a whole-`self` method call can't do,
        // but plain field access can.
        let Editor { state, lsp, .. } = &mut *self;
        let Some(session) = lsp.completion.as_mut() else {
            return; // can't happen (checked above), but never assume it
        };
        session.update_filter(state, text.clone());
        let incomplete = session.incomplete();
        let bid = session.bid();

        // Phase 3 — borrows from phase 2 have ended; back to whole-`self`.
        // `on-completion-refilter` fires only while the server said
        // `isIncomplete` — a complete list needs no re-request, so a normal
        // session stays hook-silent on every keystroke.
        if incomplete {
            let bid_val = SteelBufferId::new(bid).into_steel_val();
            let text_val = steel::rvals::SteelVal::StringV(text.into());
            self.queue_event(EditorEvent::OnCompletionRefilter, &[bid_val, text_val]);
        }
        self.lsp.completion_ui = None;
    }

    // ── Auto-pair helpers ─────────────────────────────────────────────────────

    /// Returns `true` if every selection is a collapsed cursor sitting in a
    /// line's leading whitespace (spaces/tabs), with at least one whitespace
    /// char before it. All-or-nothing: if any selection doesn't qualify, the
    /// whole batch falls back to plain Backspace so multi-cursor behaviour
    /// stays consistent.
    ///
    /// "In leading whitespace" means every char in `[line_start, head)` is a
    /// space or tab — so a cursor on the first content char (right after the
    /// indent) also qualifies, matching the dedent-to-prev-tab-stop behaviour
    /// of modern editors. The boundary itself comes from the shared
    /// [`leading_whitespace_end`] primitive.
    fn should_dedent_backspace(&self) -> bool {
        let buf = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            if !sel.is_collapsed() {
                return false;
            }
            let p = sel.head();
            let line_idx = buf.char_to_line(p);
            let line_start = buf.line_to_char(line_idx);
            // `p > line_start` rules out col 0 (nothing to dedent). `p <=
            // leading_whitespace_end` keeps the all-or-nothing "in leading ws"
            // rule: at exactly the end the cursor sits on the first content
            // char and still qualifies.
            p > line_start && p <= leading_whitespace_end(buf, line_idx)
        })
    }

    /// Returns `true` if every selection is a cursor AND the character at each
    /// cursor's `head` equals `ch`.
    ///
    /// All-or-nothing: if even one cursor doesn't match, the whole operation
    /// falls back to normal insert, keeping multi-cursor behavior consistent.
    fn should_skip_close(&self, ch: char) -> bool {
        self.current_selections()
            .iter_sorted()
            .all(|sel| sel.is_collapsed() && self.doc().text().char_at(sel.head()) == Some(ch))
    }

    /// Returns `true` if every selection is a cursor AND the pair
    /// `(char_before_cursor, char_at_cursor)` matches a configured pair.
    ///
    /// Used by Backspace to decide whether to delete both brackets or just one.
    fn is_between_pair(&self, pairs: &[hume_ops::auto_pairs::Pair]) -> bool {
        let buf = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            if !sel.is_collapsed() || sel.head() == 0 {
                return false;
            }
            // prev_grapheme_boundary handles multi-codepoint clusters; bracket/quote
            // chars are always single codepoints, but using it keeps the logic uniform.
            let prev = hume_editing::grapheme::prev_grapheme_boundary(buf, sel.head());
            match (buf.char_at(prev), buf.char_at(sel.head())) {
                (Some(before), Some(at)) => pairs.iter().any(|p| p.open == before && p.close == at),
                _ => false,
            }
        })
    }

    /// Returns `true` if auto-pairing `pair` is appropriate given the current
    /// selections. All-or-nothing: every collapsed selection must satisfy the
    /// context rules; non-collapsed selections always pass (they wrap).
    fn should_auto_pair(
        &self,
        pair: &hume_ops::auto_pairs::Pair,
        ap_pairs: &[hume_ops::auto_pairs::Pair],
    ) -> bool {
        let buf = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            !sel.is_collapsed()
                || hume_ops::auto_pairs::should_auto_pair_at(buf, sel.head(), pair, ap_pairs)
        })
    }
}
