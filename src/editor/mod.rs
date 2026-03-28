use std::io;
use std::path::PathBuf;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event};
use crossterm::execute;
use unicode_width::UnicodeWidthStr;

use crate::auto_pairs::AutoPairsConfig;
use crate::buffer::Buffer;
use crate::command::CommandRegistry;
use crate::document::Document;
use crate::highlight::HighlightSet;
use crate::io::FileMeta;
use crate::register::RegisterSet;
use crate::renderer::{cursor_screen_pos, render, RenderCtx};
use crate::selection::{Selection, SelectionSet};
use crate::statusline::StatusLineConfig;
use crate::terminal::Term;
use crate::text_object::find_bracket_pair;
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
    /// After `f` — waiting for the character to find forward (inclusive).
    FindForward,
    /// After `F` — waiting for the character to find backward (inclusive).
    FindBackward,
    /// After `t` — waiting for the character to find forward (exclusive: stop before).
    TillForward,
    /// After `T` — waiting for the character to find backward (exclusive: stop after).
    TillBackward,
}

// ── Find/till state ───────────────────────────────────────────────────────────

/// Whether an f/t motion places the cursor on the found character or adjacent to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FindKind {
    /// `f`/`F`: cursor lands ON the found character.
    Inclusive,
    /// `t`/`T`: cursor lands one grapheme before (forward) or after (backward) it.
    Exclusive,
}

/// The character and kind stored by the last f/t/F/T motion.
///
/// Direction is NOT stored — the repeat keys `=` (forward) and `-` (backward)
/// use absolute direction, so re-searching always means "next on the right" or
/// "previous on the left" regardless of whether the original motion was f or F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FindChar {
    pub ch: char,
    pub kind: FindKind,
}

// ── Mode ──────────────────────────────────────────────────────────────────────

/// The current editing mode.
///
/// Starts as `Normal`. `Insert` is entered via `i`/`a` and exited via `Escape`.
/// `Command` is entered via `:` and exited via `Enter` (execute) or `Esc` (cancel).
/// The keymap is completely different in each mode — `handle_key` dispatches
/// to the appropriate handler accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Insert,
    Command,
}

// ── MiniBuffer ────────────────────────────────────────────────────────────────

/// The command-line mini-buffer, active while the user is typing a command.
///
/// Designed to be reused for search (`/`) in M4 — `prompt` distinguishes
/// the context without needing separate mode variants for each prompt type.
pub(super) struct MiniBuffer {
    /// The character shown before the input, e.g. `:` for commands, `/` for search.
    pub prompt: char,
    /// The text typed so far.
    pub input: String,
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    pub(super) doc: Document,
    pub(super) view: ViewState,
    pub(super) file_path: Option<PathBuf>,
    pub(super) mode: Mode,
    /// When `true`, all motions extend the current selection rather than moving it.
    /// Toggled by `e` in Normal mode; cleared on entering Insert mode or pressing Esc.
    pub(super) extend: bool,
    pub(super) pending: PendingKey,
    pub(super) registers: RegisterSet,
    pub(super) colors: EditorColors,
    pub(super) should_quit: bool,
    /// Active when the user is typing a command (`:`) or, later, a search (`/`).
    /// `None` when the mini-buffer is not visible.
    pub(super) minibuf: Option<MiniBuffer>,
    /// Transient one-line message shown in the status bar after an action
    /// (e.g. "Written 42 lines", "Error: no file name"). Cleared on the next keypress.
    pub(super) status_msg: Option<String>,
    /// Metadata captured from the file at open time (permissions, ownership,
    /// resolved path). `None` for scratch buffers. Used by the write path to
    /// preserve the original file's attributes across atomic saves.
    pub(super) file_meta: Option<FileMeta>,
    /// Status bar layout configuration.
    ///
    /// Initialized with [`StatusLineConfig::default`] (mode pill + separator +
    /// filename on the left, position on the right). The Steel scripting layer
    /// will replace this with the user's configured value when it is ready.
    pub(super) statusline_config: StatusLineConfig,
    /// Registry of all mappable commands (motions, selections, edits).
    ///
    /// The keymap trie (M4) will use this to translate command names to
    /// function pointers, replacing the hardcoded `match` arms in `handle_normal`.
    #[allow(dead_code)] // consumed by keymap trie (M4)
    pub(super) registry: CommandRegistry,
    /// Auto-pair configuration (bracket/quote completion, skip-close, auto-delete).
    ///
    /// Initialized with sensible defaults. The Steel scripting layer will allow
    /// users to override this globally or per language once scripting is ready.
    pub(super) auto_pairs: AutoPairsConfig,
    /// The character and kind (inclusive/exclusive) from the last f/t/F/T motion.
    ///
    /// Used by the repeat keys: `=` repeats the search forward, `-` backward.
    /// `None` until the user performs a find/till motion.
    pub(super) last_find: Option<FindChar>,
}

impl Editor {
    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0 in Normal mode. Terminal dimensions are
    /// placeholder values replaced on the first event-loop iteration.
    pub(crate) fn open(file_path: Option<PathBuf>) -> io::Result<Self> {
        let (buf, file_meta) = match &file_path {
            Some(path) => {
                let (content, meta) = crate::io::read_file(path)?;
                (Buffer::from(content.as_str()), Some(meta))
            }
            None => (Buffer::empty(), None),
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
            minibuf: None,
            status_msg: None,
            file_meta,
            statusline_config: StatusLineConfig::default(),
            registry: CommandRegistry::with_defaults(),
            auto_pairs: AutoPairsConfig::default(),
            last_find: None,
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

            // ── 3b. Bracket match highlight ───────────────────────────────────
            // When the primary cursor sits on a bracket, highlight the matching
            // partner. Suppressed in Insert mode — the bar cursor doesn't "sit
            // on" a character the same way.
            // `owned_hl` holds the built set when highlights are computed; in
            // Insert mode we skip computation and borrow the static EMPTY instead.
            let owned_hl;
            let highlights: &HighlightSet = if self.mode != Mode::Insert {
                let head = self.doc.sels().primary().head;
                let mut hl = HighlightSet::new();
                if let Some(ch) = self.doc.buf().char_at(head) {
                    let pair = match ch {
                        '(' | ')' => Some(('(', ')')),
                        '[' | ']' => Some(('[', ']')),
                        '{' | '}' => Some(('{', '}')),
                        '<' | '>' => Some(('<', '>')),
                        _ => None,
                    };
                    if let Some((open, close)) = pair
                        && let Some((op, cp)) = find_bracket_pair(self.doc.buf(), head, open, close)
                    {
                        // Highlight the OTHER bracket — the cursor already marks the one it's on.
                        let match_pos = if head == op { cp } else { op };
                        hl.push(match_pos, match_pos, self.colors.bracket_match);
                    }
                }
                owned_hl = hl.build();
                &owned_hl
            } else {
                &crate::highlight::EMPTY  // static — zero allocation
            };

            // ── 4. Render ─────────────────────────────────────────────────────
            // Capture references before the draw closure so the borrow checker
            // sees them as separate borrows of distinct fields, not of `self`.
            let ctx = RenderCtx {
                doc: &self.doc,
                view: &self.view,
                file_path: self.file_path.as_deref(),
                mode: self.mode,
                extend: self.extend,
                colors: &self.colors,
                minibuf: self.minibuf.as_ref().map(|m| (m.prompt, m.input.as_str())),
                status_msg: self.status_msg.as_deref(),
                statusline_config: &self.statusline_config,
                highlights,
            };
            term.draw(|frame| {
                render(&ctx, frame.area(), frame.buffer_mut());
                // In Insert and Command mode, show the real terminal cursor (bar).
                // Normal mode uses the white-block cursor_head cell style — no real cursor needed.
                match ctx.mode {
                    Mode::Insert => {
                        if let Some(pos) = cursor_screen_pos(ctx.doc.buf(), ctx.view, ctx.doc.sels().primary().head) {
                            frame.set_cursor_position(pos);
                        }
                    }
                    Mode::Command => {
                        if let Some((_, input)) = ctx.minibuf {
                            // Layout: 1-col margin + prompt char = col 2, then display-width of input.
                            let col = 2 + UnicodeWidthStr::width(input) as u16;
                            let row = ctx.view.height as u16;
                            frame.set_cursor_position((col, row));
                        }
                    }
                    Mode::Normal => {}
                }
            })?;

            // ── 4b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let cursor_style = match self.mode {
                Mode::Normal => SetCursorStyle::SteadyBlock,
                Mode::Insert | Mode::Command => SetCursorStyle::SteadyBar,
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
            // Command mode transitions do not affect undo groups.
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
