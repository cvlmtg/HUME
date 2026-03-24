use std::io;
use std::path::PathBuf;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event};
use crossterm::execute;

use crate::buffer::Buffer;
use crate::document::Document;
use crate::register::RegisterSet;
use crate::renderer::{cursor_screen_pos, render};
use crate::selection::{Selection, SelectionSet};
use crate::terminal::Term;
use crate::theme::EditorColors;
use crate::view::{compute_gutter_width, LineNumberStyle, ViewState};

mod mappings;

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
pub(super) enum PendingKey {
    #[default]
    None,
    /// After `m` — waiting for `i` (inner) or `a` (around).
    Match,
    /// After `mi` — waiting for the object char.
    MatchInner,
    /// After `ma` — waiting for the object char.
    MatchAround,
    /// After `r` — waiting for the replacement character.
    Replace,
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
    pub(super) doc: Document,
    pub(super) view: ViewState,
    pub(super) file_path: Option<PathBuf>,
    pub(super) mode: Mode,
    /// When `true`, all motions extend the current selection rather than moving it.
    /// Toggled by `x` in Normal mode; cleared on entering Insert mode or pressing Esc.
    pub(super) extend: bool,
    pub(super) pending: PendingKey,
    pub(super) registers: RegisterSet,
    pub(super) colors: EditorColors,
    pub(super) should_quit: bool,
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
            extend: false,
            pending: PendingKey::None,
            registers: RegisterSet::new(),
            colors: EditorColors::default(),
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
            let extend = self.extend;
            let colors = &self.colors;
            term.draw(|frame| {
                render(doc, view, mode, extend, file_path, colors, frame.area(), frame.buffer_mut());
                // In Insert mode, show the real terminal cursor (bar) so
                // SetCursorStyle is visible. Normal mode uses the reversed-cell
                // rendering only — no real cursor, so the letter stays visible.
                if mode == Mode::Insert {
                    if let Some(pos) = cursor_screen_pos(doc.buf(), view, doc.sels().primary().head) {
                        frame.set_cursor_position(pos);
                    }
                }
            })?;

            // ── 4b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let cursor_style = match self.mode {
                Mode::Normal => SetCursorStyle::SteadyBlock,
                Mode::Insert => SetCursorStyle::SteadyBar,
            };
            let _ = execute!(std::io::stdout(), cursor_style);

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
        // Restore the user's default cursor shape before returning to the shell.
        execute!(std::io::stdout(), SetCursorStyle::DefaultUserShape)?;
        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    pub(super) fn set_mode(&mut self, mode: Mode) {
        match (self.mode, mode) {
            (Mode::Normal, Mode::Insert) => {
                self.extend = false;
                // Only open a new group if one isn't already open. `c` opens
                // the group itself (folding the delete in) before calling set_mode.
                if !self.doc.is_group_open() {
                    self.doc.begin_edit_group();
                }
            }
            (Mode::Insert, Mode::Normal) => {
                // Leaving Insert: commit all accumulated edits as one undo step.
                self.doc.commit_edit_group();
            }
            _ => {}
        }
        self.mode = mode;
    }

    /// Apply a motion command and store the resulting selection.
    ///
    /// The explicit block ensures the immutable borrow of `self.doc.buf()`
    /// ends before the mutable `set_selections` call — a requirement of the
    /// borrow checker even with NLL.
    pub(super) fn apply_motion(&mut self, f: impl FnOnce(&Buffer, SelectionSet) -> SelectionSet) {
        let new_sels = {
            let buf = self.doc.buf();
            let sels = self.doc.sels().clone();
            f(buf, sels)
        };
        self.doc.set_selections(new_sels);
    }
}

#[cfg(test)]
mod tests;
