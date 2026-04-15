use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::auto_pairs::{delete_pair, insert_pair_close};
use crate::core::jump_list::JumpEntry;
use super::commands::{cmd_clear_search, search_sel};
use super::registry::MappableCommand;
use crate::core::selection::Selection;
use crate::ops::edit::{delete_char_backward, delete_char_forward, insert_char};
use crate::ops::motion::cmd_move_right;
use crate::ops::MotionMode;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::{compile_search_regex, find_next_match};
use crate::ops::selection_cmd::select_matches_within;

use super::keymap::{WaitCharPending, WalkResult};
use super::Severity;
use engine::types::EditorMode;

use crate::ops::register::MACRO_REGISTER;

use super::{Editor, MacroPending, Mode, SearchDirection};


/// Valid register names for macro recording/replay: `q` (default) and `0`–`9`.
///
/// `q` is the default macro register (`QQ`/`qq`). The digits `0`–`9` are the
/// named storage registers. Other letters (special registers like `b`, `c`, `s`)
/// are not valid macro targets.
fn is_valid_macro_register(ch: char) -> bool {
    ch == MACRO_REGISTER || ch.is_ascii_digit()
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

        // ── Scratch view intercept ────────────────────────────────────────────
        // When a scratch buffer is open (e.g. `:messages`), intercept all keys
        // for navigation and dismissal. The real document is left untouched.
        if self.scratch_view.is_some() {
            self.handle_scratch_key(key);
            return;
        }

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
        if let Some((_, ref mut keys)) = self.macro_recording
            && !self.skip_macro_record
        {
            keys.push(key);
        }
        self.skip_macro_record = false;
    }

    // ── Scratch view mode ─────────────────────────────────────────────────────

    /// Handle a keypress while a scratch buffer (`:messages`, `:help`, …) is open.
    ///
    /// Only navigation and dismissal are supported. All other keys are silently
    /// swallowed so the real document cannot be accidentally modified.
    fn handle_scratch_key(&mut self, key: KeyEvent) {
        use KeyCode::{Char, Esc, Down, Up};
        use crate::ops::motion::{cmd_select_line, cmd_select_line_backward, cmd_goto_first_line, cmd_goto_last_line};

        let sv = self.scratch_view.as_mut().expect("called only when scratch_view is Some");
        match key.code {
            Char('q') | Esc => {
                self.scratch_view = None;
            }
            Char('j') | Down => {
                sv.sels = cmd_select_line(&sv.buf, sv.sels.clone(), MotionMode::Move);
            }
            Char('k') | Up => {
                sv.sels = cmd_select_line_backward(&sv.buf, sv.sels.clone(), MotionMode::Move);
            }
            Char('g') => {
                sv.sels = cmd_goto_first_line(&sv.buf, sv.sels.clone(), 1, MotionMode::Move);
            }
            Char('G') => {
                sv.sels = cmd_goto_last_line(&sv.buf, sv.sels.clone(), 1, MotionMode::Move);
            }
            _ => {} // swallow all other keys
        }
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
                self.execute_keymap_command(wc.cmd_name.clone(), count, extend);
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
            let _ = cmd_clear_search(self, 0, MotionMode::Move);
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
                    // A count prefix before `Q<reg>` has no meaning for recording.
                    // Clear it so it doesn't leak into the first key typed during
                    // the session (which would fire with count N instead of 1).
                    self.count = None;
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

        // ── Extend resolution overview ────────────────────────────────────────
        //
        // "Should this command extend?" is answered in three stages, because
        // extend depends on *which command* was resolved, and the Ctrl path
        // changes which key is looked up — so we can't separate extend
        // resolution from trie walking.
        //
        //  Stage 1 (extend-trie override, below):
        //      In sticky extend mode, try the extend trie first. It maps keys
        //      to *replacement* commands (e.g. `o → flip-selections` instead
        //      of `o → open-below`), dispatched with extend = false. A miss
        //      falls through to the normal trie.
        //
        //  Stage 2 (Ctrl normalisation, further below):
        //      Ctrl+key may strip CONTROL and re-walk with the bare key
        //      (kitty one-shot extend). Whether to extend depends on whether
        //      the *resolved bare-key command* is extendable — we don't know
        //      that until the trie walk completes, so is_extendable() runs
        //      here, producing `ctrl_extend`.
        //
        //  Stage 3 (final merge, after the trie walk):
        //      Merges the two extend sources: sticky mode (EditorMode::Extend)
        //      and one-shot Ctrl (ctrl_extend). This is the earliest point
        //      where both inputs are available.

        // ── Stage 1: Extend-trie override ────────────────────────────────────
        //
        // We walk with [pending_keys..., key] without committing the push yet —
        // only `Interior` commits the key (so the sequence accumulates correctly
        // across keypresses). On `NoMatch` the key is not yet in `pending_keys`,
        // so the normal-trie path below can push it as usual.
        if self.mode == EditorMode::Extend && !key.modifiers.contains(KeyModifiers::CONTROL) {
            let mut seq = self.pending_keys.clone();
            seq.push(key);
            match self.keymap.extend.walk(&seq) {
                WalkResult::Leaf(cmd) => {
                    self.pending_keys.clear();
                    let count = self.count.take().unwrap_or(1);
                    self.explicit_count = false;
                    self.execute_keymap_command(cmd.name.clone(), count, false);
                    return;
                }
                WalkResult::Interior { .. } => {
                    // Mid-sequence — commit the key and wait for more.
                    self.pending_keys.push(key);
                    return;
                }
                WalkResult::WaitChar(wc) => {
                    self.wait_char = Some(wc);
                    return;
                }
                WalkResult::NoMatch => {
                    // No extend-trie match — fall through to normal trie.
                }
            }
        }

        // ── Stage 2: Ctrl-key normalisation + one-shot extend ────────────────
        //
        // `ctrl_extend` is computed here — alongside the trie walk — because
        // it depends on which command the key resolves to, and the Ctrl path
        // changes what key is walked. Separating extend resolution from the
        // trie walk would require walking twice or caching the result.
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
                        // Kitty mode: strip CONTROL, re-walk as extend. Only proceed if the
                        // resolved command is extendable — prevents e.g. Ctrl+u running
                        // "undo" (not a motion) as a one-shot extend.
                        let bare = KeyEvent::new(key.code, KeyModifiers::NONE);
                        self.pending_keys.push(bare);
                        let result = self.keymap.normal.walk(&self.pending_keys);
                        let is_extendable = match &result {
                            WalkResult::Leaf(c) => self.registry.get_mappable(c.name.as_ref()).map_or(false, |r| r.is_extendable()),
                            WalkResult::WaitChar(wc) => self.registry.get_mappable(wc.cmd_name.as_ref()).map_or(false, |r| r.is_extendable()),
                            _ => false,
                        };
                        if !is_extendable {
                            self.pending_keys.clear();
                            self.count = None;
                            return;
                        }
                        (result, true)
                    }
                    WalkResult::NoMatch => return, // Legacy: no-op.
                    // Explicit Ctrl+letter binding. Treat as extend if the command
                    // is extendable (e.g. Ctrl+x → select-line always extends).
                    matched => {
                        let ctrl_extend = match &matched {
                            WalkResult::Leaf(c) => self.registry.get_mappable(c.name.as_ref()).map_or(false, |r| r.is_extendable()),
                            WalkResult::WaitChar(wc) => self.registry.get_mappable(wc.cmd_name.as_ref()).map_or(false, |r| r.is_extendable()),
                            _ => false,
                        };
                        (matched, ctrl_extend)
                    }
                }
            } else {
                self.pending_keys.push(key);
                (self.keymap.normal.walk(&self.pending_keys), false)
            };

        // ── Stage 3: Final extend merge ───────────────────────────────────────
        //
        // Both inputs are now available: sticky extend from editor mode, and
        // one-shot extend from the Ctrl path (ctrl_extend). Merge them here.
        // `extend` is passed as a parameter — no mode transition occurs.
        let extend = (self.mode == EditorMode::Extend) || ctrl_extend;

        match result {
            WalkResult::Leaf(cmd) => {
                self.pending_keys.clear();
                let raw_count = self.count.take();
                self.explicit_count = raw_count.is_some();
                let count = raw_count.unwrap_or(1);
                self.execute_keymap_command(cmd.name.clone(), count, extend);
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

                // Pair-wrap: unbound pair-open key with a non-collapsed selection → wrap.
                if let KeyCode::Char(ch) = key.code {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        let (ap_enabled, ap_pairs) = self.doc.overrides.auto_pairs_ref(&self.settings);
                        if ap_enabled {
                            if let Some(pair) = ap_pairs.iter().find(|p| p.open == ch) {
                                let has_selection = self.doc.sels().iter_sorted().any(|s| !s.is_collapsed());
                                if has_selection {
                                    let (open, close) = (pair.open, pair.close);
                                    self.doc.apply_edit(|b, s| insert_pair_close(b, s, open, close));
                                }
                            }
                        }
                    }
                }
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
                self.execute_keymap_command(cmd.name.clone(), 1, false);
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
                let (ap_enabled, ap_pairs) = self.doc.overrides.auto_pairs_ref(&self.settings);
                if ap_enabled {
                    if let Some(pair) = ap_pairs.iter().find(|p| p.open == ch) {
                        let (open, close, symmetric) = (pair.open, pair.close, pair.is_symmetric());
                        if symmetric && self.should_skip_close(ch) {
                            // e.g. typing `"` when cursor already sits on `"`.
                            self.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
                        } else if self.should_auto_pair(pair, ap_pairs) {
                            // Context is clear: insert open+close or wrap selection.
                            self.doc.apply_edit_grouped(|b, s| insert_pair_close(b, s, open, close));
                        } else {
                            // Next char is a word char (or symmetric prev is word char):
                            // insert only the typed character.
                            self.doc.apply_edit_grouped(|b, s| insert_char(b, s, ch));
                        }
                    } else if ap_pairs.iter().any(|p| p.close == ch && !p.is_symmetric())
                        && self.should_skip_close(ch)
                    {
                        // Asymmetric close (e.g. `)`) when cursor is already on it.
                        self.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
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
                let (ap_enabled, ap_pairs) = self.doc.overrides.auto_pairs_ref(&self.settings);
                if ap_enabled && self.is_between_pair(ap_pairs) {
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
    /// `extend` is converted to `MotionMode::Extend` / `MotionMode::Move` and
    /// passed to the command function. The command itself decides what to do
    /// with the mode — motions and selections branch on it; edits ignore it.
    pub(super) fn execute_keymap_command(&mut self, name: Cow<'static, str>, count: usize, extend: bool) {
        let Some(reg_cmd) = self.registry.get_mappable(name.as_ref()).cloned() else {
            self.report(Severity::Warning, format!("unknown command: {name}"));
            return;
        };
        {
            // Snapshot pending_char before dispatch — commands consume it via `.take()`.
            let char_arg = self.pending_char;

            // ── Jump list: capture pre-command state ─────────────────────────
            // Motions, explicit jump commands, and vertical visual-line EditorCmds
            // can all produce large enough line jumps to warrant a jump entry.
            let is_explicit_jump = reg_cmd.is_jump();
            let is_vertical_visual = reg_cmd.is_visual_move();
            let pre_jump = if is_explicit_jump || is_vertical_visual || matches!(reg_cmd, MappableCommand::Motion { .. }) {
                let line = self.doc.buf().char_to_line(self.doc.sels().primary().head);
                Some((self.doc.sels().clone(), line))
            } else {
                None
            };

            let motion_mode = if extend { MotionMode::Extend } else { MotionMode::Move };

            match reg_cmd {
                MappableCommand::Motion { fun, .. } => {
                    // Motion functions take (buf, sels, count, mode). count defaults to 1
                    // if the user typed no prefix.
                    self.apply_motion(|b, s| fun(b, s, count, motion_mode));
                }
                MappableCommand::Selection { fun, .. } => {
                    // Selection / text-object functions don't take count.
                    self.apply_motion(|b, s| fun(b, s, motion_mode));
                }
                MappableCommand::Edit { fun, .. } => {
                    self.doc.apply_edit(fun);
                }
                MappableCommand::EditorCmd { fun, .. } => {
                    if let Err(e) = fun(self, count, motion_mode) {
                        self.report(Severity::Error, e.0);
                    }
                }
                MappableCommand::SteelBacked { ref steel_proc, .. } => {
                    let (queue, wait_char_cmd) = if let Some(host) = self.scripting.as_mut() {
                        match host.call_steel_cmd(&steel_proc) {
                            Ok(r) => r,
                            Err(e) => { self.report(Severity::Error, e); return; }
                        }
                    } else {
                        return;
                    };
                    for cmd_name in queue {
                        self.execute_keymap_command(cmd_name.into(), count, extend);
                    }
                    if let Some(wc) = wait_char_cmd {
                        self.wait_char = Some(WaitCharPending {
                            cmd_name: wc.into(),
                            ctrl_extend: false,
                        });
                    }
                }
            }

            // ── Jump list: record if this was a jump ─────────────────────────
            if let Some((pre_sels, pre_line)) = pre_jump {
                let post_line = self.doc.buf().char_to_line(self.doc.sels().primary().head);
                if is_explicit_jump || pre_line.abs_diff(post_line) > self.settings.jump_line_threshold {
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
                    command: name.clone(),
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
    fn is_between_pair(&self, pairs: &[crate::auto_pairs::Pair]) -> bool {
        let buf = self.doc.buf();
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

    /// Returns `true` if auto-pairing `pair` is appropriate given the current
    /// selections. All-or-nothing: every collapsed selection must satisfy the
    /// context rules; non-collapsed selections always pass (they wrap).
    fn should_auto_pair(&self, pair: &crate::auto_pairs::Pair, ap_pairs: &[crate::auto_pairs::Pair]) -> bool {
        let buf = self.doc.buf();
        self.doc.sels().iter_sorted().all(|sel| {
            !sel.is_collapsed()
                || crate::auto_pairs::should_auto_pair_at(buf, sel.head, pair, ap_pairs)
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

        if let Some(tc) = self.registry.get_typed(cmd) {
            let fun = tc.fun;
            if let Err(e) = fun(self, arg, force) {
                self.report(Severity::Error, e.0);
            }
        } else if self.registry.get_mappable(cmd).is_some() {
            // Any mappable command can be invoked from the command line with
            // an implicit count of 1. This means `:clear-search`, `:undo`, etc.
            // all work without needing typed-command wrappers.
            // `cmd` is already the canonical name — no need to clone the command.
            self.execute_keymap_command(cmd.to_owned().into(), 1, false);
        } else {
            self.report(Severity::Warning, format!("Unknown command: {cmd}"));
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Guard: every jump command has `is_jump() == true` in the registry.
    ///
    /// The registry is the single source of truth — there is no separate
    /// `JUMP_COMMANDS` list to keep in sync.
    #[test]
    fn jump_and_visual_move_flags_are_correct() {
        let reg = super::super::registry::CommandRegistry::with_defaults();

        let must_be_jump = [
            "goto-first-line", "goto-last-line",
            "search-next", "search-prev",
            "page-down", "page-up",
        ];
        for name in must_be_jump {
            assert!(
                reg.get_mappable(name).expect(name).is_jump(),
                "'{name}' should have jump: true"
            );
        }

        let must_be_visual_move = ["move-down", "move-up"];
        for name in must_be_visual_move {
            assert!(
                reg.get_mappable(name).expect(name).is_visual_move(),
                "'{name}' should have visual_move: true"
            );
        }

        // Spot-check non-jump commands.
        for name in ["move-left", "move-right", "delete", "undo", "insert-before"] {
            assert!(
                !reg.get_mappable(name).expect(name).is_jump(),
                "'{name}' should have jump: false"
            );
            assert!(
                !reg.get_mappable(name).expect(name).is_visual_move(),
                "'{name}' should have visual_move: false"
            );
        }
    }
}
