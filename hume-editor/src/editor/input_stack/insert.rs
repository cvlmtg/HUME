//! The `Insert` layer, plus [`Editor::handle_insert`] and
//! [`Editor::apply_insert_mode_paste`] — kept as `impl Editor` methods
//! rather than free functions like this crate's other layer handlers,
//! because both have a second caller outside dispatch: `replay.rs`'s
//! dot-repeat/macro replay calls them directly (`self.handle_insert(*key)`,
//! `self.apply_insert_mode_paste(text)`) to replay a recorded insert
//! session's keystrokes. Converting them to free functions would force that
//! caller into a new qualified path for no benefit `insert_input` doesn't
//! already provide.

use hume_editing::changeset::ChangeSet;
use hume_editing::lines::leading_whitespace_end;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::MotionMode;
use hume_ops::auto_pairs::{delete_pair, insert_pair_close};
use hume_ops::edit::{
    dedent_tab_backward, delete_char_backward, delete_char_forward, insert_char,
    insert_newline_indent, insert_tab,
};
use hume_ops::motion::cmd_move_right;

use super::super::event::EditorEvent;
use super::super::keymap::WalkResult;
use super::super::registry::MappableCommand;
use super::super::replay::InsertInput;
use super::super::{Editor, EditorState, commands, doc_ops};
use super::popup::PopupLayer;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct InsertLayer {
    pub(in crate::editor) sticky_popup: Option<PopupLayer>,
}

impl Layer for InsertLayer {
    fn handler(&self) -> LayerHandler {
        insert_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Insert)
    }
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView, _why: Removal) {
        commands::tear_down_insert(state, view);
    }
    fn sticky_popup_slot(&self) -> Option<&Option<PopupLayer>> {
        Some(&self.sticky_popup)
    }
    fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        Some(&mut self.sticky_popup)
    }
}

pub(in crate::editor) fn insert_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    match ev {
        InputEvent::Key(key) => ed.handle_insert(key),
        InputEvent::Paste(text) => {
            ed.apply_insert_mode_paste(&text);
            if let Some(session) = ed.state.insert_session.as_mut() {
                session.keystrokes.push(InsertInput::Paste(text));
            }
        }
        // A click or a wheel notch is Base's own action to run — most
        // visibly, a click's `focus_pane` ends this very Insert session
        // before resolving the click (see `focus::focus_pane`'s doc).
        InputEvent::Mouse(mouse) => ed.fall_through(r, InputEvent::Mouse(mouse)),
    }
}

impl Editor {
    // ── Insert mode ───────────────────────────────────────────────────────────

    /// Applies a grouped edit on the focused (pane, buffer) and, if an LSP
    /// completion session is open on that same buffer, records the edit on
    /// it via `observe_edit` — the chokepoint every keystroke handler below
    /// that edits the focused buffer directly goes through, so no such call
    /// site needs its own record-or-not decision. (A cursor-motion or
    /// edit-command key that instead resolves through the insert trie is a
    /// separate case — `completion_input`'s own trie peek dismisses the
    /// session outright before falling through to any of those, since none
    /// of them route back through here.) See `CompletionSession::observe_edit`
    /// for why every keystroke reaching this function needs recording, not
    /// just ones at the primary cursor.
    fn apply_insert_edit(
        &mut self,
        cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
    ) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        let cs = doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
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
            .state
            .input
            .completion_mut()
            .is_some_and(|session| session.bid() == buf && !session.observe_edit(&cs));
        if stale {
            self.state.dismiss_completion(&self.view);
        }
    }

    pub(in crate::editor) fn handle_insert(&mut self, key: KeyEvent) {
        // Walk the insert trie first: handles Esc, Ctrl-c, and arrow keys.
        // Regular characters (Char without CONTROL) and Backspace/Delete/Enter
        // are NOT in the insert trie — they're handled below.
        let trie_result = self.state.config.keymap.insert.walk(&[key]);
        match trie_result {
            WalkResult::Leaf(cmd) => {
                // Every key that resolves to a trie leaf is a cursor motion
                // or an edit command — Esc, arrows, Ctrl-w, any user-bound
                // insert key. None of them route through `apply_insert_edit`
                // (motions bypass it entirely; an edit command reaches the
                // buffer either via `MappableCommand::Edit` below through
                // `run_native_body`, or — like Ctrl-w's `EditorCmd` — through
                // the ordinary `execute_keymap_command` dispatch further
                // down; neither hands its `ChangeSet` back here), so an open
                // completion session can't stay correctly anchored past one.
                // Dismissing it is `completion_input`'s job, not this
                // function's: it peeks this same trie walk before falling
                // through here, and retires the layer once this call
                // returns — see its own doc.
                let Some(reg_cmd) = self
                    .state
                    .config
                    .registry
                    .get_mappable(cmd.name.as_ref())
                    .cloned()
                else {
                    self.report_unknown_command(
                        cmd.name.as_ref(),
                        format!("unknown command: {}", cmd.name),
                    );
                    return;
                };
                // Edit commands (e.g. Ctrl-w) must compose into the open insert-session
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
                // A cursor-motion command reached here invalidates a pinned
                // typed run — see `step_clear_typed_run` (`commands/pipeline.rs`),
                // which `execute_keymap_command` below runs through.
                self.execute_keymap_command(cmd.name, Some(1), false);
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
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        match key.code {
            KeyCode::Char(ch) if !key.modifiers.contains(Modifiers::CONTROL) => {
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
                        .map(|id| self.state.config.languages.name_of(id));
                    for source in self.state.trigger_sources_for(ch, language) {
                        self.state.queue_event(EditorEvent::OnTriggerChar {
                            buffer: buf,
                            ch,
                            source,
                        });
                    }
                }
            }

            // ── Tab ────────────────────────────────────────────────────────────
            // Governed by the `tab-style` setting: Hard inserts a literal `\t`,
            // Soft inserts spaces to the next tab stop (width from `tab-width`).
            KeyCode::Tab => {
                let (style, tw) = commands::tab_format(self.doc(), &self.state.settings);
                self.apply_insert_edit(move |b, s| insert_tab(b, s, style, tw));
            }

            // ── Newline ───────────────────────────────────────────────────────
            // Auto-indent: copy the current line's leading whitespace onto the
            // new line. No smart indent.
            //
            // `allowed` (vim autoindent parity): only vacate a blank line's
            // whitespace if it's owned by an earlier auto-indent this session
            // itself made — never on the first Enter that lands on a
            // pre-existing blank line, since nothing has armed a record for
            // it yet. `arm_autoindent` after the edit records the *new*
            // line's own copied indent, so the next Enter/Esc on it trims.
            KeyCode::Enter => {
                let allowed = commands::autoindent_owned(&self.state, &self.view);
                self.apply_insert_edit(move |b, s| insert_newline_indent(b, s, &allowed));
                commands::arm_autoindent(&mut self.state, &self.view);
            }

            // ── Delete ────────────────────────────────────────────────────────
            // Backspace needs no special handling to preserve autoindent
            // ownership: deleting *inside* the owned range only shrinks the
            // line's current whitespace, which stays within the recorded
            // `allowed.end` (see `is_owned_blank_line`'s containment check) —
            // matching `:help autoindent`'s own carve-out naming `<BS>` as the
            // one key that doesn't cancel a pending auto-indent trim.
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
                self.apply_insert_edit(delete_char_forward);
            }

            _ => {}
        }
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
        let text = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            if !sel.is_collapsed() {
                return false;
            }
            let p = sel.head();
            let line_idx = text.char_to_line(p);
            let line_start = text.line_to_char(line_idx.into());
            // `p > line_start` rules out char_col 0 (nothing to dedent). `p <=
            // leading_whitespace_end` keeps the all-or-nothing "in leading ws"
            // rule: at exactly the end the cursor sits on the first content
            // char and still qualifies.
            p > line_start && p <= leading_whitespace_end(text, line_idx)
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
        let text = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            if !sel.is_collapsed() || sel.head() == hume_rope::offset::CharOffset::new(0) {
                return false;
            }
            // prev_grapheme_boundary handles multi-codepoint clusters; bracket/quote
            // chars are always single codepoints, but using it keeps the logic uniform.
            let prev = hume_editing::grapheme::prev_grapheme_boundary(text, sel.head());
            match (text.char_at(prev), text.char_at(sel.head())) {
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
        let text = self.doc().text();
        let chars = commands::effective_word_chars(self.doc(), &self.state.settings);
        self.current_selections().iter_sorted().all(|sel| {
            !sel.is_collapsed()
                || hume_ops::auto_pairs::should_auto_pair_at(
                    text,
                    sel.head(),
                    pair,
                    ap_pairs,
                    chars,
                )
        })
    }

    /// Bulk-insert `text` into the focused buffer as one grouped edit — the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Deliberately bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(in crate::editor) fn apply_insert_mode_paste(&mut self, text: &str) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            focused,
            buf,
            |b, s| hume_ops::edit::insert_str(b, s, text),
        );
        self.state.dismiss_completion(&self.view);
    }
}
