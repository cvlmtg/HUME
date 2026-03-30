use std::io;
use std::path::PathBuf;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::execute;

use crate::auto_pairs::AutoPairsConfig;
use crate::core::buffer::Buffer;
use self::registry::CommandRegistry;
use crate::core::document::Document;
use crate::io::FileMeta;
use crate::ops::register::RegisterSet;
use crate::ui::renderer::{cursor_style, render};
use crate::core::selection::{Selection, SelectionSet};
use crate::ui::statusline::StatusLineConfig;
use crate::terminal::Term;
use crate::ui::theme::EditorColors;
use crate::ui::view::{compute_gutter_width, LineNumberStyle, ViewState};

use self::keymap::{Keymap, WaitCharPending};

mod registry;
mod commands;
mod keymap;
mod mappings;

// ── Dot-repeat state ─────────────────────────────────────────────────────────

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset —
/// changesets are position-dependent and can't be replayed at a different cursor.
#[derive(Debug, Clone)]
pub(super) struct RepeatableAction {
    /// The command name that initiated this action (e.g. `"delete"`, `"change"`).
    pub command: &'static str,
    /// The count prefix used originally. Overridden when `.` itself is given a count.
    pub count: usize,
    /// Character argument for wait-char commands (`r`, `f`, `t`, …).
    /// `None` for commands that don't consume a char.
    pub char_arg: Option<char>,
    /// Keystrokes typed during the insert session, if any.
    ///
    /// Populated by the insert-mode recording path when the command transitions
    /// to Insert mode. Empty for non-insert actions like `delete` or `paste-after`.
    pub insert_keys: Vec<KeyEvent>,
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
pub(crate) struct MiniBuffer {
    /// The character shown before the input, e.g. `:` for commands, `/` for search.
    pub prompt: char,
    /// The text typed so far.
    pub input: String,
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    pub(crate) doc: Document,
    pub(crate) view: ViewState,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) mode: Mode,
    /// When `true`, all motions extend the current selection rather than moving it.
    /// Toggled by `e` in Normal mode; cleared on entering Insert mode or pressing Esc.
    pub(crate) extend: bool,
    /// Keys consumed so far in the current multi-key sequence (max depth 3).
    ///
    /// Empty when at the trie root. Re-walked from the root on each new keypress.
    /// Cleared on Esc, on a successful command dispatch, or on NoMatch.
    pub(super) pending_keys: Vec<KeyEvent>,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    ///
    /// `None` until the user starts typing digits. Defaults to `1` at dispatch.
    pub(super) count: Option<usize>,
    /// Pending wait-char state for a f/t/F/T/r binding.
    ///
    /// When `Some`, the next character keypress is consumed as an argument,
    /// stored in `pending_char`, and the named command is dispatched.
    /// Cleared immediately after use.
    pub(super) wait_char: Option<WaitCharPending>,
    /// Character argument for the current parameterized command (find/till/replace).
    ///
    /// Set just before dispatching a wait-char command; consumed (`.take()`) by
    /// `dispatch_editor_cmd`. Always `None` between commands.
    pub(super) pending_char: Option<char>,
    pub(super) registers: RegisterSet,
    pub(crate) colors: EditorColors,
    pub(super) should_quit: bool,
    /// Active when the user is typing a command (`:`) or, later, a search (`/`).
    /// `None` when the mini-buffer is not visible.
    pub(crate) minibuf: Option<MiniBuffer>,
    /// Transient one-line message shown in the status bar after an action
    /// (e.g. "Written 42 lines", "Error: no file name"). Cleared on the next keypress.
    pub(crate) status_msg: Option<String>,
    /// Metadata captured from the file at open time (permissions, ownership,
    /// resolved path). `None` for scratch buffers. Used by the write path to
    /// preserve the original file's attributes across atomic saves.
    pub(super) file_meta: Option<FileMeta>,
    /// Status bar layout configuration.
    ///
    /// Initialized with [`StatusLineConfig::default`] (mode pill + separator +
    /// filename on the left, position on the right). Configurable via the
    /// Steel scripting layer.
    pub(crate) statusline_config: StatusLineConfig,
    /// Registry of all mappable commands (motions, selections, edits).
    ///
    /// Keyed by name; looked up by `execute_keymap_command` when dispatching
    /// [`KeymapCommand::Cmd`] bindings.
    pub(super) registry: CommandRegistry,
    /// The trie-based keymap for each mode.
    ///
    /// Built once at startup from [`Keymap::default`]. Extended by the Steel
    /// config layer to support user overrides.
    pub(super) keymap: Keymap,
    /// Auto-pair configuration (bracket/quote completion, skip-close, auto-delete).
    ///
    /// Initialized with sensible defaults. Configurable globally or per language
    /// via the Steel scripting layer.
    pub(super) auto_pairs: AutoPairsConfig,
    /// The character and kind (inclusive/exclusive) from the last f/t/F/T motion.
    ///
    /// Used by the repeat keys: `=` repeats the search forward, `-` backward.
    /// `None` until the user performs a find/till motion.
    pub(super) last_find: Option<FindChar>,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    ///
    /// When `true`, the terminal sends CSI-u sequences that disambiguate
    /// Ctrl+h from Backspace, Ctrl+j from Enter, etc. — unlocking Ctrl+motion
    /// one-shot extend shortcuts. Set by the caller after [`Editor::open`].
    pub(crate) kitty_enabled: bool,

    // ── Dot-repeat fields ─────────────────────────────────────────────────────

    /// The last repeatable editing action, available for replay via `.`.
    /// `None` until the user performs a repeatable command.
    pub(super) last_action: Option<RepeatableAction>,
    /// Insert-mode keystroke buffer, active while recording an insert session.
    ///
    /// `Some` between entering Insert mode (via a repeatable command) and
    /// returning to Normal mode. Text-input keys (Char, Enter, Backspace, Delete)
    /// are pushed here so they can be replayed by `.`. Navigation keys (arrows,
    /// Esc) are not recorded — consistent with Vim's repeat semantics.
    /// `None` at all other times.
    pub(super) insert_recording: Option<Vec<KeyEvent>>,
    /// Whether the user explicitly typed a count prefix before the current command.
    ///
    /// Set in `handle_normal` when `self.count` is `Some` before being consumed.
    /// Read by `cmd_repeat` to decide whether to use the new count or reuse the
    /// original action's count. Cleared after every dispatch.
    pub(super) explicit_count: bool,
    /// `true` while `cmd_repeat` is re-executing a recorded action.
    ///
    /// Prevents the replayed command from overwriting `last_action` and prevents
    /// `set_mode` from opening a new `insert_recording` during replay.
    pub(super) replaying: bool,
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
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            colors: EditorColors::default(),
            should_quit: false,
            minibuf: None,
            status_msg: None,
            file_meta,
            statusline_config: StatusLineConfig::default(),
            registry: CommandRegistry::with_defaults(),
            keymap: Keymap::default(),
            auto_pairs: AutoPairsConfig::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_recording: None,
            explicit_count: false,
            replaying: false,
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
            // All mutations are done above. Rust allows a shared reborrow of
            // `self` here since no mutable reference is live at this point.
            // Highlights (bracket match, etc.) are computed inside render().
            term.draw(|frame| {
                let cursor = render(self, frame.area(), frame.buffer_mut());
                if let Some(pos) = cursor.pos {
                    frame.set_cursor_position(pos);
                }
            })?;

            // ── 4b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let _ = execute!(std::io::stdout(), cursor_style(self.mode));

            // ── 5 & 6. Event ──────────────────────────────────────────────────
            match event::read()? {
                // Release events arrive only with kitty keyboard protocol
                // (REPORT_EVENT_TYPES flag). Ignore them — we act on Press and
                // Repeat (held key). Without kitty all events are Press anyway.
                Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
                Event::Key(_) => {}
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
                // Only open a new group if one isn't already open. `c` and
                // `open-line-*` open the group themselves (folding structural
                // edits in) before calling set_mode.
                if !self.doc.is_group_open() {
                    self.doc.begin_edit_group();
                }
                // Start recording insert keystrokes for `.` repeat, but skip
                // this during replay (we're feeding recorded keys, not new ones).
                if !self.replaying {
                    self.insert_recording = Some(Vec::new());
                }
            }
            (Mode::Insert, Mode::Normal) => {
                // Leaving Insert: commit all accumulated edits as one undo step.
                self.doc.commit_edit_group();
                // Finalize the insert recording into last_action (if both exist).
                if let (Some(keys), Some(action)) =
                    (self.insert_recording.take(), self.last_action.as_mut())
                {
                    action.insert_keys = keys;
                }
            }
            // Other transitions (e.g. Normal → Command) do not affect undo groups.
            // Clear any stale insert_recording defensively — it should never be
            // Some here in normal usage, but belt-and-suspenders prevents a
            // half-recorded session from leaking into the next insert entry.
            _ => {
                self.insert_recording = None;
            }
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

// ── Test constructors ─────────────────────────────────────────────────────────

#[cfg(test)]
impl Editor {
    /// Construct a minimal `Editor` for renderer unit tests.
    ///
    /// Only `doc` and `view` are meaningful — all other fields are set to
    /// sensible defaults (Normal mode, default colors, no file path, etc.).
    /// Use the builder methods below to override specific fields.
    pub(crate) fn for_testing(doc: Document, view: ViewState) -> Self {
        Self {
            doc,
            view,
            file_path: None,
            mode: Mode::Normal,
            extend: false,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            colors: EditorColors::default(),
            should_quit: false,
            minibuf: None,
            status_msg: None,
            file_meta: None,
            statusline_config: StatusLineConfig::default(),
            registry: registry::CommandRegistry::with_defaults(),
            keymap: keymap::Keymap::default(),
            auto_pairs: crate::auto_pairs::AutoPairsConfig::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_recording: None,
            explicit_count: false,
            replaying: false,
        }
    }

    pub(crate) fn with_mode(mut self, mode: Mode) -> Self { self.mode = mode; self }
    pub(crate) fn with_extend(mut self, extend: bool) -> Self { self.extend = extend; self }
    pub(crate) fn with_file_path(mut self, path: PathBuf) -> Self { self.file_path = Some(path); self }
    pub(crate) fn with_statusline_config(mut self, config: StatusLineConfig) -> Self {
        self.statusline_config = config;
        self
    }
    pub(crate) fn with_minibuf(mut self, prompt: char, input: &str) -> Self {
        self.minibuf = Some(MiniBuffer { prompt, input: input.to_string() });
        self
    }
}

#[cfg(test)]
mod tests;
