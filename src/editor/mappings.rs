use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use regex_cursor::engines::meta::Regex;

use crate::auto_pairs::{delete_pair, insert_pair_close};
use super::commands::{cmd_clear_search, search_sel};
use super::registry::MappableCommand;
use crate::core::selection::Selection;
use crate::ops::edit::{delete_char_backward, delete_char_forward, insert_char};
use crate::ops::motion::cmd_move_right;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::find_next_match;

use super::keymap::{KeymapCommand, WalkResult};
use super::{Editor, Mode, SearchDirection};

impl Editor {
    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        // Any keypress dismisses the previous transient status message.
        self.status_msg = None;
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Command => self.handle_command(key),
            Mode::Search => self.handle_search(key),
        }
    }

    // ── Normal mode ───────────────────────────────────────────────────────────

    fn handle_normal(&mut self, key: KeyEvent) {
        // ── Consume WaitChar argument ─────────────────────────────────────────
        // If a f/t/F/T/r binding fired on the previous keypress, `wait_char`
        // holds the command name to dispatch. The next character (any key)
        // becomes the argument — stored in `pending_char` for the command to read.
        if let Some(wc) = self.wait_char.take() {
            if let KeyCode::Char(ch) = key.code {
                let count = self.count.take().unwrap_or(1);
                self.pending_char = Some(ch);
                // Resolve extend duality at char-consumption time (not setup time),
                // since extend mode could change between the trigger key and the char.
                let name = if self.extend {
                    wc.extend_name.unwrap_or(wc.cmd_name)
                } else {
                    wc.cmd_name
                };
                let cmd = KeymapCommand { name, extend_name: None };
                self.execute_keymap_command(cmd, count);
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
            self.extend = false;
            cmd_clear_search(self, 0);
            return;
        }

        // ── Count prefix accumulation ─────────────────────────────────────────
        // Only accumulate when we're at the trie root (no pending sequence).
        // `0` without an existing count is the goto-line-start binding, not a digit.
        if self.pending_keys.is_empty() {
            match key.code {
                KeyCode::Char(d @ '1'..='9') => {
                    let n = self.count.unwrap_or(0) * 10 + (d as usize - '0' as usize);
                    self.count = Some(n);
                    return;
                }
                KeyCode::Char('0') if self.count.is_some() => {
                    self.count = Some(self.count.unwrap() * 10);
                    return;
                }
                _ => {}
            }
        }

        // ── Ctrl-key normalisation ────────────────────────────────────────────
        //
        // Three categories of CONTROL keys:
        //
        // 1. Explicit Ctrl bindings (Ctrl+c, Ctrl+r, Ctrl+,, Ctrl+x, Ctrl+X):
        //    Have a dedicated trie entry. Used as-is regardless of kitty mode.
        //
        // 2. Kitty one-shot extend (Ctrl+h/j/k/l/w/b and similar motion keys):
        //    No explicit trie binding. Normalised by stripping CONTROL and
        //    temporarily setting extend=true. Only triggers the one-shot extend
        //    when kitty_enabled is true (otherwise strips CONTROL but leaves
        //    extend unchanged, preserving legacy "Ctrl+motion = bare motion"
        //    behaviour on real terminals).
        //
        // Detection: try the key as-is in the trie first. If NoMatch and the key
        // had CONTROL, strip CONTROL and retry; set one_shot_extend=kitty_enabled.

        let (lookup_key, one_shot_extend) =
            if key.modifiers.contains(KeyModifiers::CONTROL) && self.pending_keys.is_empty() {
                // Fast-check: does an explicit Ctrl binding exist?
                let ctrl_result = self.keymap.normal.walk(&[key]);
                if matches!(ctrl_result, WalkResult::NoMatch) {
                    // No explicit Ctrl binding — strip CONTROL and dispatch as bare key.
                    let bare = KeyEvent::new(key.code, KeyModifiers::NONE);
                    let one_shot = self.kitty_enabled; // only extend in kitty mode
                    (bare, one_shot)
                } else {
                    (key, false)
                }
            } else {
                (key, false)
            };

        // Temporarily activate extend for kitty one-shot.
        let saved_extend = self.extend;
        if one_shot_extend {
            self.extend = true;
        }

        self.pending_keys.push(lookup_key);
        let result = self.keymap.normal.walk(&self.pending_keys);

        match result {
            WalkResult::Leaf(cmd) => {
                self.pending_keys.clear();
                let raw_count = self.count.take();
                self.explicit_count = raw_count.is_some();
                let count = raw_count.unwrap_or(1);
                self.execute_keymap_command(cmd, count);
                self.explicit_count = false;
                if one_shot_extend {
                    self.extend = saved_extend;
                }
            }
            WalkResult::WaitChar(wc) => {
                self.pending_keys.clear();
                self.wait_char = Some(wc);
                if one_shot_extend {
                    self.extend = saved_extend;
                }
            }
            WalkResult::Interior { .. } => {
                // More keys needed. pending_keys stays populated.
                if one_shot_extend {
                    self.extend = saved_extend;
                }
            }
            WalkResult::NoMatch => {
                self.pending_keys.clear();
                self.count = None;
                if one_shot_extend {
                    self.extend = saved_extend;
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
                self.execute_keymap_command(cmd, 1);
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

    /// Execute a keymap command with the given count.
    ///
    /// Resolves extend-mode duality, then dispatches through the
    /// [`CommandRegistry`]. `EditorCmd` variants are dispatched to
    /// [`dispatch_editor_cmd`].
    ///
    /// [`CommandRegistry`]: super::registry::CommandRegistry
    pub(super) fn execute_keymap_command(&mut self, cmd: KeymapCommand, count: usize) {
        // Resolve extend-mode duality: when extend is active, prefer extend_name.
        // (For WaitChar commands this is already resolved before calling here.)
        let resolved = if self.extend {
            cmd.extend_name.unwrap_or(cmd.name)
        } else {
            cmd.name
        };

        if let Some(reg_cmd) = self.registry.get(resolved).cloned() {
            // Snapshot pending_char before dispatch — commands consume it via `.take()`.
            let char_arg = self.pending_char;

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
            sel.is_cursor() && self.doc.buf().char_at(sel.head) == Some(ch)
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
            if !sel.is_cursor() || sel.head == 0 {
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

    /// Replace the primary selection, preserving all other selections.
    pub(super) fn set_primary_selection(&mut self, new_sel: Selection) {
        let idx = self.doc.sels().primary_index();
        let new_sels = self.doc.sels().clone().replace(idx, new_sel);
        self.doc.set_selections(new_sels);
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
                self.registers.write(SEARCH_REGISTER, vec![pattern]);
                // Keep the position that live search already moved us to;
                // discard the pre-search snapshot.
                self.search.pre_search_sels = None;
                // search.regex stays alive for immediate n/N without recompile.
                // set_mode does not touch search state, so it is safe to call here.
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            MiniBufferEvent::EmptiedByBackspace => {
                // Restore position when pattern is fully erased, but stay in Search mode.
                if let Some(sels) = self.search.pre_search_sels.clone() {
                    self.doc.set_selections(sels);
                }
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

        let regex = match Regex::new(&pattern) {
            Ok(r) => r,
            Err(_) => {
                // Invalid regex in progress — don't move; just clear cached regex.
                self.search.set_regex(None);
                return;
            }
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
                let anchor = if self.extend {
                    // Extend from the original anchor.
                    Some(self.search.pre_search_sels.as_ref().map(|s| s.primary().anchor).unwrap_or(start))
                } else {
                    None
                };
                self.set_primary_selection(search_sel(start, end_incl, anchor, direction));
            }
            None => {
                // No match — restore position to pre-search.
                if let Some(sels) = self.search.pre_search_sels.clone() {
                    self.doc.set_selections(sels);
                }
            }
        }

        self.search.set_regex(Some(regex));
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

        // Parse trailing `!` once so all command arms can opt in to force semantics.
        // Commands that don't support `!` explicitly reject it with an error.
        let (cmd, force) = match cmd_raw.strip_suffix('!') {
            Some(base) => (base, true),
            None => (cmd_raw, false),
        };

        match cmd {
            "q" | "quit" => {
                if !force && self.doc.is_dirty() {
                    self.status_msg = Some("Unsaved changes (add ! to override)".into());
                } else {
                    self.should_quit = true;
                }
            }
            "w" | "write" => {
                // No read-only file semantics yet, so :w! has no defined meaning.
                if force {
                    self.status_msg = Some("Error: w! is not supported".into());
                } else {
                    self.write_file_cmd(arg);
                }
            }
            "wq" => {
                // force applies to the quit part: quit even if the write fails.
                if self.write_file_cmd(arg) || force {
                    self.should_quit = true;
                }
            }
            "clearsearch" | "cs" => {
                cmd_clear_search(self, 0);
            }
            other => {
                self.status_msg = Some(format!("Unknown command: {other}"));
            }
        }
    }

    /// Serialize the buffer and write it to disk.
    ///
    /// If `arg` is `Some(path)`, performs a save-as: writes to the specified
    /// path and updates `self.file_path` / `self.file_meta` so that subsequent
    /// `:w` (no argument) targets the same path.
    ///
    /// If `arg` is `None`, writes to the current file. Errors with
    /// "no file name" if the buffer is a scratch buffer with no path.
    ///
    /// On success, calls `self.doc.mark_saved()` and sets a status message.
    /// Returns `true` on success, `false` on any error.
    fn write_file_cmd(&mut self, arg: Option<&str>) -> bool {
        let (content, line_count) = {
            let buf = self.doc.buf();
            // The rope is always stored LF-normalized; restore CRLF for files that
            // originally used it so we don't silently change line endings on save.
            let content = if buf.line_ending() == crate::core::buffer::LineEnding::CrLf {
                buf.to_string().replace('\n', "\r\n")
            } else {
                buf.to_string()
            };
            // The buffer always ends with a structural '\n', so len_lines() returns
            // one more than the number of visible lines (ropey counts the empty
            // string after the final newline as an extra line).
            let line_count = buf.len_lines().saturating_sub(1);
            (content, line_count)
        }; // buf borrow released here

        if let Some(path_str) = arg {
            // Save-as: write to the specified path.
            let path = std::path::Path::new(path_str);
            // Try to preserve existing file's permissions; if the file doesn't
            // exist yet, write_file_new creates it with default permissions.
            let result = match crate::io::read_file_meta(path) {
                Ok(meta) => crate::io::write_file_atomic(&content, &meta).map(|()| meta),
                Err(_)   => crate::io::write_file_new(&content, path),
            };
            match result {
                Ok(meta) => {
                    // Store the canonicalized path so file_path and file_meta.resolved_path
                    // always agree, even when the user supplied a relative or symlink path.
                    self.file_path = Some(meta.resolved_path.clone());
                    self.file_meta = Some(meta);
                    self.doc.mark_saved();
                    self.status_msg = Some(format!("Written {line_count} lines"));
                    true
                }
                Err(e) => {
                    self.status_msg = Some(format!("Error: {e}"));
                    false
                }
            }
        } else {
            // Write to the current file.
            let Some(meta) = self.file_meta.as_ref() else {
                self.status_msg = Some("Error: no file name".into());
                return false;
            };
            match crate::io::write_file_atomic(&content, meta) {
                Ok(()) => {
                    self.doc.mark_saved();
                    self.status_msg = Some(format!("Written {line_count} lines"));
                    true
                }
                Err(e) => {
                    self.status_msg = Some(format!("Error: {e}"));
                    false
                }
            }
        }
    }
}

