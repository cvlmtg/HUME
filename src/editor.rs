use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Buffer;
use crate::document::Document;
use crate::edit::{
    delete_char_backward, delete_char_forward, delete_selection, insert_char, paste_after,
    paste_before,
};
use crate::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start, cmd_move_down,
    cmd_move_left, cmd_move_right, cmd_move_up, cmd_next_paragraph, cmd_prev_paragraph,
    cmd_select_next_WORD, cmd_select_next_word, cmd_select_prev_WORD, cmd_select_prev_word,
};
use crate::register::{yank_selections, RegisterSet, DEFAULT_REGISTER};
use crate::renderer::render;
use crate::selection::{Selection, SelectionSet};
use crate::selection_cmd::{
    cmd_collapse_selection, cmd_copy_selection_on_next_line, cmd_cycle_primary_backward,
    cmd_cycle_primary_forward, cmd_keep_primary_selection,
};
use crate::terminal::Term;
use crate::text_object::{
    cmd_around_WORD, cmd_around_backtick, cmd_around_brace, cmd_around_bracket,
    cmd_around_double_quote, cmd_around_paren, cmd_around_single_quote, cmd_around_word,
    cmd_inner_WORD, cmd_inner_angle, cmd_inner_backtick, cmd_inner_brace, cmd_inner_bracket,
    cmd_inner_double_quote, cmd_inner_paren, cmd_inner_single_quote, cmd_inner_word,
    cmd_around_angle,
};
use crate::view::{compute_gutter_width, LineNumberStyle, ViewState};

// ── PendingKey ────────────────────────────────────────────────────────────────

/// Tracks multi-key sequences that require waiting for additional key presses.
///
/// Text objects use a 3-key sequence: `m` → `i`/`a` → object char.
/// For example, `mi(` selects the inner content of the nearest paren pair.
///
/// On any unrecognized key at any stage the pending state resets to `None` and
/// the key is re-dispatched normally (so `mq` quits rather than silently eating
/// the `q`). Esc always resets to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PendingKey {
    #[default]
    None,
    /// After `m` — waiting for `i` (inner) or `a` (around).
    Match,
    /// After `mi` — waiting for the object char.
    MatchInner,
    /// After `ma` — waiting for the object char.
    MatchAround,
}

// ── Mode ──────────────────────────────────────────────────────────────────────

/// The current editing mode.
///
/// Starts as `Normal`. `Insert` is entered via `i`/`a` and exited via `Escape`.
/// The keymap is completely different in each mode — `handle_key` dispatches
/// to `handle_normal` or `handle_insert` accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Insert,
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    doc: Document,
    view: ViewState,
    file_path: Option<PathBuf>,
    mode: Mode,
    pending: PendingKey,
    registers: RegisterSet,
    should_quit: bool,
}

impl Editor {
    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0 in Normal mode. Terminal dimensions are
    /// placeholder values replaced on the first event-loop iteration.
    pub(crate) fn open(file_path: Option<PathBuf>) -> io::Result<Self> {
        let buf = match &file_path {
            Some(path) => {
                let content = std::fs::read_to_string(path)?;
                Buffer::from(content.as_str())
            }
            None => Buffer::empty(),
        };

        let sels = SelectionSet::single(Selection::cursor(0));
        let doc = Document::new(buf, sels);

        // Placeholder dimensions — updated at the top of every event-loop
        // iteration before the first render.
        let view = ViewState {
            scroll_offset: 0,
            height: 24,
            width: 80,
            gutter_width: compute_gutter_width(doc.buf().len_lines()),
            line_number_style: LineNumberStyle::Hybrid,
        };

        Ok(Self {
            doc,
            view,
            file_path,
            mode: Mode::Normal,
            pending: PendingKey::None,
            registers: RegisterSet::new(),
            should_quit: false,
        })
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Sync viewport dimensions from the terminal.
    /// 2. Recompute gutter width (line count changes on every edit).
    /// 3. Scroll so the cursor stays on screen.
    /// 4. Render.
    /// 5. Block until the next terminal event.
    /// 6. Dispatch the event.
    pub(crate) fn run(&mut self, term: &mut Term) -> io::Result<()> {
        loop {
            // ── 1 & 2. Sync dimensions ────────────────────────────────────────
            let size = term.size()?;
            self.view.width = size.width as usize;
            // Reserve one row for the status bar.
            self.view.height = (size.height as usize).saturating_sub(1);
            self.view.gutter_width = compute_gutter_width(self.doc.buf().len_lines());

            // ── 3. Scroll ─────────────────────────────────────────────────────
            self.view.ensure_cursor_visible(self.doc.buf(), self.doc.sels());

            // ── 4. Render ─────────────────────────────────────────────────────
            // Capture references before the draw closure so the borrow checker
            // sees them as separate borrows of distinct fields, not of `self`.
            let doc = &self.doc;
            let view = &self.view;
            let file_path = self.file_path.as_deref();
            let mode = self.mode;
            term.draw(|frame| {
                render(doc, view, mode, file_path, frame.area(), frame.buffer_mut());
            })?;

            // ── 5 & 6. Event ──────────────────────────────────────────────────
            match event::read()? {
                Event::Key(key) => self.handle_key(key),
                Event::Resize(_, _) => {} // dimensions re-read at loop top
                _ => {}
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    // ── Key dispatch ──────────────────────────────────────────────────────────

    fn handle_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
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
                    PendingKey::None => unreachable!(),
                }
            }
            // Non-char key (e.g. Esc) or unrecognized char: reset and fall through.
            self.pending = PendingKey::None;
        }

        match key.code {
            // ── Quit (temporary until :q is implemented) ──────────────────────
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }

            // ── Basic motion ──────────────────────────────────────────────────
            KeyCode::Char('h') | KeyCode::Left  => self.apply_motion(|b, s| cmd_move_left(b, s, 1)),
            KeyCode::Char('l') | KeyCode::Right => self.apply_motion(|b, s| cmd_move_right(b, s, 1)),
            KeyCode::Char('j') | KeyCode::Down  => self.apply_motion(|b, s| cmd_move_down(b, s, 1)),
            KeyCode::Char('k') | KeyCode::Up    => self.apply_motion(|b, s| cmd_move_up(b, s, 1)),

            // ── Word motion ───────────────────────────────────────────────────
            KeyCode::Char('w') => self.apply_motion(|b, s| cmd_select_next_word(b, s, 1)),
            KeyCode::Char('W') => self.apply_motion(|b, s| cmd_select_next_WORD(b, s, 1)),
            KeyCode::Char('b') => self.apply_motion(|b, s| cmd_select_prev_word(b, s, 1)),
            KeyCode::Char('B') => self.apply_motion(|b, s| cmd_select_prev_WORD(b, s, 1)),

            // ── Line start / end ──────────────────────────────────────────────
            KeyCode::Char('0') | KeyCode::Home => self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1)),
            KeyCode::Char('$') | KeyCode::End => self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1)),
            KeyCode::Char('^') => self.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1)),

            // ── Paragraph motion ──────────────────────────────────────────────
            KeyCode::Char('{') => self.apply_motion(|b, s| cmd_prev_paragraph(b, s, 1)),
            KeyCode::Char('}') => self.apply_motion(|b, s| cmd_next_paragraph(b, s, 1)),

            // ── Page scroll ───────────────────────────────────────────────────
            KeyCode::PageDown => {
                let count = self.view.height.max(1);
                self.apply_motion(|b, s| cmd_move_down(b, s, count));
            }
            KeyCode::PageUp => {
                let count = self.view.height.max(1);
                self.apply_motion(|b, s| cmd_move_up(b, s, count));
            }

            // ── Selection ─────────────────────────────────────────────────────
            KeyCode::Char(';') => self.apply_motion(|b, s| cmd_collapse_selection(b, s)),
            KeyCode::Char(',') => self.apply_motion(|b, s| cmd_keep_primary_selection(b, s)),
            // `(`/`)` — cycle the primary selection backward/forward.
            KeyCode::Char('(') => self.apply_motion(|b, s| cmd_cycle_primary_backward(b, s)),
            KeyCode::Char(')') => self.apply_motion(|b, s| cmd_cycle_primary_forward(b, s)),
            // `C` — duplicate the selection onto the next line (multicursor).
            KeyCode::Char('C') => self.apply_motion(|b, s| cmd_copy_selection_on_next_line(b, s)),

            // ── Edit ──────────────────────────────────────────────────────────
            // `d` — delete selection and yank into default register.
            KeyCode::Char('d') => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.apply_edit(|b, s| delete_selection(b, s));
                self.registers.write(DEFAULT_REGISTER, yanked);
            }
            // `c` — change: yank, delete selection, then enter Insert mode.
            KeyCode::Char('c') => {
                let yanked = yank_selections(self.doc.buf(), self.doc.sels());
                self.doc.apply_edit(|b, s| delete_selection(b, s));
                self.registers.write(DEFAULT_REGISTER, yanked);
                self.mode = Mode::Insert;
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
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.doc.redo();
            }

            // ── Text objects ──────────────────────────────────────────────────
            // `m` — enter match mode; next key selects inner (`i`) or around (`a`),
            // then the object char completes the sequence.
            KeyCode::Char('m') => self.pending = PendingKey::Match,

            // ── Mode transitions ──────────────────────────────────────────────
            // `i` — enter Insert at current position
            KeyCode::Char('i') => self.mode = Mode::Insert,

            // `a` — enter Insert after the cursor (one grapheme right).
            // If the cursor is on the structural '\n' (end of buffer), don't
            // advance further — there is nowhere to go.
            KeyCode::Char('a') => {
                self.apply_motion(|b, s| cmd_move_right(b, s, 1));
                self.mode = Mode::Insert;
            }

            // Esc resets any pending key sequence (already in Normal mode).
            KeyCode::Esc => self.pending = PendingKey::None,

            _ => {}
        }
    }

    // ── Insert mode ───────────────────────────────────────────────────────────

    fn handle_insert(&mut self, key: KeyEvent) {
        match key.code {
            // ── Return to Normal mode ─────────────────────────────────────────
            KeyCode::Esc => self.mode = Mode::Normal,

            // ── Character input ───────────────────────────────────────────────
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.doc.apply_edit(|b, s| insert_char(b, s, ch));
            }

            // ── Newline ───────────────────────────────────────────────────────
            KeyCode::Enter => {
                self.doc.apply_edit(|b, s| insert_char(b, s, '\n'));
            }

            // ── Delete ────────────────────────────────────────────────────────
            KeyCode::Backspace => {
                self.doc.apply_edit(|b, s| delete_char_backward(b, s));
            }
            KeyCode::Delete => {
                self.doc.apply_edit(|b, s| delete_char_forward(b, s));
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

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Apply a motion command and store the resulting selection.
    ///
    /// The explicit block ensures the immutable borrow of `self.doc.buf()`
    /// ends before the mutable `set_selections` call — a requirement of the
    /// borrow checker even with NLL.
    fn apply_motion(&mut self, f: impl FnOnce(&Buffer, SelectionSet) -> SelectionSet) {
        let new_sels = {
            let buf = self.doc.buf();
            let sels = self.doc.sels().clone();
            f(buf, sels)
        };
        self.doc.set_selections(new_sels);
    }

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
            _ => return false,
        }
        true
    }
}
