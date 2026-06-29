use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hume_editing::lines::leading_whitespace_end;

use super::super::keymap::WalkResult;
use super::super::registry::MappableCommand;
use super::super::{Editor, Severity, doc_ops};
use crate::auto_pairs::{delete_pair, insert_pair_close};
use crate::ops::MotionMode;
use crate::ops::edit::{
    dedent_tab_backward, delete_char_backward, delete_char_forward, insert_char,
    insert_newline_indent, insert_tab,
};
use crate::ops::motion::cmd_move_right;

impl Editor {
    // ── Insert mode ───────────────────────────────────────────────────────────

    pub(in super::super) fn handle_insert(&mut self, key: KeyEvent) {
        // Walk the insert trie first: handles Esc, Ctrl+C, and arrow keys.
        // Regular characters (Char without CONTROL) and Backspace/Delete/Enter
        // are NOT in the insert trie — they're handled below.
        let trie_result = self.state.keymap.insert.walk(&[key]);
        match trie_result {
            WalkResult::Leaf(cmd) => {
                // Dispatch Edit commands (e.g. Ctrl-W) as grouped edits so they
                // compose into the insert session's open edit group instead of
                // creating a standalone undo revision that would corrupt the
                // group's changeset composition.
                let Some(reg_cmd) = self.state.registry.get_mappable(cmd.name.as_ref()).cloned()
                else {
                    self.report(Severity::Warning, format!("unknown command: {}", cmd.name));
                    return;
                };
                // single-funnel-exempt: insert-mode edits must go through apply_doc_edit_grouped (stamps_last_command handled inline)
                if let MappableCommand::Edit { fun, name, .. } = reg_cmd {
                    let focused = self.state.focused_pane_id;
                    let buf = self.focused_buffer_id();
                    doc_ops::apply_doc_edit_grouped(
                        &mut self.state.buffers,
                        &mut self.state.panes.state,
                        focused,
                        buf,
                        fun,
                    );
                    self.state.last_command = Some(name);
                    return;
                }
                self.execute_keymap_command(cmd.name, 1, false, vec![]);
                return;
            }
            WalkResult::NoMatch => {}
            // Interior / WaitChar can't arise in the insert trie (no multi-key
            // sequences, no wait-char bindings).
            WalkResult::Interior { .. } | WalkResult::WaitChar(_) => {}
        }

        // ── Dot-repeat recording ──────────────────────────────────────────────
        // Trie-matched keys (Esc, arrows) returned early above, so everything
        // reaching here is a text-modifying key — safe to record for replay.
        if let Some(ref mut session) = self.state.insert_session {
            session.keystrokes.push(key);
        }

        // ── Character input ───────────────────────────────────────────────────
        let focused = self.state.focused_pane_id;
        let buf = self.focused_buffer_id();
        match key.code {
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let (ap_enabled, ap_pairs) =
                    self.doc().overrides.auto_pairs_ref(&self.state.settings);
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
                        } else if self.should_auto_pair(pair, ap_pairs) {
                            // Context is clear: insert open+close or wrap selection.
                            // NLL: `ap_pairs` last used in the condition above; borrow ends here.
                            doc_ops::apply_doc_edit_grouped(
                                &mut self.state.buffers,
                                &mut self.state.panes.state,
                                focused,
                                buf,
                                |b, s| insert_pair_close(b, s, open, close),
                            );
                        } else {
                            // Next char is a word char (or symmetric prev is word char):
                            // insert only the typed character.
                            doc_ops::apply_doc_edit_grouped(
                                &mut self.state.buffers,
                                &mut self.state.panes.state,
                                focused,
                                buf,
                                |b, s| insert_char(b, s, ch),
                            );
                        }
                    } else if ap_pairs.iter().any(|p| p.close == ch && !p.is_symmetric())
                        && self.should_skip_close(ch)
                    {
                        // Asymmetric close (e.g. `)`) when cursor is already on it.
                        // NLL: `ap_pairs` last used in the condition above; borrow ends here.
                        doc_ops::apply_doc_motion(
                            &self.state.buffers,
                            &mut self.state.panes.state,
                            focused,
                            buf,
                            |b, s| cmd_move_right(b, s, 1, MotionMode::Move),
                        );
                    } else {
                        doc_ops::apply_doc_edit_grouped(
                            &mut self.state.buffers,
                            &mut self.state.panes.state,
                            focused,
                            buf,
                            |b, s| insert_char(b, s, ch),
                        );
                    }
                } else {
                    doc_ops::apply_doc_edit_grouped(
                        &mut self.state.buffers,
                        &mut self.state.panes.state,
                        focused,
                        buf,
                        |b, s| insert_char(b, s, ch),
                    );
                }
            }

            // ── Tab ────────────────────────────────────────────────────────────
            // Governed by the `tab-style` setting: Hard inserts a literal `\t`,
            // Soft inserts spaces to the next tab stop (width from `tab-width`).
            KeyCode::Tab => {
                let style = self.doc().overrides.tab_style(&self.state.settings);
                let tw = self.doc().overrides.tab_width(&self.state.settings);
                doc_ops::apply_doc_edit_grouped(
                    &mut self.state.buffers,
                    &mut self.state.panes.state,
                    focused,
                    buf,
                    move |b, s| insert_tab(b, s, style, tw),
                );
            }

            // ── Newline ───────────────────────────────────────────────────────
            // Auto-indent: copy the current line's leading whitespace onto the
            // new line. No smart indent (tree-sitter indent.scm is a separate
            // roadmap milestone).
            KeyCode::Enter => {
                doc_ops::apply_doc_edit_grouped(
                    &mut self.state.buffers,
                    &mut self.state.panes.state,
                    focused,
                    buf,
                    insert_newline_indent,
                );
            }

            // ── Delete ────────────────────────────────────────────────────────
            KeyCode::Backspace => {
                let (ap_enabled, ap_pairs) =
                    self.doc().overrides.auto_pairs_ref(&self.state.settings);
                if self.should_dedent_backspace() {
                    let tw = self.doc().overrides.tab_width(&self.state.settings);
                    // Dedent: snap every cursor in leading whitespace back to
                    // the previous tab stop. All-or-nothing — if any cursor
                    // isn't in leading ws, the whole batch falls back.
                    doc_ops::apply_doc_edit_grouped(
                        &mut self.state.buffers,
                        &mut self.state.panes.state,
                        focused,
                        buf,
                        move |b, s| dedent_tab_backward(b, s, tw),
                    );
                } else if ap_enabled && self.is_between_pair(ap_pairs) {
                    // NLL: `ap_pairs` last used in the condition above; borrow ends here.
                    doc_ops::apply_doc_edit_grouped(
                        &mut self.state.buffers,
                        &mut self.state.panes.state,
                        focused,
                        buf,
                        delete_pair,
                    );
                } else {
                    doc_ops::apply_doc_edit_grouped(
                        &mut self.state.buffers,
                        &mut self.state.panes.state,
                        focused,
                        buf,
                        delete_char_backward,
                    );
                }
            }
            KeyCode::Delete => {
                doc_ops::apply_doc_edit_grouped(
                    &mut self.state.buffers,
                    &mut self.state.panes.state,
                    focused,
                    buf,
                    delete_char_forward,
                );
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
    /// [`leading_whitespace_end`] primitive, so this gate and
    /// [`hume_editing::lines::leading_whitespace`] agree on what counts.
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
            // rule from the byte-scan version: at exactly the end the cursor
            // sits on the first content char and still qualifies.
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
    fn is_between_pair(&self, pairs: &[crate::auto_pairs::Pair]) -> bool {
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
        pair: &crate::auto_pairs::Pair,
        ap_pairs: &[crate::auto_pairs::Pair],
    ) -> bool {
        let buf = self.doc().text();
        self.current_selections().iter_sorted().all(|sel| {
            !sel.is_collapsed()
                || crate::auto_pairs::should_auto_pair_at(buf, sel.head(), pair, ap_pairs)
        })
    }
}
