use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Buffer;
use crate::document::Document;
use crate::motion::{
    cmd_goto_line_end, cmd_goto_line_start, cmd_move_down, cmd_move_left, cmd_move_right,
    cmd_move_up,
};
use crate::renderer::render;
use crate::selection::{Selection, SelectionSet};
use crate::terminal::Term;
use crate::view::{compute_gutter_width, LineNumberStyle, ViewState};

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    doc: Document,
    view: ViewState,
    file_path: Option<PathBuf>,
    should_quit: bool,
}

impl Editor {
    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0. Terminal dimensions are placeholder
    /// values that are replaced on the first iteration of [`run`]'s event loop.
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

        Ok(Self { doc, view, file_path, should_quit: false })
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
            term.draw(|frame| {
                render(doc, view, file_path, frame.area(), frame.buffer_mut());
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

    // ── Key handling ──────────────────────────────────────────────────────────

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            // ── Quit ──────────────────────────────────────────────────────────
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }

            // ── Horizontal motion ─────────────────────────────────────────────
            KeyCode::Right => self.apply_motion(|b, s| cmd_move_right(b, s, 1)),
            KeyCode::Left => self.apply_motion(|b, s| cmd_move_left(b, s, 1)),

            // ── Vertical motion ───────────────────────────────────────────────
            KeyCode::Down => self.apply_motion(|b, s| cmd_move_down(b, s, 1)),
            KeyCode::Up => self.apply_motion(|b, s| cmd_move_up(b, s, 1)),

            // ── Page scroll ───────────────────────────────────────────────────
            // Move the cursor by a full viewport height; ensure_cursor_visible
            // then scrolls the viewport to follow.
            KeyCode::PageDown => {
                let count = self.view.height.max(1);
                self.apply_motion(|b, s| cmd_move_down(b, s, count));
            }
            KeyCode::PageUp => {
                let count = self.view.height.max(1);
                self.apply_motion(|b, s| cmd_move_up(b, s, count));
            }

            // ── Line start / end ──────────────────────────────────────────────
            KeyCode::Home => self.apply_motion(|b, s| cmd_goto_line_start(b, s, 1)),
            KeyCode::End => self.apply_motion(|b, s| cmd_goto_line_end(b, s, 1)),

            _ => {}
        }
    }

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
}
