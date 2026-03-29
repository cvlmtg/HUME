use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::auto_pairs::{delete_pair, insert_pair_close};
use crate::command::MappableCommand;
use crate::ops::edit::{
    delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
    paste_before, replace_selections,
};
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start, cmd_move_left, cmd_move_right,
    find_char_backward, find_char_forward, MotionMode,
};
use crate::ops::register::{yank_selections, DEFAULT_REGISTER};
use crate::ops::selection_cmd::{cmd_collapse_selection, cmd_flip_selections};

use super::keymap::{EditorAction, KeymapCommand, WalkResult};
use super::{Editor, FindChar, MiniBuffer, Mode};

impl Editor {
    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        // Any keypress dismisses the previous transient status message.
        self.status_msg = None;
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Command => self.handle_command(key),
        }
    }

    // ── Normal mode ───────────────────────────────────────────────────────────

    fn handle_normal(&mut self, key: KeyEvent) {
        // ── Consume WaitChar argument ─────────────────────────────────────────
        // If a f/t/F/T/r binding fired on the previous keypress, the trie stored
        // its constructor here. The next character (any key) becomes the argument.
        if let Some(constructor) = self.wait_char.take() {
            if let KeyCode::Char(ch) = key.code {
                let count = self.count.take().unwrap_or(1);
                let cmd = constructor(ch);
                self.execute_keymap_command(cmd, count);
            }
            // Non-char key (e.g. Esc after pressing `f`) just resets — nothing to do,
            // wait_char was already taken above.
            return;
        }

        // ── Hard-reset on Esc ─────────────────────────────────────────────────
        if key.code == KeyCode::Esc {
            self.pending_keys.clear();
            self.count = None;
            self.extend = false;
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
                let count = self.count.take().unwrap_or(1);
                self.execute_keymap_command(cmd, count);
                if one_shot_extend {
                    self.extend = saved_extend;
                }
            }
            WalkResult::WaitChar(constructor) => {
                self.pending_keys.clear();
                self.wait_char = Some(constructor);
                if one_shot_extend {
                    self.extend = saved_extend;
                }
            }
            WalkResult::Interior { .. } => {
                // More keys needed. pending_keys stays populated.
                // (Status bar could show the node name — future work.)
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

    fn handle_insert(&mut self, key: KeyEvent) {
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
    /// Resolves extend-mode duality for `Cmd` variants, then dispatches either
    /// through the [`CommandRegistry`] (for pure `cmd_*` functions) or directly
    /// to [`execute_editor_action`] (for composite/side-effectful actions).
    ///
    /// [`CommandRegistry`]: crate::command::CommandRegistry
    fn execute_keymap_command(&mut self, cmd: KeymapCommand, count: usize) {
        match cmd {
            KeymapCommand::Cmd { name, extend_name } => {
                // Resolve the extend variant if extend mode is active.
                let resolved = if self.extend {
                    extend_name.unwrap_or(name)
                } else {
                    name
                };
                if let Some(reg_cmd) = self.registry.get(resolved).cloned() {
                    match reg_cmd {
                        MappableCommand::Motion { fun, .. } => {
                            // Motion functions take (buf, sels, count). count defaults to 1
                            // if the user typed no prefix.
                            self.apply_motion(|b, s| fun(b, s, count));
                        }
                        MappableCommand::Selection { fun, .. } => {
                            // Selection / text-object functions don't take count.
                            self.apply_motion(|b, s| fun(b, s));
                        }
                        MappableCommand::Edit { fun, .. } => {
                            self.doc.apply_edit(fun);
                        }
                    }
                }
            }
            KeymapCommand::Action(action) => {
                self.execute_editor_action(action, count);
            }
        }
    }

    /// Execute an [`EditorAction`] — composite or side-effectful operations that
    /// cannot be expressed as pure `cmd_*` function pointers.
    ///
    /// This is a 1:1 migration of the `match` arms that previously lived inline
    /// in `handle_normal`. Logic is unchanged; the structure is now centralised.
    fn execute_editor_action(&mut self, action: EditorAction, count: usize) {
        match action {
            // ── Mode transitions ──────────────────────────────────────────────
            EditorAction::EnterCommandMode => {
                self.set_mode(Mode::Command);
                self.minibuf = Some(MiniBuffer { prompt: ':', input: String::new() });
            }

            EditorAction::ExitInsert => {
                self.set_mode(Mode::Normal);
            }

            // `i` — collapse each selection to its start, enter Insert.
            EditorAction::EnterInsertBefore => {
                self.apply_motion(|_b, sels| {
                    use crate::core::selection::Selection;
                    sels.map(|s| Selection::cursor(s.start()))
                });
                self.set_mode(Mode::Insert);
            }

            // `a` — move one grapheme right, enter Insert.
            EditorAction::EnterInsertAfter => {
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `I` — move to first non-blank on the line, enter Insert.
            EditorAction::EnterInsertLineStart => {
                self.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `A` — move to line end, then one more right, enter Insert.
            EditorAction::EnterInsertLineEnd => {
                self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `o` — dual purpose:
            //   extend mode: flip anchor/head of every selection.
            //   normal mode: open a new line below, enter Insert.
            //
            // The edit group opened here ensures the structural '\n' and
            // everything typed before Esc form one undo step (same pattern as `c`).
            EditorAction::OpenLineBelowOrFlip => {
                if self.extend {
                    self.apply_motion(cmd_flip_selections);
                } else {
                    self.doc.begin_edit_group();
                    self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
                    self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                    self.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
                    self.set_mode(Mode::Insert);
                }
            }

            // `O` — open a new line above, enter Insert.
            EditorAction::OpenLineAbove => {
                self.doc.begin_edit_group();
                self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1));
                self.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
                self.apply_motion(|b, s| cmd_move_left(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // ── Edit composites ───────────────────────────────────────────────

            // `d` — yank selections into the default register, then delete them.
            EditorAction::Delete => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.apply_edit(delete_selection);
                self.registers.write(DEFAULT_REGISTER, yanked);
            }

            // `c` — yank, delete, enter Insert (all in one undo group).
            // The group is opened here so the delete is folded in; set_mode(Insert)
            // sees the group is already open and skips begin_edit_group.
            EditorAction::Change => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.begin_edit_group();
                self.doc.apply_edit_grouped(delete_selection);
                self.registers.write(DEFAULT_REGISTER, yanked);
                self.set_mode(Mode::Insert);
            }

            // `y` — yank selections into the default register (no buffer change).
            EditorAction::Yank => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.registers.write(DEFAULT_REGISTER, yanked);
            }

            // `p` — paste after; swap displaced text back into the register when
            // the selection was non-cursor (replace-and-swap semantics).
            EditorAction::PasteAfter => {
                if let Some(reg) = self.registers.read(DEFAULT_REGISTER) {
                    let values = reg.values().to_vec();
                    let displaced = self.doc.apply_edit(|b, s| paste_after(b, s, &values));
                    if displaced.iter().any(|s| !s.is_empty()) {
                        self.registers.write(DEFAULT_REGISTER, displaced);
                    }
                }
            }

            // `P` — paste before; same swap semantics.
            EditorAction::PasteBefore => {
                if let Some(reg) = self.registers.read(DEFAULT_REGISTER) {
                    let values = reg.values().to_vec();
                    let displaced = self.doc.apply_edit(|b, s| paste_before(b, s, &values));
                    if displaced.iter().any(|s| !s.is_empty()) {
                        self.registers.write(DEFAULT_REGISTER, displaced);
                    }
                }
            }

            EditorAction::Undo => self.doc.undo(),
            EditorAction::Redo => self.doc.redo(),

            // ── Selection state ───────────────────────────────────────────────

            // `;` — collapse AND exit extend mode (collapsing is a "done" signal).
            EditorAction::CollapseAndExitExtend => {
                self.extend = false;
                self.apply_motion(cmd_collapse_selection);
            }

            EditorAction::ToggleExtend => {
                self.extend = !self.extend;
            }

            // ── Find / till character ─────────────────────────────────────────

            EditorAction::FindForward { ch, kind } => {
                let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                self.apply_motion(|b, s| find_char_forward(b, s, mode, count, ch, kind));
                self.last_find = Some(FindChar { ch, kind });
            }

            EditorAction::FindBackward { ch, kind } => {
                let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                self.apply_motion(|b, s| find_char_backward(b, s, mode, count, ch, kind));
                self.last_find = Some(FindChar { ch, kind });
            }

            // `=` / `-` — repeat last find in absolute direction.
            EditorAction::RepeatFindForward => {
                if let Some(FindChar { ch, kind }) = self.last_find {
                    let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                    self.apply_motion(|b, s| find_char_forward(b, s, mode, count, ch, kind));
                }
            }
            EditorAction::RepeatFindBackward => {
                if let Some(FindChar { ch, kind }) = self.last_find {
                    let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                    self.apply_motion(|b, s| find_char_backward(b, s, mode, count, ch, kind));
                }
            }

            // `r` + char — replace every character in every selection with `ch`.
            EditorAction::Replace(ch) => {
                self.doc.apply_edit(|b, s| replace_selections(b, s, ch));
            }

            // ── Page scroll ───────────────────────────────────────────────────
            // Uses view.height as count (not the user's count prefix).

            EditorAction::PageDown => {
                let page = self.view.height.max(1);
                if self.extend {
                    // "extend-down" command, look it up in registry.
                    if let Some(MappableCommand::Motion { fun, .. }) =
                        self.registry.get("extend-down").cloned()
                    {
                        self.apply_motion(|b, s| fun(b, s, page));
                    }
                } else if let Some(MappableCommand::Motion { fun, .. }) =
                    self.registry.get("move-down").cloned()
                {
                    self.apply_motion(|b, s| fun(b, s, page));
                }
            }

            EditorAction::PageUp => {
                let page = self.view.height.max(1);
                if self.extend {
                    if let Some(MappableCommand::Motion { fun, .. }) =
                        self.registry.get("extend-up").cloned()
                    {
                        self.apply_motion(|b, s| fun(b, s, page));
                    }
                } else if let Some(MappableCommand::Motion { fun, .. }) =
                    self.registry.get("move-up").cloned()
                {
                    self.apply_motion(|b, s| fun(b, s, page));
                }
            }

            // ── Misc ──────────────────────────────────────────────────────────

            EditorAction::Quit => {
                self.should_quit = true;
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

    // ── Command mode ──────────────────────────────────────────────────────────

    fn handle_command(&mut self, key: KeyEvent) {
        match key.code {
            // ── Cancel ────────────────────────────────────────────────────────
            KeyCode::Esc => {
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }
            // Ctrl+C acts as Esc in all modes (Vim convention).
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }

            // ── Execute ───────────────────────────────────────────────────────
            KeyCode::Enter => {
                self.execute_command();
                self.set_mode(Mode::Normal);
                self.minibuf = None;
            }

            // ── Edit input ────────────────────────────────────────────────────
            KeyCode::Backspace => {
                if let Some(mb) = &mut self.minibuf {
                    if mb.input.is_empty() {
                        // Backspace on empty input cancels (Kakoune behaviour).
                        self.set_mode(Mode::Normal);
                        self.minibuf = None;
                    } else {
                        mb.input.pop();
                    }
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(mb) = &mut self.minibuf {
                    mb.input.push(ch);
                }
            }

            _ => {}
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

        match input.as_str() {
            "q" | "quit" => self.should_quit = true,
            "w" | "write" => { self.write_file(); }
            "wq" => {
                if self.write_file() {
                    self.should_quit = true;
                }
            }
            other => {
                self.status_msg = Some(format!("Unknown command: {other}"));
            }
        }
    }

    /// Serialize the buffer and write it to disk atomically, preserving the
    /// original file's permissions, ownership, and symlink structure.
    ///
    /// Delegates the I/O to `crate::io::write_file_atomic`. Sets
    /// `self.status_msg` on both success and failure.
    /// Returns `true` on success, `false` on any error.
    fn write_file(&mut self) -> bool {
        let Some(meta) = self.file_meta.as_ref() else {
            self.status_msg = Some("Error: no file name".into());
            return false;
        };

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

        match crate::io::write_file_atomic(&content, meta) {
            Ok(()) => {
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
