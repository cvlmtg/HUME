use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::auto_pairs::{delete_pair, insert_pair_close};
use crate::edit::{
    delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
    paste_before, replace_selections,
};
use crate::motion::{
    cmd_extend_first_nonblank, cmd_extend_line_end, cmd_extend_line_start,
    cmd_extend_down, cmd_extend_left, cmd_extend_next_paragraph, cmd_extend_prev_paragraph,
    cmd_extend_right, cmd_extend_select_line, cmd_extend_select_line_backward,
    cmd_extend_select_next_WORD, cmd_extend_select_next_word, cmd_extend_select_prev_WORD,
    cmd_extend_select_prev_word, cmd_extend_up, cmd_goto_first_nonblank, cmd_goto_line_end,
    cmd_goto_line_start, cmd_move_down, cmd_move_left, cmd_move_right, cmd_move_up,
    cmd_next_paragraph, cmd_prev_paragraph, cmd_select_line, cmd_select_line_backward,
    cmd_select_next_WORD, cmd_select_next_word, cmd_select_prev_WORD, cmd_select_prev_word,
    find_char_backward, find_char_forward,
};
use crate::register::{yank_selections, DEFAULT_REGISTER};
use crate::selection::Selection;
use crate::selection_cmd::{
    cmd_collapse_selection, cmd_copy_selection_on_next_line, cmd_cycle_primary_backward,
    cmd_cycle_primary_forward, cmd_flip_selections, cmd_keep_primary_selection,
    cmd_remove_primary_selection, cmd_split_selection_on_newlines, cmd_trim_selection_whitespace,
};
use crate::text_object::{
    cmd_around_WORD, cmd_around_angle, cmd_around_argument, cmd_around_backtick, cmd_around_brace,
    cmd_around_bracket, cmd_around_double_quote, cmd_around_line, cmd_around_paren,
    cmd_around_single_quote, cmd_around_word, cmd_extend_around_WORD, cmd_extend_around_angle,
    cmd_extend_around_argument, cmd_extend_around_backtick, cmd_extend_around_brace,
    cmd_extend_around_bracket, cmd_extend_around_double_quote, cmd_extend_around_line,
    cmd_extend_around_paren, cmd_extend_around_single_quote, cmd_extend_around_word,
    cmd_extend_inner_WORD, cmd_extend_inner_angle, cmd_extend_inner_argument,
    cmd_extend_inner_backtick, cmd_extend_inner_brace, cmd_extend_inner_bracket,
    cmd_extend_inner_double_quote, cmd_extend_inner_line, cmd_extend_inner_paren,
    cmd_extend_inner_single_quote, cmd_extend_inner_word, cmd_inner_WORD, cmd_inner_angle,
    cmd_inner_argument, cmd_inner_backtick, cmd_inner_brace, cmd_inner_bracket,
    cmd_inner_double_quote, cmd_inner_line, cmd_inner_paren, cmd_inner_single_quote,
    cmd_inner_word,
};

use super::{Editor, FindChar, FindKind, MiniBuffer, Mode, PendingKey};
use crate::motion::MotionMode;

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
        // ── Pending key sequences ──────────────────────────────────────────────
        //
        // Text objects are entered as `m` → `i`/`a` → object char.
        // Each stage either advances the sequence or resets and re-dispatches.
        if self.pending != PendingKey::None {
            if let KeyCode::Char(ch) = key.code {
                match self.pending {
                    PendingKey::Match => {
                        match ch {
                            'i' => { self.pending = PendingKey::MatchInner; return; }
                            'a' => { self.pending = PendingKey::MatchAround; return; }
                            _ => {} // fall through to normal dispatch below
                        }
                    }
                    PendingKey::MatchInner => {
                        self.pending = PendingKey::None;
                        if self.dispatch_text_object(ch, true) {
                            return;
                        }
                        // Unrecognized object char — fall through.
                    }
                    PendingKey::MatchAround => {
                        self.pending = PendingKey::None;
                        if self.dispatch_text_object(ch, false) {
                            return;
                        }
                        // Unrecognized object char — fall through.
                    }
                    PendingKey::Replace => {
                        self.pending = PendingKey::None;
                        self.doc.apply_edit(|b, s| replace_selections(b, s, ch));
                        return;
                    }
                    PendingKey::FindForward => {
                        self.pending = PendingKey::None;
                        let kind = FindKind::Inclusive;
                        let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                        self.apply_motion(|b, s| find_char_forward(b, s, mode, 1, ch, kind));
                        self.last_find = Some(FindChar { ch, kind });
                        return;
                    }
                    PendingKey::FindBackward => {
                        self.pending = PendingKey::None;
                        let kind = FindKind::Inclusive;
                        let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                        self.apply_motion(|b, s| find_char_backward(b, s, mode, 1, ch, kind));
                        self.last_find = Some(FindChar { ch, kind });
                        return;
                    }
                    PendingKey::TillForward => {
                        self.pending = PendingKey::None;
                        let kind = FindKind::Exclusive;
                        let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                        self.apply_motion(|b, s| find_char_forward(b, s, mode, 1, ch, kind));
                        self.last_find = Some(FindChar { ch, kind });
                        return;
                    }
                    PendingKey::TillBackward => {
                        self.pending = PendingKey::None;
                        let kind = FindKind::Exclusive;
                        let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                        self.apply_motion(|b, s| find_char_backward(b, s, mode, 1, ch, kind));
                        self.last_find = Some(FindChar { ch, kind });
                        return;
                    }
                    PendingKey::None => unreachable!(),
                }
            }
            // Non-char key (e.g. Esc) or unrecognized char: reset and fall through.
            self.pending = PendingKey::None;
        }

        match key.code {
            // ── Command mode ──────────────────────────────────────────────────
            KeyCode::Char(':') => {
                self.set_mode(Mode::Command);
                self.minibuf = Some(MiniBuffer { prompt: ':', input: String::new() });
            }

            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }

            // ── Basic motion ──────────────────────────────────────────────────
            // In extend mode (self.extend), motions grow the selection instead of moving it.
            KeyCode::Char('h') | KeyCode::Left  => if self.extend {
                self.apply_motion(|b, s| cmd_extend_left(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_move_left(b, s, 1))
            },
            KeyCode::Char('l') | KeyCode::Right => if self.extend {
                self.apply_motion(|b, s| cmd_extend_right(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_move_right(b, s, 1))
            },
            KeyCode::Char('j') | KeyCode::Down  => if self.extend {
                self.apply_motion(|b, s| cmd_extend_down(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_move_down(b, s, 1))
            },
            KeyCode::Char('k') | KeyCode::Up    => if self.extend {
                self.apply_motion(|b, s| cmd_extend_up(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_move_up(b, s, 1))
            },

            // ── Word motion ───────────────────────────────────────────────────
            // In extend mode: union the current selection with the next/prev word range.
            KeyCode::Char('w') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_select_next_word(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_select_next_word(b, s, 1))
            },
            KeyCode::Char('W') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_select_next_WORD(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_select_next_WORD(b, s, 1))
            },
            KeyCode::Char('b') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_select_prev_word(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_select_prev_word(b, s, 1))
            },
            KeyCode::Char('B') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_select_prev_WORD(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_select_prev_WORD(b, s, 1))
            },

            // ── Line start / end ──────────────────────────────────────────────
            KeyCode::Char('0') | KeyCode::Home => if self.extend {
                self.apply_motion(|b, s| cmd_extend_line_start(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1))
            },
            KeyCode::Char('$') | KeyCode::End => if self.extend {
                self.apply_motion(|b, s| cmd_extend_line_end(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1))
            },
            KeyCode::Char('^') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_first_nonblank(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1))
            },

            // ── Paragraph motion ──────────────────────────────────────────────
            KeyCode::Char('{') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_prev_paragraph(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_prev_paragraph(b, s, 1))
            },
            KeyCode::Char('}') => if self.extend {
                self.apply_motion(|b, s| cmd_extend_next_paragraph(b, s, 1))
            } else {
                self.apply_motion(|b, s| cmd_next_paragraph(b, s, 1))
            },

            // ── Page scroll ───────────────────────────────────────────────────
            KeyCode::PageDown => {
                let count = self.view.height.max(1);
                if self.extend {
                    self.apply_motion(|b, s| cmd_extend_down(b, s, count));
                } else {
                    self.apply_motion(|b, s| cmd_move_down(b, s, count));
                }
            }
            KeyCode::PageUp => {
                let count = self.view.height.max(1);
                if self.extend {
                    self.apply_motion(|b, s| cmd_extend_up(b, s, count));
                } else {
                    self.apply_motion(|b, s| cmd_move_up(b, s, count));
                }
            }

            // ── Selection ─────────────────────────────────────────────────────
            // `;` collapses and also exits extend mode — collapsing is a natural "done" signal.
            KeyCode::Char(';') => {
                self.extend = false;
                self.apply_motion(|b, s| cmd_collapse_selection(b, s));
            }
            // `,` — keep primary selection; `ctrl+,` — remove it (keep secondaries).
            // Note: ctrl+, is only transmitted by kitty keyboard protocol; silently ignored in legacy mode.
            KeyCode::Char(',') => if key.modifiers.contains(KeyModifiers::CONTROL) {
                self.apply_motion(|b, s| cmd_remove_primary_selection(b, s));
            } else {
                self.apply_motion(|b, s| cmd_keep_primary_selection(b, s));
            },
            // `S` — split each selection on newlines, producing one cursor per line.
            // `R` — reserved for split-on-regex (needs minibuffer input, not yet implemented).
            KeyCode::Char('S') => self.apply_motion(|b, s| cmd_split_selection_on_newlines(b, s)),
            // `(`/`)` — cycle the primary selection backward/forward.
            KeyCode::Char('(') => self.apply_motion(|b, s| cmd_cycle_primary_backward(b, s)),
            KeyCode::Char(')') => self.apply_motion(|b, s| cmd_cycle_primary_forward(b, s)),
            // `C` — duplicate the selection onto the next line (multicursor).
            KeyCode::Char('C') => self.apply_motion(|b, s| cmd_copy_selection_on_next_line(b, s)),
            // `_` — trim leading/trailing whitespace from each selection.
            KeyCode::Char('_') => self.apply_motion(|b, s| cmd_trim_selection_whitespace(b, s)),

            // ── Edit ──────────────────────────────────────────────────────────
            // `d` — delete selection and yank into default register.
            KeyCode::Char('d') => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.apply_edit(|b, s| delete_selection(b, s));
                self.registers.write(DEFAULT_REGISTER, yanked);
            }
            // `c` — change: yank, delete selection, then enter Insert mode.
            // The delete and everything typed before Esc form one undo step (Vim model):
            // we open the group here so the delete is folded in, then set_mode(Insert)
            // sees the group already open and skips begin_edit_group.
            KeyCode::Char('c') => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.begin_edit_group();
                self.doc.apply_edit_grouped(|b, s| delete_selection(b, s));
                self.registers.write(DEFAULT_REGISTER, yanked);
                self.set_mode(Mode::Insert);
            }
            // `y` — yank selection into default register (no buffer change).
            KeyCode::Char('y') => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.registers.write(DEFAULT_REGISTER, yanked);
            }
            // `p` — paste after; if the selection is non-cursor, the displaced
            // text is swapped back into the default register.
            KeyCode::Char('p') => {
                if let Some(reg) = self.registers.read(DEFAULT_REGISTER) {
                    let values = reg.values().to_vec();
                    let displaced = self.doc.apply_edit(|b, s| paste_after(b, s, &values));
                    if displaced.iter().any(|s| !s.is_empty()) {
                        self.registers.write(DEFAULT_REGISTER, displaced);
                    }
                }
            }
            // `P` — paste before; same swap semantics as `p`.
            KeyCode::Char('P') => {
                if let Some(reg) = self.registers.read(DEFAULT_REGISTER) {
                    let values = reg.values().to_vec();
                    let displaced = self.doc.apply_edit(|b, s| paste_before(b, s, &values));
                    if displaced.iter().any(|s| !s.is_empty()) {
                        self.registers.write(DEFAULT_REGISTER, displaced);
                    }
                }
            }
            KeyCode::Char('u') => self.doc.undo(),
            KeyCode::Char('U') => self.doc.redo(),
            // `r` — replace: wait for the next character, then replace every
            // character in every selection with it (handled in pending dispatch above).
            // `Ctrl+r` — redo (same key, modifier distinguishes).
            KeyCode::Char('r') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.doc.redo();
                } else {
                    self.pending = PendingKey::Replace;
                }
            }

            // ── Find/till character ───────────────────────────────────────────
            // `f`/`F` — jump to next/previous occurrence of a character (inclusive).
            // `t`/`T` — jump to just before/after the character (exclusive).
            // The next keystroke supplies the target character (pending dispatch above).
            KeyCode::Char('f') => self.pending = PendingKey::FindForward,
            KeyCode::Char('F') => self.pending = PendingKey::FindBackward,
            KeyCode::Char('t') => self.pending = PendingKey::TillForward,
            KeyCode::Char('T') => self.pending = PendingKey::TillBackward,

            // ── Repeat last find ──────────────────────────────────────────────
            // `=` — repeat forward (absolute direction, always goes right).
            // `-` — repeat backward (absolute direction, always goes left).
            // Both are no-ops when no prior f/t/F/T has been executed.
            KeyCode::Char('=') => {
                if let Some(FindChar { ch, kind }) = self.last_find {
                    let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                    self.apply_motion(|b, s| find_char_forward(b, s, mode, 1, ch, kind));
                }
            }
            KeyCode::Char('-') => {
                if let Some(FindChar { ch, kind }) = self.last_find {
                    let mode = if self.extend { MotionMode::Extend } else { MotionMode::Move };
                    self.apply_motion(|b, s| find_char_backward(b, s, mode, 1, ch, kind));
                }
            }

            // ── Text objects ──────────────────────────────────────────────────
            // `m` — enter match mode; next key selects inner (`i`) or around (`a`),
            // then the object char completes the sequence.
            KeyCode::Char('m') => self.pending = PendingKey::Match,

            // ── Line selection ────────────────────────────────────────────────
            // `x`: select the current line. If already on a full-line selection,
            // jump to the next line. Ctrl+x / extend mode accumulates lines.
            // `X`: same but walks backward; Ctrl+X accumulates backward.
            KeyCode::Char('x') => if key.modifiers.contains(KeyModifiers::CONTROL) || self.extend {
                self.apply_motion(|b, s| cmd_extend_select_line(b, s))
            } else {
                self.apply_motion(|b, s| cmd_select_line(b, s))
            },
            KeyCode::Char('X') => if key.modifiers.contains(KeyModifiers::CONTROL) || self.extend {
                self.apply_motion(|b, s| cmd_extend_select_line_backward(b, s))
            } else {
                self.apply_motion(|b, s| cmd_select_line_backward(b, s))
            },

            // ── Extend mode toggle ────────────────────────────────────────────
            // `e` toggles sticky extend mode: motions extend the selection instead of moving.
            KeyCode::Char('e') => self.extend = !self.extend,

            // ── Mode transitions ──────────────────────────────────────────────
            // `i` — enter Insert before the selection (collapse to start).
            KeyCode::Char('i') => {
                self.apply_motion(|_b, sels| sels.map(|s| Selection::cursor(s.start())));
                self.set_mode(Mode::Insert);
            }

            // `I` — enter Insert at the first non-blank character of the line.
            KeyCode::Char('I') => {
                self.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `a` — enter Insert after the cursor (one grapheme right).
            // If the cursor is on the structural '\n' (end of buffer), don't
            // advance further — there is nowhere to go.
            KeyCode::Char('a') => {
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `A` — enter Insert after the last character of the line.
            KeyCode::Char('A') => {
                self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // `o` — dual-purpose:
            //   extend mode: flip the anchor/head of every selection (Vim visual `o`).
            //   normal mode: open a new line below the current line and enter Insert.
            KeyCode::Char('o') => if self.extend {
                self.apply_motion(|b, s| cmd_flip_selections(b, s));
            } else {
                // Open the edit group *before* the newline insertion so that
                // the structural '\n' and everything typed before Esc form one
                // undo step (same pattern as `c`). set_mode(Insert) sees the
                // group is already open and skips begin_edit_group.
                self.doc.begin_edit_group();
                self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
                self.set_mode(Mode::Insert);
            },

            // `O` — open a new line above the current line and enter Insert.
            // Moves to the line start, inserts a '\n' (pushing current content
            // down), then steps left onto the new empty line.
            // Same undo-group strategy as `o`.
            KeyCode::Char('O') => {
                self.doc.begin_edit_group();
                self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1));
                self.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
                self.apply_motion(|b, s| cmd_move_left(b, s, 1));
                self.set_mode(Mode::Insert);
            }

            // Esc resets pending key sequence and extend mode.
            KeyCode::Esc => {
                self.pending = PendingKey::None;
                self.extend = false;
            }

            _ => {}
        }
    }

    // ── Insert mode ───────────────────────────────────────────────────────────

    fn handle_insert(&mut self, key: KeyEvent) {
        match key.code {
            // ── Return to Normal mode ─────────────────────────────────────────
            KeyCode::Esc => self.set_mode(Mode::Normal),

            // ── Character input ───────────────────────────────────────────────
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
                    self.doc.apply_edit_grouped(|b, s| delete_char_backward(b, s));
                }
            }
            KeyCode::Delete => {
                self.doc.apply_edit_grouped(|b, s| delete_char_forward(b, s));
            }

            // ── Navigation (same as Normal) ───────────────────────────────────
            KeyCode::Left  => self.apply_motion(|b, s| cmd_move_left(b, s, 1)),
            KeyCode::Right => self.apply_motion(|b, s| cmd_move_right(b, s, 1)),
            KeyCode::Down  => self.apply_motion(|b, s| cmd_move_down(b, s, 1)),
            KeyCode::Up    => self.apply_motion(|b, s| cmd_move_up(b, s, 1)),
            KeyCode::Home  => self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1)),
            KeyCode::End   => self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1)),

            _ => {}
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
            let prev = crate::grapheme::prev_grapheme_boundary(buf, sel.head);
            match (buf.char_at(prev), buf.char_at(sel.head)) {
                (Some(before), Some(at)) => {
                    pairs.iter().any(|p| p.open == before && p.close == at)
                }
                _ => false,
            }
        })
    }

    // ── Text object dispatch ──────────────────────────────────────────────────

    /// Dispatch a text-object command by object char.
    ///
    /// Called by the pending-key handler after `mi`/`ma` + object char.
    /// Returns `true` if `ch` matched a known object, `false` if unrecognized
    /// (caller falls through to normal dispatch).
    ///
    /// `inner == true` → select the interior (e.g. contents inside parens).
    /// `inner == false` → select around (e.g. parens themselves included).
    #[allow(non_snake_case)] // WORD (uppercase) is an intentional Vim/Helix concept
    fn dispatch_text_object(&mut self, ch: char, inner: bool) -> bool {
        if self.extend {
            match (ch, inner) {
                // ── Word / WORD ───────────────────────────────────────────────
                ('w', true)  => self.apply_motion(cmd_extend_inner_word),
                ('w', false) => self.apply_motion(cmd_extend_around_word),
                ('W', true)  => self.apply_motion(cmd_extend_inner_WORD),
                ('W', false) => self.apply_motion(cmd_extend_around_WORD),
                // ── Brackets ─────────────────────────────────────────────────
                ('(' | ')', true)  => self.apply_motion(cmd_extend_inner_paren),
                ('(' | ')', false) => self.apply_motion(cmd_extend_around_paren),
                ('[' | ']', true)  => self.apply_motion(cmd_extend_inner_bracket),
                ('[' | ']', false) => self.apply_motion(cmd_extend_around_bracket),
                ('{' | '}', true)  => self.apply_motion(cmd_extend_inner_brace),
                ('{' | '}', false) => self.apply_motion(cmd_extend_around_brace),
                ('<' | '>', true)  => self.apply_motion(cmd_extend_inner_angle),
                ('<' | '>', false) => self.apply_motion(cmd_extend_around_angle),
                // ── Quotes ───────────────────────────────────────────────────
                ('"', true)  => self.apply_motion(cmd_extend_inner_double_quote),
                ('"', false) => self.apply_motion(cmd_extend_around_double_quote),
                ('\'', true)  => self.apply_motion(cmd_extend_inner_single_quote),
                ('\'', false) => self.apply_motion(cmd_extend_around_single_quote),
                ('`', true)  => self.apply_motion(cmd_extend_inner_backtick),
                ('`', false) => self.apply_motion(cmd_extend_around_backtick),
                // ── Arguments ────────────────────────────────────────────────
                ('a', true)  => self.apply_motion(cmd_extend_inner_argument),
                ('a', false) => self.apply_motion(cmd_extend_around_argument),
                // ── Line ─────────────────────────────────────────────────────
                ('l', true)  => self.apply_motion(cmd_extend_inner_line),
                ('l', false) => self.apply_motion(cmd_extend_around_line),
                _ => return false,
            }
        } else {
            match (ch, inner) {
                // ── Word / WORD ───────────────────────────────────────────────
                ('w', true)  => self.apply_motion(cmd_inner_word),
                ('w', false) => self.apply_motion(cmd_around_word),
                ('W', true)  => self.apply_motion(cmd_inner_WORD),
                ('W', false) => self.apply_motion(cmd_around_WORD),
                // ── Brackets ─────────────────────────────────────────────────
                ('(' | ')', true)  => self.apply_motion(cmd_inner_paren),
                ('(' | ')', false) => self.apply_motion(cmd_around_paren),
                ('[' | ']', true)  => self.apply_motion(cmd_inner_bracket),
                ('[' | ']', false) => self.apply_motion(cmd_around_bracket),
                ('{' | '}', true)  => self.apply_motion(cmd_inner_brace),
                ('{' | '}', false) => self.apply_motion(cmd_around_brace),
                ('<' | '>', true)  => self.apply_motion(cmd_inner_angle),
                ('<' | '>', false) => self.apply_motion(cmd_around_angle),
                // ── Quotes ───────────────────────────────────────────────────
                ('"', true)  => self.apply_motion(cmd_inner_double_quote),
                ('"', false) => self.apply_motion(cmd_around_double_quote),
                ('\'', true)  => self.apply_motion(cmd_inner_single_quote),
                ('\'', false) => self.apply_motion(cmd_around_single_quote),
                ('`', true)  => self.apply_motion(cmd_inner_backtick),
                ('`', false) => self.apply_motion(cmd_around_backtick),
                // ── Arguments ────────────────────────────────────────────────
                ('a', true)  => self.apply_motion(cmd_inner_argument),
                ('a', false) => self.apply_motion(cmd_around_argument),
                // ── Line ─────────────────────────────────────────────────────
                ('l', true)  => self.apply_motion(cmd_inner_line),
                ('l', false) => self.apply_motion(cmd_around_line),
                _ => return false,
            }
        }
        true
    }

    // ── Command mode ──────────────────────────────────────────────────────────

    fn handle_command(&mut self, key: KeyEvent) {
        match key.code {
            // ── Cancel ────────────────────────────────────────────────────────
            KeyCode::Esc => {
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
        let content = if buf.line_ending() == crate::buffer::LineEnding::CrLf {
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
