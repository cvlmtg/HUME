use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::execute;

use engine::builtins::line_number::{LineNumberColumn, LineNumberStyle as EngineLineNumberStyle};
use engine::pane::{Pane, ViewportState};
use engine::pipeline::{BufferId, EngineView, LayoutTree, PaneId, RenderContext, SharedBuffer};
use engine::types::{EditorMode, Selection as EngineSelection};

use crate::core::buffer::Buffer;
use self::registry::CommandRegistry;
use crate::core::document::Document;
use crate::io::FileMeta;
use crate::ops::register::RegisterSet;
use crate::ops::search::{find_all_matches, search_match_info};
use crate::ops::pair::find_bracket_pair;
use crate::core::selection::{Selection, SelectionSet};
use crate::settings::EditorSettings;
use crate::terminal::Term;

use self::keymap::{Keymap, WaitCharPending};

mod registry;
mod commands;
mod keymap;
mod mappings;
mod minibuf;
mod mouse;
mod search_state;
pub(super) mod scroll;
mod visual_move;

pub(crate) use search_state::{SearchDirection, SearchState};

pub(crate) use minibuf::MiniBuffer;
use minibuf::MiniBufferEvent;

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// State for an active insert session (entered via a repeatable command).
///
/// Tracks keystrokes for dot-repeat recording. Created by
/// [`Editor::begin_insert_session`] and consumed by [`Editor::end_insert_session`].
///
/// `None` on the editor when there is no active session — including during
/// replay, where the replay path pre-opens the edit group to signal
/// [`begin_insert_session`] that recording should be suppressed.
pub(super) struct InsertSession {
    keystrokes: Vec<KeyEvent>,
}

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset —
/// changesets are position-dependent and can't be replayed at a different cursor.
#[derive(Debug, Clone)]
pub(super) struct RepeatableAction {
    /// The command name that initiated this action (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-in commands (zero allocation); `Cow::Owned` for
    /// dynamically-registered commands (e.g. from the Steel scripting layer).
    pub command: Cow<'static, str>,
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

// ── Macro recording state ─────────────────────────────────────────────────────

/// Pending state for the two-keystroke `q<reg>` / `Q<reg>` sequences.
///
/// Set when the user presses `q` or `Q` in normal mode; cleared when the
/// next keypress is consumed as the register name (or cancelled on Esc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MacroPending {
    /// `Q` was pressed — waiting for a register name to start recording.
    Record,
    /// `q` was pressed — waiting for a register name to start replay.
    Replay,
}

// ── Find/till state ───────────────────────────────────────────────────────────

/// Whether an f/t motion places the cursor on the found character or adjacent to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FindKind {
    /// `find-forward` / `find-backward`: cursor lands ON the found character.
    Inclusive,
    /// `till-forward` / `till-backward`: cursor lands one grapheme before (forward) or after (backward) it.
    Exclusive,
}

/// The character and kind stored by the last find/till motion.
///
/// Direction is NOT stored — `repeat-find-forward` and `repeat-find-backward`
/// use absolute direction, so re-searching always means "next on the right" or
/// "previous on the left" regardless of the original motion's direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FindChar {
    pub ch: char,
    pub kind: FindKind,
}

// ── Mode ──────────────────────────────────────────────────────────────────────
//
// The editor uses `engine::types::EditorMode` directly. It unifies the old
// `Mode` enum and the `extend: bool` field: sticky extend is represented as
// `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch local variable
// and is NOT a mode change.
//
// `pub(crate) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias without a rename migration sweep.
pub(crate) use engine::types::EditorMode as Mode;

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    pub(crate) doc: Document,
    pub(crate) file_path: Option<Arc<PathBuf>>,
    /// Current editing mode. `EditorMode::Extend` represents the sticky extend
    /// state (previously a separate `extend: bool` field). Mode is the single
    /// source of truth — `extend: bool` has been removed.
    pub(crate) mode: Mode,
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
    pub(super) should_quit: bool,
    /// Active when the user is typing a command (`:`) or, later, a search (`/`).
    /// `None` when the mini-buffer is not visible.
    pub(crate) minibuf: Option<MiniBuffer>,
    /// Transient one-line message shown in the statusline after an action
    /// (e.g. "Written 42 lines", "Error: no file name"). Cleared on the next keypress.
    pub(crate) status_msg: Option<String>,
    /// Metadata captured from the file at open time (permissions, ownership,
    /// resolved path). `None` for scratch buffers. Used by the write path to
    /// preserve the original file's attributes across atomic saves.
    pub(super) file_meta: Option<FileMeta>,
    /// All editor settings — global defaults and per-buffer-overridable values.
    ///
    /// This is the single source of truth for every configurable setting.
    /// Per-buffer overrides live on [`Document::overrides`]; resolution happens
    /// at read time via [`crate::settings::BufferOverrides`] accessor methods.
    pub(crate) settings: EditorSettings,
    /// Registry of all mappable commands (motions, selections, edits).
    ///
    /// Keyed by name; looked up by `execute_keymap_command` at dispatch time.
    /// Also stores extend-variant pairings (base command → extend command).
    pub(super) registry: CommandRegistry,
    /// The trie-based keymap for each mode.
    ///
    /// Built once at startup from [`Keymap::default`]. Extended by the Steel
    /// config layer to support user overrides.
    pub(super) keymap: Keymap,
    /// The character and kind (inclusive/exclusive) from the last find/till motion.
    ///
    /// Used by `repeat-find-forward` / `repeat-find-backward`.
    /// `None` until the user performs a find/till motion.
    pub(super) last_find: Option<FindChar>,

    // ── Search ────────────────────────────────────────────────────────────────
    pub(super) search: SearchState,

    // ── Select (s) ───────────────────────────────────────────────────────────
    /// Snapshot of selections taken when entering Select mode (`select-within`).
    /// Restored on cancel; discarded on confirm.
    pub(super) pre_select_sels: Option<SelectionSet>,

    // ── Engine rendering state ────────────────────────────────────────────────
    /// The engine's rendering state: layout, panes, buffers, theme.
    pub(crate) engine_view: EngineView,
    /// The single pane created in `open()`.
    pub(crate) pane_id: PaneId,
    /// The single buffer registered in `open()`.
    pub(crate) buffer_id: BufferId,
    /// Shared bracket match highlight data: `(line_idx, byte_start, byte_end)`.
    /// Written by `update_highlight_providers()` each frame; read by the provider.
    pub(crate) bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared search match highlight data: same shape as `bracket_hl_data`.
    pub(crate) search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,

    // ── Jump list ────────────────────────────────────────────────────────────
    /// Navigable history of cursor positions before large movements.
    /// `jump-backward` / `jump-forward` traverse the list.
    pub(super) jump_list: crate::core::jump_list::JumpList,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    ///
    /// When `true`, the terminal sends CSI-u sequences that disambiguate
    /// Ctrl+h from Backspace, Ctrl+j from Enter, etc. — unlocking Ctrl+motion
    /// one-shot extend shortcuts. Set by the caller after [`Editor::open`].
    pub(crate) kitty_enabled: bool,

    // ── Visual-line movement ──────────────────────────────────────────────────

    /// Per-selection sticky display columns for visual-line j/k movement.
    ///
    /// Keyed by each selection's `head` char offset. On the first j/k press
    /// an entry is computed and inserted for every selection; on subsequent
    /// consecutive presses the stored value is reused so each cursor can
    /// return to its original column after passing through shorter rows.
    /// Cleared on any non-vertical command.
    ///
    /// Using head position as the key avoids coupling display concerns to the
    /// core `Selection` type. After `merge_overlapping`, all head positions
    /// are unique, so the key space is always valid.
    pub(super) preferred_display_cols: HashMap<usize, u16>,

    /// Reusable scratch buffer for format operations in visual-line movement.
    ///
    /// Allocated once and reused every j/k press to avoid per-keypress
    /// heap allocation.
    pub(super) motion_format_scratch: engine::format::FormatScratch,

    // ── Dot-repeat fields ─────────────────────────────────────────────────────

    /// The last repeatable editing action, available for replay via `.`.
    /// `None` until the user performs a repeatable command.
    pub(super) last_action: Option<RepeatableAction>,
    /// Active insert session, present between [`begin_insert_session`] and
    /// [`end_insert_session`]. Keystroke recording for dot-repeat lives here.
    /// `None` at all other times — including during replay, where the replay
    /// path pre-opens the edit group to suppress session creation.
    pub(super) insert_session: Option<InsertSession>,
    /// Whether the user explicitly typed a count prefix before the current command.
    ///
    /// Set in `handle_normal` when `self.count` is `Some` before being consumed.
    /// Read by `cmd_repeat` to decide whether to use the new count or reuse the
    /// original action's count. Cleared after every dispatch.
    pub(super) explicit_count: bool,

    // ── Keyboard macro fields ─────────────────────────────────────────────────

    /// Active macro recording session.
    ///
    /// `Some((register, keys))` while recording is in progress; `None` otherwise.
    /// The register name was supplied after the initial `q` keypress.
    pub(super) macro_recording: Option<(char, Vec<KeyEvent>)>,

    /// Pending two-keystroke macro command.
    ///
    /// Set when `q` or `Q` is pressed; the next keypress is consumed as the
    /// register name. Cleared (and cancelled) on Esc or invalid input.
    pub(super) macro_pending: Option<MacroPending>,

    /// Queue of keys to replay before reading the next terminal event.
    ///
    /// Populated by the `q<reg>` replay path; drained by the main event loop
    /// one key at a time at the same stack depth as normal input. This avoids
    /// recursion for long macros and allows `should_quit` to be checked between
    /// replayed keys.
    pub(super) replay_queue: VecDeque<KeyEvent>,

    /// Single-frame flag: skip recording the current key.
    ///
    /// Set by the stop-recording `Q` intercept so that the stop key itself is
    /// not appended to the macro buffer. Checked and cleared unconditionally at
    /// the end of every `handle_key` call.
    pub(super) skip_macro_record: bool,

    /// True while the event loop is draining the replay queue.
    ///
    /// Used to suppress nested recording: a `Q` key inside a replayed macro
    /// must not start a new recording session. Checking this flag is more
    /// reliable than checking `replay_queue.is_empty()`, which becomes `true`
    /// at the exact moment the last replayed key is processed.
    pub(super) is_replaying: bool,

    // ── Mouse ─────────────────────────────────────────────────────────────────

    /// Anchor char offset set on `MouseButton::Left` down when `mouse_select`
    /// is enabled. Cleared on mouse up.
    pub(super) mouse_drag_anchor: Option<usize>,
}

// proptest requires `Debug` on strategy values; this minimal impl satisfies it.
#[cfg(test)]
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Editor(buf={:?}, mode={:?})", self.doc.buf().to_string(), self.mode)
    }
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

        let sels = SelectionSet::single(Selection::collapsed(0));
        let doc = Document::new(buf, sels);

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);

        // Intern highlight scopes before registering providers.
        let bracket_scope = engine_view.registry.intern("ui.cursor.match");
        let search_scope  = engine_view.registry.intern("ui.selection.search");

        // Register the shared highlight data arcs.
        let bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>> = Arc::new(RwLock::new(Vec::new()));
        let search_hl_data:  Arc<RwLock<Vec<(usize, usize, usize)>>> = Arc::new(RwLock::new(Vec::new()));

        // Insert a buffer — just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());

        // Build the initial pane.
        let mut providers = engine::providers::ProviderSet::new();
        providers.add_gutter_column(Box::new(
            LineNumberColumn::with_style(EngineLineNumberStyle::Hybrid)
        ));
        providers.add_highlight_source(Box::new(crate::ui::highlight_providers::SharedHighlighter {
            scope: bracket_scope,
            tier: engine::providers::HighlightTier::BracketMatch,
            data: Arc::clone(&bracket_hl_data),
        }));
        providers.add_highlight_source(Box::new(crate::ui::highlight_providers::SharedHighlighter {
            scope: search_scope,
            tier: engine::providers::HighlightTier::SearchMatch,
            data: Arc::clone(&search_hl_data),
        }));

        let settings = EditorSettings::default();

        let pane = Pane {
            buffer_id,
            viewport: ViewportState::new(80, 24),
            selections: vec![EngineSelection { anchor: 0, head: 0 }],
            primary_idx: 0,
            mode: EditorMode::Normal,
            wrap_mode: settings.wrap_mode.clone(),
            tab_width: settings.tab_width,
            whitespace: settings.whitespace.clone(),
            providers,
        };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let file_path_arc: Option<Arc<PathBuf>> = file_path.map(Arc::new);
        let jump_list_capacity = settings.jump_list_capacity;

        // Bake theme now that all scopes are interned.
        engine_view.theme.bake(&engine_view.registry);

        Ok(Self {
            doc,
            file_path: file_path_arc,
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            should_quit: false,
            minibuf: None,
            status_msg: None,
            file_meta,
            settings,
            registry: CommandRegistry::with_defaults(),
            keymap: Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            pre_select_sels: None,
            jump_list: crate::core::jump_list::JumpList::new(jump_list_capacity),
            engine_view,
            pane_id,
            buffer_id,
            bracket_hl_data,
            search_hl_data,
            preferred_display_cols: HashMap::new(),
            motion_format_scratch: engine::format::FormatScratch::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
        })
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Prepare the frame: sync all editor state to the engine pane.
    /// 2. Render.
    /// 3. Block until the next terminal event.
    /// 4. Dispatch the event.
    pub(crate) fn run(&mut self, term: &mut Term) -> io::Result<()> {
        // Render context lives here — allocated once, reused every frame.
        // It must be outside `self` so `HumeStatusline { editor: self }` can
        // borrow `self` immutably while ctx is borrowed mutably.
        let mut ctx = RenderContext::new();
        let mut last_cursor_color_mode: Option<EditorMode> = None;
        loop {
            // ── 1. Prepare frame (single sync point) ─────────────────────────
            let size = term.size()?;
            self.prepare_frame(size.width, size.height, &mut ctx);

            // ── 2. Render ─────────────────────────────────────────────────────
            // Compute terminal cursor position before the draw closure to avoid
            // split-borrow conflicts: pane borrows and rope borrows must end
            // before `&mut self.engine_view` is captured by the closure.
            let cursor_screen = if let Some(mb) = &self.minibuf {
                // Minibuf active (Command / Search): place the terminal cursor
                // in the statusline at the minibuf edit position.
                let statusline_row = size.height.saturating_sub(1);
                Some((mb.statusline_cursor_col(), statusline_row))
            } else if self.mode.cursor_is_bar() {
                // Insert / Select: place the terminal cursor at the document head.
                let cursor_char = self.doc.sels().primary().head;
                let (vp, wrap_mode, tab_width, whitespace, gutter_w) = {
                    let pane = &self.engine_view.panes[self.pane_id];
                    let gw = crate::cursor::gutter_width(pane.providers.gutter_columns(), self.doc.buf().len_lines());
                    (pane.viewport.clone(), pane.wrap_mode.clone(), pane.tab_width, pane.whitespace.clone(), gw)
                };
                crate::cursor::screen_pos(
                    &vp, self.doc.buf().rope(), cursor_char,
                    &wrap_mode, tab_width, &whitespace,
                    &mut ctx,
                ).map(|(col, row)| (col + gutter_w, row))
            } else {
                None
            };

            // The statusline provider borrows `self` — create it before the
            // draw closure so the lifetime is tied to this stack frame.
            let statusline = crate::ui::statusline::HumeStatusline { editor: self };

            // Split borrows: `engine_view` and `doc` are disjoint fields of `self`.
            let rope        = self.doc.buf().rope();
            let buffer_id   = self.buffer_id;
            let engine_view = &self.engine_view;
            term.draw(|frame| {
                engine_view.render(frame.area(), frame.buffer_mut(), |bid| {
                    if bid == buffer_id { Some(rope) } else { None }
                }, Some(&statusline), &mut ctx);
                if let Some((col, row)) = cursor_screen {
                    frame.set_cursor_position((col, row));
                }
            })?;

            // ── 2b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let _ = execute!(std::io::stdout(), crate::cursor::shape(self.mode));
            if last_cursor_color_mode != Some(self.mode) {
                let _ = crate::cursor::set_color_for_mode(self.mode);
                last_cursor_color_mode = Some(self.mode);
            }

            // ── 3. Event ──────────────────────────────────────────────────────
            match event::read()? {
                // Release events arrive only with kitty keyboard protocol
                // (REPORT_EVENT_TYPES flag). Ignore them — we act on Press and
                // Repeat (held key). Without kitty all events are Press anyway.
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_key(key);
                    self.update_search_cache();
                }
                Event::Key(_) => {}
                Event::Mouse(mouse) => {
                    self.handle_mouse(mouse);
                }
                Event::Resize(_, _) => {} // dimensions re-read at loop top
                _ => {}
            }

            if self.should_quit {
                break;
            }

            // ── 4. Drain macro replay queue ───────────────────────────────────
            // Drain after handling the terminal event so that a key that
            // populates the queue (e.g. the register name after `Q`) causes
            // replay to run immediately — the results are visible on the very
            // next frame rather than requiring an additional keypress.
            // `last_action` is saved/restored so replay does not corrupt dot-repeat.
            self.drain_replay_queue();
            // One cache update covers the entire replay batch — the search
            // cache only changes when the buffer revision changes, so calling
            // it per-key would redundantly clone the regex on every iteration.
            self.update_search_cache();
            if self.should_quit { break; }
        }
        // Restore the user's default cursor shape and colour before returning to the shell.
        execute!(std::io::stdout(), crossterm::cursor::SetCursorStyle::DefaultUserShape)?;
        let _ = crate::cursor::set_color_for_mode(EditorMode::Normal); // emits reset sequence
        Ok(())
    }

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// This is the **single sync point** between the editor and the engine.
    /// No other code path should write to `pane.mode`, `pane.selections`, or
    /// the highlight/statusline shared buffers — all such writes happen here,
    /// immediately before every `render()` call.
    fn prepare_frame(&mut self, terminal_width: u16, terminal_height: u16, ctx: &mut RenderContext) {
        // 1. Sync viewport dimensions.
        // Engine reserves 1 row for the statusline; the pane gets the rest.
        {
            let vp = self.viewport_mut();
            vp.width  = terminal_width;
            vp.height = terminal_height.saturating_sub(1);
        }

        // 2. Sync mode.
        self.engine_view.panes[self.pane_id].mode = self.mode;

        // 3. Push char-offset selections to the engine pane (no conversion needed).
        self.push_selections_to_pane();

        // 4. Push resolved per-buffer settings into the pane so the engine
        //    always reads the effective values, regardless of overrides.
        {
            let pane = &mut self.engine_view.panes[self.pane_id];
            pane.tab_width = self.doc.overrides.tab_width(&self.settings);
            pane.wrap_mode = self.doc.overrides.wrap_mode(&self.settings);
            pane.whitespace = self.doc.overrides.whitespace(&self.settings);
        }

        // 5. Scroll so the primary cursor stays visible.
        let cursor_char = self.doc.sels().primary().head;
        let v_margin = self.settings.scroll_margin;
        let h_margin = self.settings.scroll_margin_h;
        {
            // Destructure `*pane` to split field borrows: `&mut viewport` and
            // `&wrap_mode`/`&whitespace` are disjoint fields, so the compiler
            // allows them simultaneously without any cloning.
            let rope = self.doc.buf().rope();
            let pane = &mut self.engine_view.panes[self.pane_id];
            let Pane { ref mut viewport, ref wrap_mode, tab_width, ref whitespace, .. } = *pane;
            scroll::ensure_cursor_visible(viewport, rope, cursor_char, wrap_mode, tab_width, whitespace, &mut ctx.cursor_format, v_margin);
            scroll::ensure_cursor_visible_horizontal(viewport, rope, cursor_char, wrap_mode, tab_width, whitespace, &mut ctx.cursor_format, h_margin);
        }

        // 6. Sync highlight data (search matches, bracket matches) to shared
        //    Arc buffers read by the highlight providers during rendering.
        self.update_highlight_providers();
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    pub(crate) fn viewport(&self) -> &ViewportState {
        &self.engine_view.panes[self.pane_id].viewport
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut ViewportState {
        &mut self.engine_view.panes[self.pane_id].viewport
    }

    /// Convert the editor's char-offset selections to engine `DocPos`-based
    /// selections and push them to the engine pane.
    ///
    /// Called once per frame from `prepare_frame`. Selections are re-sorted by
    /// `head` for the engine (which requires head-order); `primary_idx` is updated
    /// to track the primary selection's position in that order.
    pub(crate) fn push_selections_to_pane(&mut self) {
        // The engine now uses the same char-offset representation as the editor,
        // so this is a direct copy with no rope lookups. The engine resolves char
        // offsets to line/column coordinates during rendering via `Grapheme::char_offset`.
        //
        // Borrow `doc` (immutable) and `engine_view` (mutable) simultaneously —
        // they are disjoint fields, so the compiler allows it.
        let primary_idx = self.doc.sels().primary_index();
        let pane = &mut self.engine_view.panes[self.pane_id];
        pane.selections.clear();

        // The engine requires selections sorted by `head`. The editor stores them
        // sorted by `start()` (min of anchor/head), which differs when selections
        // are backward (head < anchor). Collect with original indices, re-sort by
        // head, then find where the primary landed.
        let mut engine_sels: Vec<(usize, EngineSelection)> = self.doc.sels()
            .iter_sorted()
            .enumerate()
            .map(|(i, sel)| (i, EngineSelection { anchor: sel.anchor, head: sel.head }))
            .collect();
        engine_sels.sort_by_key(|(_, s)| s.head);
        pane.primary_idx = engine_sels.iter()
            .position(|(orig_i, _)| *orig_i == primary_idx)
            .unwrap_or(0);
        pane.selections.extend(engine_sels.into_iter().map(|(_, s)| s));
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Recompute and cache the match list and current/total count.
    ///
    /// Called once after each `handle_key` so the render path reads
    /// pre-computed values and does zero regex work.
    /// Skipped when no search is active and the cache is already clear.
    pub(super) fn update_search_cache(&mut self) {
        let Some(regex) = self.search.regex.clone() else {
            // No active search. The cache was already zeroed by clear() or was
            // never populated; nothing to do.
            return;
        };

        let revision = self.doc.revision_id();
        let head = self.doc.sels().primary().head;

        // Recompute the full match list only when the buffer content changed.
        if revision != self.search.cache_revision {
            self.search.matches = find_all_matches(self.doc.buf(), &regex);
            self.search.cache_revision = revision;
            // Head may not have changed, but match_count depends on the (now
            // stale) match list, so force it to recompute below.
            self.search.cache_head = usize::MAX;
        }

        // Recompute the current/total count only when the cursor moved.
        if head != self.search.cache_head {
            self.search.match_count = Some(search_match_info(&self.search.matches, head));
            self.search.cache_head = head;
        }
    }


    /// Write per-frame highlight data to the shared `Arc<RwLock<...>>` buffers
    /// read by `BracketMatchHighlighter` and `SearchMatchHighlighter`.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode.
    pub(super) fn update_highlight_providers(&mut self) {
        let buf = self.doc.buf();

        // Visible line range — skip matches outside the viewport (search matches
        // are sorted by document order, so we can break early past the bottom).
        let top_line = self.viewport().top_line;
        let bot_line = top_line + self.viewport().height as usize;

        // ── Search match highlights ───────────────────────────────────────────
        {
            let mut data = self.search_hl_data.write().expect("RwLock not poisoned");
            data.clear();
            // Hidden in Insert mode — matches aren't actionable while typing and
            // clutter the view. Same pattern as bracket match highlights below.
            if self.mode != EditorMode::Insert {
                // Matches are sorted by document order. Binary-search to the first
                // match that starts at or after `top_line` to skip pre-viewport entries.
                let top_char = buf.line_to_char(top_line.min(buf.len_lines().saturating_sub(1)));
                let matches = self.search.matches();
                let first = matches.partition_point(|&(start, _)| start < top_char);
                for &(start, end_incl) in &matches[first..] {
                    let (line, byte_start) = char_to_line_byte(buf, start);
                    if line > bot_line { break; }
                    // end_incl is inclusive char offset; +1 makes it exclusive in chars,
                    // then convert to byte.
                    let end_char = (end_incl + 1).min(buf.len_chars());
                    let (_, byte_end) = char_to_line_byte(buf, end_char);
                    data.push((line, byte_start, byte_end));
                }
            }
        }

        // ── Bracket match highlight ───────────────────────────────────────────
        {
            let mut data = self.bracket_hl_data.write().expect("RwLock not poisoned");
            data.clear();
            if self.mode != EditorMode::Insert {
                let head = self.doc.sels().primary().head;
                if let Some(ch) = buf.char_at(head) {
                    let pair = match ch {
                        '(' | ')' => Some(('(', ')')),
                        '[' | ']' => Some(('[', ']')),
                        '{' | '}' => Some(('{', '}')),
                        '<' | '>' => Some(('<', '>')),
                        _ => None,
                    };
                    if let Some((open, close)) = pair
                        && let Some((op, cp)) = find_bracket_pair(buf, head, open, close)
                    {
                        let match_pos = if head == op { cp } else { op };
                        let (line, byte) = char_to_line_byte(buf, match_pos);
                        // Single-char match: byte_end = byte + utf8 length of the char.
                        let ch_len = buf.char_at(match_pos).map(|c| c.len_utf8()).unwrap_or(1);
                        data.push((line, byte, byte + ch_len));
                    }
                }
            }
        }
    }

    /// Capture the current editor state into `statusline_data` so `HumeStatusline`
    /// renders a consistent snapshot each frame.
    ///
    /// Called once per frame before `term.draw`. Per-frame rebuild is cheap:
    /// a handful of clones of short strings.
    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// For Insert mode entry and exit use [`begin_insert_session`] and
    /// [`end_insert_session`] instead — they manage the undo group and
    /// dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
    }

    /// Enter Insert mode as a repeatable insert action.
    ///
    /// Opens a new undo edit group and starts keystroke recording for
    /// dot-repeat, then sets the mode to Insert.
    ///
    /// **Replay signal**: if an edit group is already open when this is called,
    /// recording is suppressed but the mode change still happens. The replay
    /// path in [`cmd_repeat`] pre-opens the group before re-executing the
    /// original command, so that the re-executed command's call here becomes a
    /// no-op for undo/repeat purposes — only the cursor motion takes effect.
    pub(super) fn begin_insert_session(&mut self) {
        if !self.doc.is_group_open() {
            self.doc.begin_edit_group();
            self.insert_session = Some(InsertSession { keystrokes: Vec::new() });
        }
        self.mode = Mode::Insert;
    }

    /// Exit Insert mode and finalise the undo/repeat state.
    ///
    /// Commits the open edit group (creating one undo step for the whole
    /// insert session) and moves the recorded keystrokes into `last_action`
    /// for dot-repeat, then sets the mode to Normal.
    pub(super) fn end_insert_session(&mut self) {
        self.doc.commit_edit_group();
        if let (Some(session), Some(action)) =
            (self.insert_session.take(), self.last_action.as_mut())
        {
            action.insert_keys = session.keystrokes;
        }
        // Engine pane is synced by `prepare_frame` each frame.
        self.mode = EditorMode::Normal;
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

    /// Drain the macro replay queue, executing each key in order.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions — including
    /// when the last key in the macro is `Q` (where `replay_queue.is_empty()`
    /// would already be `true` and would fail to suppress it).
    ///
    /// Saves and restores `last_action` so replay does not corrupt dot-repeat.
    pub(crate) fn drain_replay_queue(&mut self) {
        let saved_action = self.last_action.take();
        self.is_replaying = true;
        while let Some(key) = self.replay_queue.pop_front() {
            self.handle_key(key);
            if self.should_quit { break; }
        }
        self.is_replaying = false;
        self.last_action = saved_action;
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
    pub(crate) fn for_testing(doc: Document) -> Self {
        // Minimal engine view for test contexts. Uses 80×24 with tab_width=4.
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());
        let settings = EditorSettings::default();
        let jump_list_capacity = settings.jump_list_capacity;
        let pane = Pane {
            buffer_id,
            viewport: ViewportState::new(80, 24),
            selections: vec![EngineSelection { anchor: 0, head: 0 }],
            primary_idx: 0,
            mode: EditorMode::Normal,
            wrap_mode: settings.wrap_mode.clone(),
            tab_width: settings.tab_width,
            whitespace: settings.whitespace.clone(),
            providers: engine::providers::ProviderSet::new(),
        };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);
        engine_view.theme.bake(&engine_view.registry);
        Self {
            doc,
            file_path: None,
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            should_quit: false,
            minibuf: None,
            status_msg: None,
            file_meta: None,
            settings,
            registry: registry::CommandRegistry::with_defaults(),
            keymap: keymap::Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            pre_select_sels: None,
            jump_list: crate::core::jump_list::JumpList::new(jump_list_capacity),
            engine_view,
            pane_id,
            buffer_id,
            bracket_hl_data: Arc::new(RwLock::new(Vec::new())),
            search_hl_data: Arc::new(RwLock::new(Vec::new())),
            preferred_display_cols: HashMap::new(),
            motion_format_scratch: engine::format::FormatScratch::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
        }
    }

    pub(crate) fn with_search_regex(mut self, pattern: &str) -> Self {
        self.search.set_regex(regex_cursor::engines::meta::Regex::new(pattern).ok());
        self.update_search_cache();
        self
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` where `byte_in_line` is the byte offset
/// from the start of the line — suitable for building highlight spans that the
/// engine expects in line-relative byte coordinates.
fn char_to_line_byte(buf: &Buffer, char_pos: usize) -> (usize, usize) {
    let line = buf.char_to_line(char_pos);
    let line_start_byte = buf.char_to_byte(buf.line_to_char(line));
    let byte = buf.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}

#[cfg(test)]
mod tests;
