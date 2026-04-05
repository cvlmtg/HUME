use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::execute;

use engine::builtins::line_number::{LineNumberColumn, LineNumberStyle as EngineLineNumberStyle};
use engine::pane::{Pane, ViewportState, WrapMode};
use engine::pipeline::{BufferId, EditorView, LayoutTree, PaneId, SharedBuffer};
use engine::types::{DocPos, EditorMode, Selection as EngineSelection};

use crate::auto_pairs::AutoPairsConfig;
use crate::core::buffer::Buffer;
use self::registry::CommandRegistry;
use crate::core::document::Document;
use crate::io::FileMeta;
use crate::core::history::RevisionId;
use crate::ops::register::RegisterSet;
use crate::ops::search::{find_all_matches, search_match_info};
use crate::ops::text_object::find_bracket_pair;
use crate::ui::highlight::{HighlightKind, HighlightMap};
use crate::ui::renderer::{cursor_style, render};
use crate::core::selection::{Selection, SelectionSet};
use crate::ui::statusline::{StatusLineConfig, StatuslineSnapshot};
use crate::terminal::Term;
use crate::ui::theme::EditorColors;
use crate::ui::gutter::GutterConfig;
use crate::ui::view::{LineNumberStyle, ViewState};
use crate::ui::whitespace::WhitespaceConfig;

use self::keymap::{Keymap, WaitCharPending};

mod registry;
mod commands;
mod keymap;
mod mappings;
mod minibuf;

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

// ── Search state ──────────────────────────────────────────────────────────────

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}


/// All search-related state, grouped to keep the "is a search active?" invariant
/// in one place instead of scattered across five independent `Editor` fields.
pub(crate) struct SearchState {
    /// Direction of the current or last search. Set when entering Search mode;
    /// persists after confirming so live search knows which way to go.
    pub direction: SearchDirection,
    /// Snapshot of selections taken when entering Search mode.
    /// Restored on cancel; discarded on confirm.
    pub pre_search_sels: Option<SelectionSet>,
    /// Whether extend mode was active when this search was started.
    ///
    /// Captured at search-enter time (before mode becomes `Search`) so live
    /// search can extend from the pre-search anchor even though `mode` is now
    /// `Search` rather than `Extend`. Cleared with the rest of `SearchState`
    /// via [`clear`].
    pub extend: bool,
    /// Compiled regex from the last confirmed (or in-progress) search pattern.
    /// `None` until a valid pattern is typed. Reused by `search-next`/`search-prev` without recompiling.
    /// Mutate only through [`set_regex`] to keep the match cache coherent.
    regex: Option<regex_cursor::engines::meta::Regex>,
    /// All non-overlapping matches of `regex` in the current buffer,
    /// as `(start_char, end_char_inclusive)` pairs in document order.
    /// Kept up to date by `update_search_cache`; empty when `regex` is `None`.
    matches: Vec<(usize, usize)>,
    /// Cached `(current_1based, total)` derived from `matches` and the
    /// primary cursor position. `None` when `regex` is `None`.
    match_count: Option<(usize, usize)>,
    /// `true` when the last `search-next`/`search-prev` jump wrapped around the buffer boundary.
    /// Read by the `SearchMatches` statusline element to show a `W` prefix.
    wrapped: bool,

    // ── Cache-invalidation keys ───────────────────────────────────────────────
    // Stored so `update_search_cache` can skip recomputation when nothing changed.
    // Both start as sentinel values that never match real state, forcing a full
    // recompute on the very first call.

    /// Buffer revision when `matches` was last computed. Changes on any edit,
    /// undo, or redo. When this differs from `doc.revision_id()`, `matches`
    /// must be recomputed.
    cache_revision: RevisionId,
    /// Primary cursor head position when `match_count` was last computed.
    /// When this differs from the current head, `match_count` must be recomputed
    /// (but `matches` can be reused if the revision hasn't changed).
    cache_head: usize,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            direction: SearchDirection::Forward,
            pre_search_sels: None,
            extend: false,
            regex: None,
            matches: Vec::new(),
            match_count: None,
            wrapped: false,
            // Sentinel values: usize::MAX can never be a real revision or cursor
            // position, so the first call to update_search_cache always recomputes.
            cache_revision: RevisionId(usize::MAX),
            cache_head: usize::MAX,
        }
    }
}

impl SearchState {
    /// Clear the active search — drops the regex and flushes the highlight cache.
    /// Direction is preserved so a future `search-next`/`search-prev` or
    /// `search-forward`/`search-backward` still knows the last-used direction.
    pub fn clear(&mut self) {
        self.pre_search_sels = None;
        self.extend = false;
        self.wrapped = false;
        self.set_regex(None);
    }

    /// Replace the regex, invalidating the match-list cache.
    ///
    /// Always call this instead of writing `self.regex = …` directly so that
    /// `update_search_cache` knows the match list must be recomputed even when
    /// the buffer revision hasn't changed (e.g. a new character was typed in
    /// the search prompt).
    pub fn set_regex(&mut self, regex: Option<regex_cursor::engines::meta::Regex>) {
        self.regex = regex;
        self.matches.clear();
        self.match_count = None;
        self.wrapped = false;
        self.cache_revision = RevisionId(usize::MAX);
        self.cache_head = usize::MAX;
    }

    pub(crate) fn matches(&self) -> &[(usize, usize)] {
        &self.matches
    }

    pub(crate) fn match_count(&self) -> Option<(usize, usize)> {
        self.match_count
    }

    pub(crate) fn wrapped(&self) -> bool {
        self.wrapped
    }
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    pub(crate) doc: Document,
    pub(crate) view: ViewState,
    pub(crate) file_path: Option<PathBuf>,
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
    pub(crate) colors: EditorColors,
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
    /// Status bar layout configuration.
    ///
    /// Initialized with [`StatusLineConfig::default`] (mode indicator + separator +
    /// filename on the left, position on the right). Configurable via the
    /// Steel scripting layer.
    pub(crate) statusline_config: StatusLineConfig,
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
    /// Auto-pair configuration (bracket/quote completion, skip-close, auto-delete).
    ///
    /// Initialized with sensible defaults. Configurable globally or per language
    /// via the Steel scripting layer.
    pub(super) auto_pairs: AutoPairsConfig,
    /// The character and kind (inclusive/exclusive) from the last find/till motion.
    ///
    /// Used by `repeat-find-forward` / `repeat-find-backward`.
    /// `None` until the user performs a find/till motion.
    pub(super) last_find: Option<FindChar>,

    // ── Search ────────────────────────────────────────────────────────────────
    pub(super) search: SearchState,

    // ── Per-frame highlights ──────────────────────────────────────────────────
    /// Pre-computed highlight map for the current frame (bracket match, search).
    ///
    /// Updated by [`update_highlight_cache`] once per event-loop iteration,
    /// after scroll is resolved and before `term.draw`. The renderer reads
    /// this directly — no highlight logic runs inside the render path.
    pub(crate) highlights: HighlightMap,

    // ── Select (s) ───────────────────────────────────────────────────────────
    /// Snapshot of selections taken when entering Select mode (`select-within`).
    /// Restored on cancel; discarded on confirm.
    pub(super) pre_select_sels: Option<SelectionSet>,

    // ── Engine rendering state ────────────────────────────────────────────────
    /// The engine's rendering state: layout, panes, buffers, theme.
    pub(crate) engine_view: EditorView,
    /// The single pane created in `open()`.
    pub(crate) pane_id: PaneId,
    /// The single buffer registered in `open()`.
    pub(crate) buffer_id: BufferId,
    /// Shared bracket match highlight data: `(line_idx, byte_start, byte_end)`.
    /// Written by `update_highlight_providers()` each frame; read by the provider.
    pub(crate) bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared search match highlight data: same shape as `bracket_hl_data`.
    pub(crate) search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared statusline snapshot. Written per-frame; read by the provider.
    pub(crate) statusline_data: Arc<Mutex<StatuslineSnapshot>>,

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
        let cached_total_lines = doc.buf().len_lines().saturating_sub(1);
        let view = ViewState {
            scroll_offset: 0,
            height: 24,
            width: 80,
            gutter: GutterConfig::default(),
            cached_total_lines,
            line_number_style: LineNumberStyle::Hybrid,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: true,
            word_wrap: true,
            indent_wrap: true,
            scroll_sub_offset: 0,
        };

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EditorView::new(theme);

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
        providers.gutter_columns.push(Box::new(
            LineNumberColumn::with_style(0, EngineLineNumberStyle::Hybrid)
        ));
        providers.highlights.push(Box::new(crate::ui::highlight_providers::BracketMatchHighlighter {
            id: 1,
            scope: bracket_scope,
            data: Arc::clone(&bracket_hl_data),
        }));
        providers.highlights.push(Box::new(crate::ui::highlight_providers::SearchMatchHighlighter {
            id: 2,
            scope: search_scope,
            data: Arc::clone(&search_hl_data),
        }));

        let pane = Pane {
            buffer_id,
            viewport: ViewportState::new(80, 24),
            selections: vec![EngineSelection {
                anchor: DocPos { line: 0, byte_offset: 0 },
                head:   DocPos { line: 0, byte_offset: 0 },
            }],
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::Indent { width: 76 },
            tab_width: 4,
            whitespace: engine::pane::WhitespaceConfig::default(),
            providers,
        };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        // Statusline provider.
        let initial_snapshot = StatuslineSnapshot {
            mode: EditorMode::Normal,
            file_path: file_path.clone(),
            cursor_pos: (1, 1),
            kitty_enabled: false,
            is_dirty: false,
            match_count: None,
            search_wrapped: false,
            minibuf: None,
            status_msg: None,
            config: StatusLineConfig::default(),
            colors: EditorColors::default(),
        };
        let statusline_data = Arc::new(Mutex::new(initial_snapshot));
        engine_view.statusline = Some(Box::new(crate::ui::statusline::HumeStatusline {
            data: Arc::clone(&statusline_data),
        }));

        // Bake theme now that all scopes are interned.
        engine_view.theme.bake(&engine_view.registry);

        Ok(Self {
            doc,
            view,
            file_path,
            mode: Mode::Normal,
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
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            highlights: HighlightMap::new().build(),
            pre_select_sels: None,
            jump_list: crate::core::jump_list::JumpList::new(),
            engine_view,
            pane_id,
            buffer_id,
            bracket_hl_data,
            search_hl_data,
            statusline_data,
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
            // Reserve one row for the statusline.
            self.view.height = (size.height as usize).saturating_sub(1);
            self.view.cached_total_lines = self.doc.buf().len_lines().saturating_sub(1);

            // ── 3. Scroll ─────────────────────────────────────────────────────
            let cursor_char = self.doc.sels().primary().head;
            let cursor_line = self.doc.buf().char_to_line(cursor_char);
            self.view.ensure_cursor_visible(self.doc.buf(), cursor_char);
            self.view.ensure_cursor_visible_horizontal(self.doc.buf(), self.doc.sels(), cursor_line);

            // ── 4. Render ─────────────────────────────────────────────────────
            // All mutations are done above. Highlights are pre-computed once
            // here so the render closure only reads state — no logic inside draw.
            self.update_highlight_cache();
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
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_key(key);
                    self.update_search_cache();
                }
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

    // ── Engine accessors ──────────────────────────────────────────────────────

    pub(crate) fn viewport(&self) -> &ViewportState {
        &self.engine_view.panes[self.pane_id].viewport
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut ViewportState {
        &mut self.engine_view.panes[self.pane_id].viewport
    }

    pub(crate) fn pane(&self) -> &Pane {
        &self.engine_view.panes[self.pane_id]
    }

    pub(crate) fn pane_mut(&mut self) -> &mut Pane {
        &mut self.engine_view.panes[self.pane_id]
    }

    /// Convert the editor's char-offset selections to engine `DocPos`-based
    /// selections and push them to the engine pane.
    ///
    /// Called after every selection-mutating operation. Cheap per-action
    /// (O(N × log M) where N = selections, M = rope size).
    pub(crate) fn push_selections_to_pane(&mut self) {
        let buf = self.doc.buf();
        let engine_sels: Vec<EngineSelection> = self.doc.sels().iter_sorted()
            .map(|sel| {
                let anchor_line = buf.char_to_line(sel.anchor);
                let anchor_line_start = buf.char_to_byte(buf.line_to_char(anchor_line));
                let anchor_byte = buf.char_to_byte(sel.anchor);

                let head_line = buf.char_to_line(sel.head);
                let head_line_start = buf.char_to_byte(buf.line_to_char(head_line));
                let head_byte = buf.char_to_byte(sel.head);

                EngineSelection {
                    anchor: DocPos { line: anchor_line, byte_offset: anchor_byte - anchor_line_start },
                    head:   DocPos { line: head_line,   byte_offset: head_byte   - head_line_start   },
                }
            })
            .collect();
        self.engine_view.panes[self.pane_id].selections = engine_sels;
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

    /// Recompute and cache the per-frame highlight map.
    ///
    /// Called once per event-loop iteration, after scroll is resolved and before
    /// `term.draw`. Bracket matching is suppressed in Insert mode — the bar
    /// cursor doesn't "sit on" a character the same way a block cursor does.
    pub(crate) fn update_highlight_cache(&mut self) {
        if self.mode == EditorMode::Insert {
            self.highlights = HighlightMap::new().build();
            return;
        }

        let head = self.doc.sels().primary().head;
        let mut hl = HighlightMap::new();

        // ── Search match highlights ───────────────────────────────────────────
        for &(start, end_incl) in self.search.matches() {
            hl.push(start, end_incl, HighlightKind::SearchMatch);
        }

        // ── Bracket match highlight ───────────────────────────────────────────
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
                hl.push(match_pos, match_pos, HighlightKind::BracketMatch);
            }
        }

        self.highlights = hl.build();
    }

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// For Insert mode entry and exit use [`begin_insert_session`] and
    /// [`end_insert_session`] instead — they manage the undo group and
    /// dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        // Push-based pane sync: mode is SSOT, so keep the engine pane in sync.
        self.engine_view.panes[self.pane_id].mode = mode;
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
        // Mode is SSOT for extend state; transitioning to Insert implicitly clears Extend.
        self.mode = Mode::Insert;
        self.engine_view.panes[self.pane_id].mode = EditorMode::Insert;
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
        self.mode = EditorMode::Normal;
        self.engine_view.panes[self.pane_id].mode = EditorMode::Normal;
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
        self.push_selections_to_pane();
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
        // Minimal engine view for test contexts.
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EditorView::new(theme);
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());
        let pane = Pane {
            buffer_id,
            viewport: ViewportState::new(view.width as u16, view.height as u16),
            selections: vec![EngineSelection {
                anchor: DocPos { line: 0, byte_offset: 0 },
                head:   DocPos { line: 0, byte_offset: 0 },
            }],
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::Indent { width: (view.width as u16).saturating_sub(4) },
            tab_width: view.tab_width as u8,
            whitespace: engine::pane::WhitespaceConfig::default(),
            providers: engine::providers::ProviderSet::new(),
        };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);
        engine_view.theme.bake(&engine_view.registry);

        let statusline_data = Arc::new(Mutex::new(StatuslineSnapshot {
            mode: EditorMode::Normal,
            file_path: None,
            cursor_pos: (1, 1),
            kitty_enabled: false,
            is_dirty: false,
            match_count: None,
            search_wrapped: false,
            minibuf: None,
            status_msg: None,
            config: StatusLineConfig::default(),
            colors: EditorColors::default(),
        }));

        Self {
            doc,
            view,
            file_path: None,
            mode: Mode::Normal,
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
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            highlights: HighlightMap::new().build(),
            pre_select_sels: None,
            jump_list: crate::core::jump_list::JumpList::new(),
            engine_view,
            pane_id,
            buffer_id,
            bracket_hl_data: Arc::new(RwLock::new(Vec::new())),
            search_hl_data: Arc::new(RwLock::new(Vec::new())),
            statusline_data,
        }
    }

    pub(crate) fn with_mode(mut self, mode: Mode) -> Self { self.mode = mode; self }
    pub(crate) fn with_file_path(mut self, path: PathBuf) -> Self { self.file_path = Some(path); self }
    pub(crate) fn with_statusline_config(mut self, config: StatusLineConfig) -> Self {
        self.statusline_config = config;
        self
    }
    pub(crate) fn with_minibuf(mut self, prompt: char, input: &str) -> Self {
        let cursor = input.len(); // cursor at end of input, which is the default state
        self.minibuf = Some(MiniBuffer { prompt, input: input.to_string(), cursor });
        self
    }
    pub(crate) fn with_search_regex(mut self, pattern: &str) -> Self {
        self.search.set_regex(regex_cursor::engines::meta::Regex::new(pattern).ok());
        self.update_search_cache();
        self
    }
}

#[cfg(test)]
mod tests;
