use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::commands::cmd_clear_search;
use super::super::dispatch::ArgSource;
use super::super::keymap::WalkResult;
use crate::ops::MotionMode;
use crate::ops::register::{MACRO_REGISTER, is_valid_macro_register, is_valid_register_name};

use super::super::Editor;
use super::super::register_ops::RegisterPrefix;
use super::super::replay::MacroPending;
use hume_engine::types::EditorMode;

/// Enqueue the keys stored in `reg` into the editor's replay queue.
///
/// The accumulated count (defaulting to 1) determines how many times the macro
/// is enqueued. Count is consumed and cleared. No-op when the register is empty,
/// unset, or holds text rather than a macro.
fn enqueue_macro_replay(ed: &mut Editor, reg: char) {
    let count = ed.state.count.take().unwrap_or(1);
    if let Some(keys) = ed
        .state
        .registers
        .read(reg)
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
    {
        for _ in 0..count {
            ed.state.replay_queue.extend(keys.iter().copied());
        }
    }
}

impl Editor {
    // ── Normal mode ───────────────────────────────────────────────────────────

    pub(super) fn handle_normal(&mut self, key: KeyEvent) {
        // ── Kitty SHIFT normalization ─────────────────────────────────────────
        // The kitty keyboard protocol reports shifted keys as Char('Q') / Char(':')
        // / Char('$') with KeyModifiers::SHIFT. DISAMBIGUATE_ESCAPE_CODES keeps the
        // physically-held SHIFT in the modifier field; REPORT_ALTERNATE_KEYS is what
        // makes compliant terminals strip it again. WezTerm and other terminals
        // enable DISAMBIGUATE but do not fully honor REPORT_ALTERNATE_KEYS, so
        // shifted punctuation (`:`, `$`, `?`, `{`, …) arrives as Char(x) + SHIFT and
        // misses its Char(x) + NONE trie binding — silently swallowing `:`, `$`,
        // `?` etc. in Normal/Extend mode.
        //
        // For HUME's purposes the shifted-ness is already encoded in the char
        // itself (every printable binding in the keymap is stored as Char(x) +
        // NONE; no binding distinguishes via SHIFT), so the SHIFT bit is redundant
        // for any Char and is stripped here. This covers both kitty's letter
        // reporting and the shifted-punctuation gap on partially-compliant terminals.
        //
        // Only strip SHIFT when it is the *only* modifier. Ctrl+Shift combinations
        // (e.g. Ctrl+X, Ctrl+}) keep CONTROL so they match their explicit Ctrl
        // bindings; Shift+Tab arrives as KeyCode::BackTab (not Char), so it is
        // untouched and keeps its SHIFT bit for the completion back-cycle.
        let key = if key.modifiers == KeyModifiers::SHIFT {
            if let KeyCode::Char(_) = key.code {
                KeyEvent::new(key.code, KeyModifiers::NONE)
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
        if let Some(wc) = self.state.wait_char.take() {
            if let KeyCode::Char(ch) = key.code {
                let count = self.state.count.take();
                self.state.pending_char = Some(ch);
                // Extend resolution: sticky extend (mode == Extend) OR one-shot
                // ctrl_extend carried into WaitCharPending from the original keypress.
                let extend = (self.state.mode() == EditorMode::Extend) || wc.ctrl_extend;
                self.execute_keymap_command(wc.cmd_name.clone(), count, extend, ArgSource::Keymap);
            }
            // Non-char key (e.g. Esc after pressing `f`): cancel the wait.
            // Clear count so a prefix like `3f<Esc>` doesn't leak into the next command.
            self.state.count = None;
            return;
        }

        // ── Hard-reset on Esc ─────────────────────────────────────────────────
        if key.code == KeyCode::Esc {
            self.state.pending_keys.clear();
            self.state.count = None;
            self.state.pending_ctrl_extend = false; // cancel any pending extend mode
            self.state.macro_pending = None; // cancel any pending q/Q register-name prompt
            self.state.register_prefix = None; // cancel any pending "<reg> state
            // Esc exits Extend mode; Normal is the reset state.
            if self.state.mode() == EditorMode::Extend {
                self.set_mode(EditorMode::Normal);
            }
            let _ = cmd_clear_search(&mut self.state, &mut self.view, 0, MotionMode::Move);
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
        if let Some(pending) = self.state.macro_pending.take() {
            match pending {
                MacroPending::Record => {
                    // A count prefix before `Q<reg>` has no meaning for recording.
                    // Clear it so it doesn't leak into the first key typed during
                    // the session (which would fire with count N instead of 1).
                    self.state.count = None;
                    match key.code {
                        // `QQ` — record into the default register. `Q` is uppercase
                        // so is_valid_macro_register won't catch it; handle explicitly.
                        KeyCode::Char('Q') => {
                            self.state.macro_recording = Some((MACRO_REGISTER, Vec::new()));
                            self.state.skip_macro_record = true;
                        }
                        KeyCode::Char(reg) if is_valid_macro_register(reg) => {
                            self.state.macro_recording = Some((reg, Vec::new()));
                            self.state.skip_macro_record = true;
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

        // ── Register prefix: consume register-name key ────────────────────────
        // After `"`, the next keypress names the register for the upcoming
        // yank/delete/change/paste. Valid names: '0'–'9', 'b' (black hole), 'c'
        // (clipboard). Invalid chars or Esc cancel silently.
        if let Some(RegisterPrefix::Awaiting) = self.state.register_prefix {
            self.state.register_prefix = None;
            if let KeyCode::Char(ch) = key.code
                && key.modifiers == KeyModifiers::NONE
                && is_valid_register_name(ch)
            {
                self.state.register_prefix = Some(RegisterPrefix::Selected(ch));
            }
            // Invalid char, modified key, or non-Char key: cancel silently (key is swallowed).
            // Count accumulated before `"` is preserved for the next command.
            return;
        }

        // ── Count prefix accumulation ─────────────────────────────────────────
        // Only accumulate when we're at the trie root (no pending sequence)
        // and no modifiers are held (Ctrl+4 is not a count digit).
        // `0` without an existing count is not a digit — it falls through to the
        // trie (unbound by default; core:vim-keybind binds it to goto-line-start).
        // NOTE: this runs AFTER macro_pending so that `Q1`/`q1` treat `1` as a
        // register name, not as a count digit.
        if self.state.pending_keys.is_empty() && key.modifiers == KeyModifiers::NONE {
            match key.code {
                KeyCode::Char(d @ '1'..='9') => {
                    let n = self.state.count.unwrap_or(0) * 10 + (d as usize - '0' as usize);
                    self.state.count = Some(n);
                    return;
                }
                KeyCode::Char('0') if self.state.count.is_some() => {
                    self.state.count = self.state.count.map(|c| c * 10);
                    return;
                }
                _ => {}
            }
        }

        // ── `Q` / `q` / `"` intercepts (bare key, at trie root, no modifiers) ──
        // `Q` toggles recording; `q` triggers replay. Recording uses uppercase
        // because you do it once; replay uses lowercase because you do it often.
        // Both are suppressed while a replay is in progress to prevent nesting.
        // `"` triggers the register-prefix — the next char names the target register.
        if self.state.pending_keys.is_empty() && key.modifiers == KeyModifiers::NONE {
            match key.code {
                KeyCode::Char('Q') => {
                    if let Some((reg, keys)) = self.state.macro_recording.take() {
                        // Always allow stopping an in-progress recording, even if
                        // the user has navigated to a read-only buffer since starting.
                        self.state.registers.write_macro(reg, keys);
                    } else if !self.state.is_replaying && !self.focused_buffer_read_only() {
                        self.state.macro_pending = Some(MacroPending::Record);
                    }
                    // During replay, or on a read-only buffer: silently ignore.
                    return;
                }
                KeyCode::Char('q') => {
                    if !self.state.is_replaying
                        && self.state.macro_recording.is_none()
                        && !self.focused_buffer_read_only()
                    {
                        // Replay: wait for the register-name key.
                        self.state.macro_pending = Some(MacroPending::Replay);
                    }
                    // During recording, replay, or on a read-only buffer: silently ignore.
                    return;
                }
                KeyCode::Char('"') => {
                    // Register prefix: `"<reg>` selects the named register for the
                    // upcoming yank/delete/change/paste. Set pending state; the next
                    // keypress will be consumed as the register name.
                    self.state.register_prefix = Some(RegisterPrefix::Awaiting);
                    return;
                }
                _ => {}
            }
        }

        // ── Extend resolution overview ────────────────────────────────────────
        //
        // "Should this command extend?" is resolved in three stages, since it
        // depends on which command a key resolves to, and the Ctrl path
        // changes which key gets looked up — extend resolution and trie
        // walking can't be separated.
        //
        // Stage 1 (below): in sticky extend mode, try the extend trie first —
        // it maps keys to *replacement* commands (e.g. `o → flip-selections`),
        // dispatched with extend = false. A miss falls through to the normal
        // trie.
        // Stage 2 (further below): Ctrl+key may strip CONTROL and re-walk with
        // the bare key (kitty one-shot extend); whether to extend depends on
        // the resolved bare-key command's extendability, computed here as
        // `ctrl_extend`.
        // Stage 3 (after the trie walk): merges sticky mode
        // (EditorMode::Extend) and one-shot Ctrl (ctrl_extend) — the earliest
        // point both are known.

        // ── Stage 1: Extend-trie override ────────────────────────────────────
        //
        // We walk with [pending_keys..., key] without committing the push yet —
        // only `Interior` commits the key (so the sequence accumulates correctly
        // across keypresses). On `NoMatch` the key is not yet in `pending_keys`,
        // so the normal-trie path below can push it as usual.
        if self.state.mode() == EditorMode::Extend && !key.modifiers.contains(KeyModifiers::CONTROL)
        {
            let mut seq = self.state.pending_keys.clone();
            seq.push(key);
            match self.state.keymap.extend.walk(&seq) {
                WalkResult::Leaf(cmd) => {
                    self.state.pending_keys.clear();
                    self.state.pending_ctrl_extend = false;
                    let count = self.state.count.take();
                    self.execute_keymap_command(cmd.name.clone(), count, false, ArgSource::Keymap);
                    return;
                }
                WalkResult::Interior { .. } => {
                    // Mid-sequence — commit the key and wait for more.
                    self.state.pending_keys.push(key);
                    return;
                }
                WalkResult::WaitChar(wc) => {
                    self.state.pending_keys.clear();
                    self.state.pending_ctrl_extend = false;
                    self.state.wait_char = Some(wc);
                    return;
                }
                WalkResult::NoMatch => {
                    // No extend-trie match — fall through to normal trie.
                }
            }
        }

        // ── Stage 2: Ctrl-key normalisation + one-shot extend ────────────────
        //
        // Two categories of CONTROL keys:
        // 1. Explicit Ctrl bindings (Ctrl+c, Ctrl+r, Ctrl+,, Ctrl+x, Ctrl+X):
        //    have a dedicated trie entry, used as-is regardless of kitty mode.
        // 2. Implicit Ctrl+motion (Ctrl+h/j/k/l/w/b, …): no explicit binding.
        //    With kitty enabled, one-shot extend: strip CONTROL, look up the
        //    bare key, dispatch extend=true. Without kitty, no-op — legacy
        //    terminals can't reliably distinguish Ctrl+letter from control
        //    codes.
        //
        // Detection: try the key as-is first; on NoMatch with CONTROL set,
        // strip it and retry only if kitty is enabled.
        //
        // REPORT_ALTERNATE_KEYS (enabled at init) makes the terminal send the
        // shifted character directly — crossterm swaps in the alternate
        // keycode and strips SHIFT, so Ctrl+} arrives as Char('}') with just
        // CONTROL, and stripping CONTROL gives the correct bare key,
        // independent of layout.

        // Trie walk + Ctrl normalisation in one pass.
        //
        // For Ctrl keys at the trie root, walk once to check for an explicit
        // binding. If found, reuse that result directly (no second walk).
        // If NoMatch, strip CONTROL and re-walk only on kitty terminals.
        let (result, ctrl_extend) = if key.modifiers.contains(KeyModifiers::CONTROL)
            && self.state.pending_keys.is_empty()
        {
            match self.state.keymap.normal.walk(&[key]) {
                WalkResult::NoMatch if self.kitty_enabled => {
                    // Kitty mode: strip CONTROL, re-walk as extend. Only proceed if the
                    // resolved command is extendable — prevents e.g. Ctrl+u running
                    // "undo" (not a motion) as a one-shot extend.
                    let bare = KeyEvent::new(key.code, KeyModifiers::NONE);
                    self.state.pending_keys.push(bare);
                    let result = self.state.keymap.normal.walk(&self.state.pending_keys);
                    let ctrl_extend = match &result {
                        // Prefix key (g, m, z…): persist extend intent for the
                        // remaining keys in the sequence. Extendability is
                        // checked at Leaf resolution, not here.
                        WalkResult::Interior { .. } => {
                            self.state.pending_ctrl_extend = true;
                            true
                        }
                        WalkResult::Leaf(c) => self
                            .state
                            .registry
                            .get_mappable(c.name.as_ref())
                            .is_some_and(|r| r.is_extendable()),
                        WalkResult::WaitChar(wc) => self
                            .state
                            .registry
                            .get_mappable(wc.cmd_name.as_ref())
                            .is_some_and(|r| r.is_extendable()),
                        WalkResult::NoMatch => false,
                    };
                    if !ctrl_extend {
                        self.state.pending_keys.clear();
                        self.state.count = None;
                        return;
                    }
                    (result, true)
                }
                WalkResult::NoMatch => return, // Legacy: no-op.
                // Explicit Ctrl+letter binding. Extend only if the binding
                // itself declares force_extend (e.g. Ctrl+x → select-line).
                // Registry's is_extendable() is not consulted here — that
                // flag means "compatible with sticky Extend mode", not
                // "pressing Ctrl means the user asked to extend".
                // Interior: the Ctrl+key starts a multi-key sequence (e.g.
                // Ctrl+p → pane prefix); save it in pending_keys so the
                // follow-up keypress can complete the trie walk.
                matched => {
                    let ctrl_extend = match &matched {
                        WalkResult::Leaf(c) => c.force_extend,
                        _ => false,
                    };
                    if matches!(matched, WalkResult::Interior { .. }) {
                        self.state.pending_keys.push(key);
                    }
                    (matched, ctrl_extend)
                }
            }
        } else {
            self.state.pending_keys.push(key);
            (
                self.state.keymap.normal.walk(&self.state.pending_keys),
                self.state.pending_ctrl_extend,
            )
        };

        // ── Stage 3: Final extend merge ───────────────────────────────────────
        //
        // Both inputs are now available: sticky extend from editor mode, and
        // one-shot extend from the Ctrl path (ctrl_extend). Merge them here.
        // `extend` is passed as a parameter — no mode transition occurs.
        let extend = (self.state.mode() == EditorMode::Extend) || ctrl_extend;

        match result {
            WalkResult::Leaf(cmd) => {
                self.state.pending_keys.clear();
                self.state.pending_ctrl_extend = false;
                // Only apply one-shot extend if the command supports it.
                let extend = extend
                    && self
                        .state
                        .registry
                        .get_mappable(cmd.name.as_ref())
                        .is_some_and(|r| r.is_extendable());
                let count = self.state.count.take();
                self.execute_keymap_command(cmd.name.clone(), count, extend, ArgSource::Keymap);
            }
            WalkResult::WaitChar(mut wc) => {
                self.state.pending_keys.clear();
                self.state.pending_ctrl_extend = false;
                // Carry ctrl_extend into WaitCharPending so extend resolution
                // happens at char-consumption time.
                wc.ctrl_extend = ctrl_extend;
                self.state.wait_char = Some(wc);
            }
            WalkResult::Interior { .. } => {
                // More keys needed. pending_keys stays populated.
            }
            WalkResult::NoMatch => {
                self.state.pending_keys.clear();
                self.state.pending_ctrl_extend = false;
                self.state.count = None;
            }
        }
    }
}
