use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::auto_pairs::{delete_pair, insert_pair_close};
use crate::core::jump_list::{JumpEntry, JUMP_LINE_THRESHOLD};
use super::commands::{cmd_clear_search, search_sel};
use super::registry::MappableCommand;
use crate::core::selection::Selection;
use crate::ops::edit::{delete_char_backward, delete_char_forward, insert_char};
use crate::ops::motion::cmd_move_right;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::{compile_search_regex, find_next_match};
use crate::ops::selection_cmd::select_matches_within;

use super::keymap::WalkResult;
use engine::types::EditorMode;

use crate::ops::register::MACRO_REGISTER;

use super::{Editor, MacroPending, Mode, SearchDirection};

/// Commands that always record a jump before executing.
const JUMP_COMMANDS: &[&str] = &[
    "goto-first-line",
    "goto-last-line",
    "search-next",
    "search-prev",
    "extend-search-next",
    "extend-search-prev",
    "page-down",
    "page-up",
    "extend-page-down",
    "extend-page-up",
];

fn is_jump_command(name: &str) -> bool {
    JUMP_COMMANDS.contains(&name)
}

/// Valid register names for macro recording/replay: lowercase letters and digits.
///
/// This covers all registers defined by HUME's naming scheme. Special registers
/// like `b` (black hole) and `c` (clipboard) are technically valid register
/// chars and are accepted here — the register layer handles black-hole semantics.
fn is_valid_macro_register(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit()
}

/// Enqueue the keys stored in `reg` into the editor's replay queue.
///
/// The accumulated count (defaulting to 1) determines how many times the macro
/// is enqueued. Count is consumed and cleared. No-op when the register is empty,
/// unset, or holds text rather than a macro.
fn enqueue_macro_replay(ed: &mut Editor, reg: char) {
    let count = ed.count.take().unwrap_or(1);
    if let Some(keys) = ed.registers.read(reg).and_then(|r| r.as_macro()).map(|k| k.to_vec()) {
        for _ in 0..count {
            ed.replay_queue.extend(keys.iter().copied());
        }
    }
}


impl Editor {
    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        // Any keypress dismisses the previous transient status message.
        self.status_msg = None;
        match self.mode {
            Mode::Normal | Mode::Extend => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Command => self.handle_command(key),
            Mode::Search => self.handle_search(key),
            Mode::Select => self.handle_select(key),
        }

        // ── Macro recording ───────────────────────────────────────────────────
        // Runs after all mode handlers so Insert, Command, and Search keys
        // are captured. `skip_macro_record` excludes the stop `Q` itself.
        if let Some((_, ref mut keys)) = self.macro_recording {
            if !self.skip_macro_record {
                keys.push(key);
            }
        }
        self.skip_macro_record = false;
    }

    // ── Normal mode ───────────────────────────────────────────────────────────

    fn handle_normal(&mut self, key: KeyEvent) {
        // ── Kitty SHIFT normalization ─────────────────────────────────────────
        // The kitty keyboard protocol reports uppercase letters as Char('Q') with
        // KeyModifiers::SHIFT. For HUME's purposes the uppercase-ness is already
        // encoded in the char, so SHIFT is redundant and should be stripped — both
        // for the q/Q intercept below and for trie lookup (which stores bindings
        // as key!('Q') = Char('Q') + NONE).
        //
        // Only strip SHIFT when it is the *only* modifier (bare Shift+letter).
        // Ctrl+Shift combinations (e.g. Ctrl+X) keep their modifiers so they
        // match explicit Ctrl bindings in the keymap.
        let key = if key.modifiers == KeyModifiers::SHIFT {
            if let KeyCode::Char(ch) = key.code {
                if ch.is_alphabetic() {
                    KeyEvent::new(key.code, KeyModifiers::NONE)
                } else {
                    key
                }
            } else {
                key
            }
        } else {
            key
        };

        // ── Consume WaitChar argument ─────────────────────────────────────────
        // If a f/t/F/T/r binding fired on the previous keypress, `wait_char`
        // holds the command name to dispatch. The next character (any key)
        // becomes the argument — stored in `pending_char` for the command to read.
        if let Some(wc) = self.wait_char.take() {
            if let KeyCode::Char(ch) = key.code {
                let count = self.count.take().unwrap_or(1);
                self.pending_char = Some(ch);
                // Extend resolution: sticky extend (mode == Extend) OR one-shot
                // ctrl_extend carried into WaitCharPending from the original keypress.
                let extend = (self.mode == EditorMode::Extend) || wc.ctrl_extend;
                self.execute_keymap_command(wc.cmd_name, count, extend);
            }
            // Non-char key (e.g. Esc after pressing `f`): cancel the wait.
            // Clear count so a prefix like `3f<Esc>` doesn't leak into the next command.
            self.count = None;
            return;
        }

        // ── Hard-reset on Esc ─────────────────────────────────────────────────
        if key.code == KeyCode::Esc {
            self.pending_keys.clear();
            self.count = None;
            self.macro_pending = None; // cancel any pending q/Q register-name prompt
            // Esc exits Extend mode; Normal is the reset state.
            if self.mode == EditorMode::Extend {
                self.mode = EditorMode::Normal;
            }
            cmd_clear_search(self, 0);
            return;
        }

        // ── Macro pending: consume register-name key ──────────────────────────
        // After `Q` or `q`, the next keypress names the register.
        //
        // Record (`Q`): next key must be a valid register name (a-z, 0-9).
        //   Esc cancels; anything else cancels.
        //
        // Replay (`q`): next key selects the register.
        //   `qq` → replay from the default register `q` (mirrors `QQ` for recording).
        //   `q<reg>` → replay from the named register (e.g. `q3`).
        //   Any other key → cancel silently (key is swallowed).
        if let Some(pending) = self.macro_pending.take() {
            match pending {
                MacroPending::Record => {
                    match key.code {
                        // `QQ` — record into the default register. `Q` is uppercase
                        // so is_valid_macro_register won't catch it; handle explicitly.
                        KeyCode::Char('Q') => {
                            self.macro_recording = Some((MACRO_REGISTER, Vec::new()));
                            self.skip_macro_record = true;
                        }
                        KeyCode::Char(reg) if is_valid_macro_register(reg) => {
                            self.macro_recording = Some((reg, Vec::new()));
                            self.skip_macro_record = true;
                        }
                        // Esc, Ctrl-C, non-Char, or invalid Char — cancel.
                        _ => {}
                    }
                    return;
                }
                MacroPending::Replay => {
                    match key.code {
                        // `q<reg>` — replay from named register (includes `qq` since
                        // `q` is a valid lowercase register name → replays from `q`).
                        KeyCode::Char(ch) if is_valid_macro_register(ch) => {
                            enqueue_macro_replay(self, ch);
                        }
                        // Any other key (Esc, non-register, etc.) — cancel silently.
                        _ => {}
                    }
                    return;
                }
            }
        }

        // ── Count prefix accumulation ─────────────────────────────────────────
        // Only accumulate when we're at the trie root (no pending sequence)
        // and no modifiers are held (Ctrl+4 is not a count digit).
        // `0` without an existing count is the goto-line-start binding, not a digit.
        // NOTE: this runs AFTER macro_pending so that `Q1`/`q1` treat `1` as a
        // register name, not as a count digit.
        if self.pending_keys.is_empty() && key.modifiers == KeyModifiers::NONE {
            match key.code {
                KeyCode::Char(d @ '1'..='9') => {
                    let n = self.count.unwrap_or(0) * 10 + (d as usize - '0' as usize);
                    self.count = Some(n);
                    return;
                }
                KeyCode::Char('0') if self.count.is_some() => {
                    self.count = self.count.map(|c| c * 10);
                    return;
                }
                _ => {}
            }
        }

        // ── `Q` / `q` intercept (bare key, at trie root, no modifiers) ────────
        // `Q` toggles recording; `q` triggers replay. Recording uses uppercase
        // because you do it once; replay uses lowercase because you do it often.
        // Both are suppressed while a replay is in progress to prevent nesting.
        if self.pending_keys.is_empty() && key.modifiers == KeyModifiers::NONE {
            match key.code {
                KeyCode::Char('Q') => {
                    if let Some((reg, keys)) = self.macro_recording.take() {
                        self.registers.write_macro(reg, keys);
                    } else if !self.is_replaying {
                        self.macro_pending = Some(MacroPending::Record);
                    }
                    // During replay: silently ignore (no nested recording).
                    return;
                }
                KeyCode::Char('q') => {
                    if !self.is_replaying && self.macro_recording.is_none() {
                        // Replay: wait for the register-name key.
                        self.macro_pending = Some(MacroPending::Replay);
                    }
                    // During recording or replay: silently ignore.
                    return;
                }
                _ => {}
            }
        }

        // ── Ctrl-key normalisation ────────────────────────────────────────────
        //
        // Two categories of CONTROL keys:
        //
        // 1. Explicit Ctrl bindings (Ctrl+c, Ctrl+r, Ctrl+,, Ctrl+x, Ctrl+X):
        //    Have a dedicated trie entry. Used as-is regardless of kitty mode.
        //
        // 2. Implicit Ctrl+motion (Ctrl+h/j/k/l/w/b and similar motion keys):
        //    No explicit trie binding. With kitty keyboard protocol enabled,
        //    these become one-shot extend: strip CONTROL, look up the bare key,
        //    and dispatch with extend=true (if the command has an extend variant).
        //    Without kitty, these are a no-op — legacy terminals can't
        //    distinguish Ctrl+letter from control codes reliably, so silently
        //    running the bare motion would be surprising.
        //
        // Detection: try the key as-is in the trie first. If NoMatch and the key
        // had CONTROL, strip CONTROL and retry only if kitty is enabled.
        //
        // REPORT_ALTERNATE_KEYS (enabled at init) makes the terminal send the
        // shifted character directly — crossterm replaces the base keycode with
        // the alternate and strips SHIFT. So Ctrl+} arrives as Char('}') with
        // just CONTROL, and stripping CONTROL gives us the correct bare key.
        // This is layout-independent: the terminal knows the real keyboard layout.

        // Trie walk + Ctrl normalisation in one pass.
        //
        // For Ctrl keys at the trie root, walk once to check for an explicit
        // binding. If found, reuse that result directly (no second walk).
        // If NoMatch, strip CONTROL and re-walk only on kitty terminals.
        let (result, ctrl_extend) =
            if key.modifiers.contains(KeyModifiers::CONTROL) && self.pending_keys.is_empty() {
                match self.keymap.normal.walk(&[key]) {
                    WalkResult::NoMatch if self.kitty_enabled => {
                        // Kitty mode: strip CONTROL, re-walk as extend.
                        let bare = KeyEvent::new(key.code, KeyModifiers::NONE);
                        self.pending_keys.push(bare);
                        (self.keymap.normal.walk(&self.pending_keys), true)
                    }
                    WalkResult::NoMatch => return, // Legacy: no-op.
                    matched => (matched, false),   // Explicit Ctrl binding — reuse.
                }
            } else {
                self.pending_keys.push(key);
                (self.keymap.normal.walk(&self.pending_keys), false)
            };

        // Compute the effective extend flag: sticky extend (mode == Extend) OR
        // kitty one-shot (ctrl_extend local). Passed as a parameter — no mode change.
        let extend = (self.mode == EditorMode::Extend) || ctrl_extend;

        // Ctrl one-shot extend guard: only dispatch if the command has an
        // extend variant. Prevents e.g. Ctrl+u from running "undo" (which
        // has no extend variant and is not a motion).
        if ctrl_extend {
            let name = match &result {
                WalkResult::Leaf(cmd) => Some(cmd.name),
                WalkResult::WaitChar(wc) => Some(wc.cmd_name),
                _ => None,
            };
            if let Some(n) = name
                && self.registry.extend_variant(n).is_none()
            {
                self.pending_keys.clear();
                self.count = None;
                return;
            }
        }

        match result {
            WalkResult::Leaf(cmd) => {
                self.pending_keys.clear();
                let raw_count = self.count.take();
                self.explicit_count = raw_count.is_some();
                let count = raw_count.unwrap_or(1);
                self.execute_keymap_command(cmd.name, count, extend);
                self.explicit_count = false;
            }
            WalkResult::WaitChar(mut wc) => {
                self.pending_keys.clear();
                // Carry ctrl_extend into WaitCharPending so extend resolution
                // happens at char-consumption time.
                wc.ctrl_extend = ctrl_extend;
                self.wait_char = Some(wc);
            }
            WalkResult::Interior { .. } => {
                // More keys needed. pending_keys stays populated.
            }
            WalkResult::NoMatch => {
                self.pending_keys.clear();
                self.count = None;
            }
        }
    }

    // ── Insert mode ───────────────────────────────────────────────────────────

    pub(super) fn handle_insert(&mut self, key: KeyEvent) {
        // Walk the insert trie first: handles Esc, Ctrl+C, and arrow keys.
        // Regular characters (Char without CONTROL) and Backspace/Delete/Enter
        // are NOT in the insert trie — they're handled below.
        let trie_result = self.keymap.insert.walk(&[key]);
        match trie_result {
            WalkResult::Leaf(cmd) => {
                self.execute_keymap_command(cmd.name, 1, false);
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
        if let Some(ref mut session) = self.insert_session {
            session.keystrokes.push(key);
        }

        // ── Character input ───────────────────────────────────────────────────
        match key.code {
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.auto_pairs.enabled {
                    if let Some(pair) = self.auto_pairs.pair_for_open(ch) {
                        let (open, close, symmetric) = (pair.open, pair.close, pair.is_symmetric());
                        if symmetric && self.should_skip_close(ch) {
                            // e.g. typing `"` when cursor already sits on `"`.
                            self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                        } else {
                            // Auto-close or wrap-selection.
                            self.doc.apply_edit_grouped(|b, s| insert_pair_close(b, s, open, close));
                        }
                    } else if self.auto_pairs.pair_for_close(ch).is_some()
                        && self.should_skip_close(ch)
                    {
                        // Asymmetric close (e.g. `)`) when cursor is already on it.
                        self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                    } else {
                        self.doc.apply_edit_grouped(|b, s| insert_char(b, s, ch));
                    }
                } else {
                    self.doc.apply_edit_grouped(|b, s| insert_char(b, s, ch));
                }
            }

            // ── Newline ───────────────────────────────────────────────────────
            KeyCode::Enter => {
                self.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
            }

            // ── Delete ────────────────────────────────────────────────────────
            KeyCode::Backspace => {
                if self.auto_pairs.enabled && self.is_between_pair() {
                    self.doc.apply_edit_grouped(delete_pair);
                } else {
                    self.doc.apply_edit_grouped(delete_char_backward);
                }
            }
            KeyCode::Delete => {
                self.doc.apply_edit_grouped(delete_char_forward);
            }

            _ => {}
        }
    }

    // ── Command execution ─────────────────────────────────────────────────────

    /// Execute a named command with the given count and extend flag.
    ///
    /// When `extend` is true, looks up the extend variant in the registry
    /// and dispatches that instead. Falls back to the base command if no
    /// extend variant exists.
    ///
    /// [`CommandRegistry`]: super::registry::CommandRegistry
    pub(super) fn execute_keymap_command(&mut self, name: &'static str, count: usize, extend: bool) {
        // Resolve extend-mode duality via the registry. When extend is active,
        // use the extend variant if one exists; otherwise fall back to the base.
        let resolved = if extend {
            self.registry.extend_variant(name).unwrap_or(name)
        } else {
            name
        };

        if let Some(reg_cmd) = self.registry.get(resolved).cloned() {
            // Snapshot pending_char before dispatch — commands consume it via `.take()`.
            let char_arg = self.pending_char;

            // ── Jump list: capture pre-command state ─────────────────────────
            // Motions, explicit jump commands, and vertical visual-line EditorCmds
            // can all produce large enough line jumps to warrant a jump entry.
            let is_explicit_jump = is_jump_command(resolved);
            let is_vertical_visual = matches!(resolved, "move-down" | "move-up" | "extend-down" | "extend-up");
            let pre_jump = if is_explicit_jump || is_vertical_visual || matches!(reg_cmd, MappableCommand::Motion { .. }) {
                let line = self.doc.buf().char_to_line(self.doc.sels().primary().head);
                Some((self.doc.sels().clone(), line))
            } else {
                None
            };

            match reg_cmd {
                MappableCommand::Motion { fun, .. } => {
                    // Motion functions take (buf, sels, count). count defaults to 1
                    // if the user typed no prefix.
                    self.apply_motion(|b, s| fun(b, s, count));
                }
                MappableCommand::Selection { fun, .. } => {
                    // Selection / text-object functions don't take count.
                    self.apply_motion(fun);
                }
                MappableCommand::Edit { fun, .. } => {
                    self.doc.apply_edit(fun);
                }
                MappableCommand::EditorCmd { fun, .. } => {
                    fun(self, count);
                }
            }

            // ── Jump list: record if this was a jump ─────────────────────────
            if let Some((pre_sels, pre_line)) = pre_jump {
                let post_line = self.doc.buf().char_to_line(self.doc.sels().primary().head);
                if is_explicit_jump || pre_line.abs_diff(post_line) > JUMP_LINE_THRESHOLD {
                    self.jump_list.push(JumpEntry { selections: pre_sels, primary_line: pre_line });
                }
            }

            // Reset the sticky display column unless this was a vertical visual-line command.
            // Any horizontal motion, edit, or mode change should clear it so the next
            // j/k press re-latches to the cursor's actual position.
            if !is_vertical_visual {
                self.preferred_display_cols.clear();
            }

            // Record repeatable actions for `.` replay.
            // Skips non-repeatable commands (motions, selections, undo, etc.).
            // During replay `cmd_repeat` restores `last_action` after the fact,
            // so any transient overwrite here is harmless.
            if reg_cmd.is_repeatable() {
                self.last_action = Some(super::RepeatableAction {
                    command: resolved,
                    count,
                    char_arg,
                    insert_keys: Vec::new(),
                });
            }
        }
    }
    // ── Auto-pair helpers ─────────────────────────────────────────────────────

    /// Returns `true` if every selection is a cursor AND the character at each
    /// cursor's `head` equals `ch`.
    ///
    /// All-or-nothing: if even one cursor doesn't match, the whole operation
    /// falls back to normal insert, keeping multi-cursor behavior consistent.
    fn should_skip_close(&self, ch: char) -> bool {
        self.doc.sels().iter_sorted().all(|sel| {
            sel.is_collapsed() && self.doc.buf().char_at(sel.head) == Some(ch)
        })
    }

    /// Returns `true` if every selection is a cursor AND the pair
    /// `(char_before_cursor, char_at_cursor)` matches a configured pair.
    ///
    /// Used by Backspace to decide whether to delete both brackets or just one.
    fn is_between_pair(&self) -> bool {
        let buf = self.doc.buf();
        let pairs = &self.auto_pairs.pairs;
        self.doc.sels().iter_sorted().all(|sel| {
            if !sel.is_collapsed() || sel.head == 0 {
                return false;
            }
            // prev_grapheme_boundary handles multi-codepoint clusters; bracket/quote
            // chars are always single codepoints, but using it keeps the logic uniform.
            let prev = crate::core::grapheme::prev_grapheme_boundary(buf, sel.head);
            match (buf.char_at(prev), buf.char_at(sel.head)) {
                (Some(before), Some(at)) => {
                    pairs.iter().any(|p| p.open == before && p.close == at)
                }
                _ => false,
            }
        })
    }

    // ── Selection helpers ─────────────────────────────────────────────────────

    /// Replace the primary selection and merge any resulting overlaps.
    ///
    /// If the new selection overlaps an existing secondary, both are merged
    /// into one — so the total selection count may decrease.
    pub(super) fn set_primary_selection(&mut self, new_sel: Selection) {
        let idx = self.doc.sels().primary_index();
        let new_sels = self.doc.sels().clone().replace(idx, new_sel).merge_overlapping();
        self.doc.set_selections(new_sels);
    }

    // ── Snapshot restore helpers ────────────────────────────────────────────────

    /// Restore selections from the search-mode snapshot without consuming it.
    fn restore_search_snapshot(&mut self) {
        if let Some(ref sels) = self.search.pre_search_sels {
            self.doc.set_selections(sels.clone());
        }
    }

    /// Restore selections from the select-mode snapshot without consuming it.
    fn restore_select_snapshot(&mut self) {
        if let Some(ref sels) = self.pre_select_sels {
            self.doc.set_selections(sels.clone());
        }
    }

    // ── Search mode ───────────────────────────────────────────────────────────

    fn handle_search(&mut self, key: KeyEvent) {
        use super::MiniBufferEvent;
        let event = match self.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        match event {
            MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => self.cancel_search(),
            MiniBufferEvent::Confirm(pattern) => {
                // Persist pattern in 's' register for future n/N.
                self.registers.write_text(SEARCH_REGISTER, vec![pattern]);
                // Record the pre-search position in the jump list before
                // discarding it — the search moved the cursor to the match.
                if let Some(sels) = self.search.pre_search_sels.take() {
                    self.jump_list.push(JumpEntry::new(sels, self.doc.buf()));
                }
                // search.regex stays alive for immediate n/N without recompile.
                // set_mode does not touch search state, so it is safe to call here.
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            MiniBufferEvent::EmptiedByBackspace => {
                // Restore position when pattern is fully erased, but stay in Search mode.
                self.restore_search_snapshot();
                self.search.set_regex(None);
            }
            MiniBufferEvent::Edited => self.update_live_search(),
            MiniBufferEvent::CursorMoved | MiniBufferEvent::Ignored => {}
        }
    }

    /// Cancel search: restore pre-search position, clear all search state, return to Normal.
    fn cancel_search(&mut self) {
        if let Some(sels) = self.search.pre_search_sels.take() {
            self.doc.set_selections(sels);
        }
        self.search.clear();
        self.mode = Mode::Normal;
        self.minibuf = None;
    }

    /// Recompile the regex from the current mini-buffer input and jump to the
    /// first match from the pre-search position.
    ///
    /// Called on every keystroke while in Search mode.
    fn update_live_search(&mut self) {
        let pattern = match self.minibuf.as_ref() {
            Some(mb) if !mb.input.is_empty() => mb.input.clone(),
            _ => return,
        };

        let Some(regex) = compile_search_regex(&pattern) else {
            // Invalid regex in progress — don't move; just clear cached regex.
            self.search.set_regex(None);
            return;
        };

        let direction = self.search.direction;

        // Start from the original pre-search position (not the current position),
        // so each additional character refines from the same anchor point.
        let from_char = match &self.search.pre_search_sels {
            Some(sels) => {
                let buf = self.doc.buf();
                let primary = sels.primary();
                match direction {
                    SearchDirection::Forward => primary.start(),
                    SearchDirection::Backward => primary.end_inclusive(buf),
                }
            }
            None => 0,
        };

        match find_next_match(self.doc.buf(), &regex, from_char, direction) {
            Some((start, end_incl, _wrapped)) => {
                let anchor = if self.search.extend {
                    // Extend from the original anchor.
                    Some(self.search.pre_search_sels.as_ref().map(|s| s.primary().anchor).unwrap_or(start))
                } else {
                    None
                };
                self.set_primary_selection(search_sel(start, end_incl, anchor, direction));
            }
            None => {
                // No match — restore position to pre-search.
                self.restore_search_snapshot();
            }
        }

        self.search.set_regex(Some(regex));
    }

    // ── Select mode (s) ────────────────────────────────────────────────────────

    fn handle_select(&mut self, key: KeyEvent) {
        use super::MiniBufferEvent;
        let event = match self.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        match event {
            MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => self.cancel_select(),
            MiniBufferEvent::Confirm(_) => {
                // Keep the selections that live preview already set.
                self.pre_select_sels = None;
                // Do NOT write to SEARCH_REGISTER or clear search state —
                // select-within is a selection op, not a search. The previous
                // search pattern and its highlights should be preserved so that
                // n/N continues to navigate the original search.
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            MiniBufferEvent::EmptiedByBackspace => {
                // Restore original selections when pattern is fully erased.
                self.restore_select_snapshot();
            }
            MiniBufferEvent::Edited => self.update_live_select(),
            MiniBufferEvent::CursorMoved | MiniBufferEvent::Ignored => {}
        }
    }

    /// Cancel select mode: restore original selections, return to Normal.
    fn cancel_select(&mut self) {
        if let Some(sels) = self.pre_select_sels.take() {
            self.doc.set_selections(sels);
        }
        // Do not clear search state — the previous search should survive a
        // cancelled select-within.
        self.mode = Mode::Normal;
        self.minibuf = None;
    }

    /// Recompile the regex and replace selections with matches within the
    /// original selections. Called on every keystroke in Select mode.
    fn update_live_select(&mut self) {
        let pattern = match self.minibuf.as_ref() {
            Some(mb) if !mb.input.is_empty() => mb.input.clone(),
            _ => return,
        };

        let Some(regex) = compile_search_regex(&pattern) else {
            // Invalid regex in progress — restore originals.
            self.restore_select_snapshot();
            return;
        };

        // Compute matches in a limited scope so the borrow on
        // pre_select_sels is released before we need to restore.
        let result = self.pre_select_sels.as_ref().and_then(|sels| {
            select_matches_within(self.doc.buf(), sels, &regex)
        });

        match result {
            Some(new_sels) => self.doc.set_selections(new_sels),
            None => self.restore_select_snapshot(),
        }
    }

    // ── Command mode ──────────────────────────────────────────────────────────

    fn handle_command(&mut self, key: KeyEvent) {
        use super::MiniBufferEvent;
        let event = match self.minibuf.as_mut() {
            Some(mb) => mb.handle_key(key),
            None => return,
        };
        match event {
            MiniBufferEvent::Cancel => {
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            MiniBufferEvent::Confirm(_) | MiniBufferEvent::ConfirmEmpty => {
                self.execute_command();
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            // Backspace at column 0 or on the last character cancels (Kakoune behaviour).
            MiniBufferEvent::EmptiedByBackspace => {
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            MiniBufferEvent::Edited | MiniBufferEvent::CursorMoved | MiniBufferEvent::Ignored => {}
        }
    }

    /// Execute the command currently in the mini-buffer.
    ///
    /// Called just before the mini-buffer is cleared and mode returns to Normal.
    fn execute_command(&mut self) {
        let input = self
            .minibuf
            .as_ref()
            .map(|m| m.input.trim().to_owned())
            .unwrap_or_default();

        // Split into command name and optional argument (e.g. "w foo.txt" → "w" + "foo.txt").
        // input is already trimmed, so splitting on the first space is sufficient.
        let (cmd_raw, arg) = match input.split_once(' ') {
            Some((c, a)) => (c, Some(a.trim())),
            None => (input.as_str(), None),
        };

        // Parse trailing `!` once so commands can opt in to force semantics.
        let (cmd, force) = match cmd_raw.strip_suffix('!') {
            Some(base) => (base, true),
            None => (cmd_raw, false),
        };

        match super::commands::find_typed_command(cmd) {
            Some(tc) => (tc.fun)(self, arg, force),
            None => { self.status_msg = Some(format!("Unknown command: {cmd}")); }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard: every command whose name starts with a jump-related prefix must
    /// appear in `JUMP_COMMANDS`. Catches silent omissions when new jump-worthy
    /// commands are added to the registry.
    #[test]
    fn jump_commands_list_is_complete() {
        let reg = super::super::registry::CommandRegistry::with_defaults();

        // Prefixes that indicate a command should always record a jump.
        let jump_prefixes = ["goto-first-line", "goto-last-line", "search-next", "search-prev",
                             "extend-search-next", "extend-search-prev",
                             "page-down", "page-up", "extend-page-down", "extend-page-up"];

        for prefix in &jump_prefixes {
            assert!(
                reg.get(prefix).is_some(),
                "JUMP_COMMANDS references '{prefix}' which is not in the registry"
            );
            assert!(
                is_jump_command(prefix),
                "'{prefix}' is registered but missing from JUMP_COMMANDS"
            );
        }

        // Reverse check: every entry in JUMP_COMMANDS must exist in the registry.
        for &name in JUMP_COMMANDS {
            assert!(
                reg.get(name).is_some(),
                "JUMP_COMMANDS contains '{name}' which is not in the registry"
            );
        }
    }
}
