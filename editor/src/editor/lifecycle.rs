use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use engine::pane::{Pane, WrapMode, WhitespaceConfig};
use engine::pipeline::{
    BufferId, EngineView, PaneId, PaneRenderSettings, RenderContext,
};
use engine::types::EditorMode;

use crate::core::search_state::SearchCursor;
use crate::core::search_state::SearchPattern;
#[cfg(test)]
use crate::core::search_state::SearchMatches;
use crate::editor::search_ops;
use crate::ops::pair::find_bracket_pair;
use crate::os::terminal::Term;
use crate::scripting::hooks::HookId;
use steel::rvals::IntoSteelVal as _;

use super::{Editor, Mode};

/// Project a `SelectionSet` into an engine pane's head-sorted selection mirror.
///
/// `SelectionSet` stores selections in `start()` order; the engine asserts they
/// are sorted by `head` (see `populate_sorted_sels`).  The two orderings differ
/// whenever a selection is backward (`anchor > head`).  `primary_idx` is
/// re-located after the sort by matching the primary's unique head value.
pub(super) fn write_pane_mirror(
    pane: &mut engine::pane::Pane,
    sels: &crate::core::selection::SelectionSet,
) {
    use engine::types::Selection as EngineSelection;
    let primary_head = sels.primary().head;
    pane.selections.clear();
    pane.selections
        .extend(sels.iter_head_sorted().map(|s| EngineSelection {
            anchor: s.anchor,
            head: s.head,
        }));
    pane.primary_idx = pane
        .selections
        .iter()
        .position(|s| s.head == primary_head)
        .unwrap_or(0);
}

impl Editor {
    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0 in Normal mode. Terminal dimensions are
    /// placeholder values replaced on the first event-loop iteration.
    pub(crate) fn open(file_path: Option<std::path::PathBuf>) -> io::Result<Self> {
        use std::collections::VecDeque;
        use slotmap::SecondaryMap;
        use engine::builtins::line_number::{LineNumberColumn, LineNumberStyle as EngineLineNumberStyle};
        use engine::pipeline::{LayoutTree, SharedBuffer};
        use crate::core::selection::{Selection, SelectionSet};
        use crate::core::text::Text;
        use crate::editor::buffer::Buffer;
        use crate::editor::buffer_store::BufferStore;
        use crate::editor::pane_state::{PaneBufferState, PaneTransient};
        use crate::ops::register::{KillRing, RegisterSet};
        use crate::settings::EditorSettings;
        use super::registry::CommandRegistry;
        use super::keymap::Keymap;
        use super::message_log::MessageLog;
        use super::clipboard;

        let doc = match file_path {
            Some(ref path) => Buffer::from_file(path)?,
            None => Buffer::new(Text::empty(), SelectionSet::single(Selection::collapsed(0))),
        };

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);

        // Intern highlight scopes before registering providers.
        let bracket_scope = engine_view.registry.intern("ui.cursor.match");
        let search_scope = engine_view.registry.intern("ui.selection.search");

        // Register the shared highlight data arcs.
        let bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>> =
            Arc::new(RwLock::new(Vec::new()));
        let search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>> =
            Arc::new(RwLock::new(Vec::new()));
        let completion_view: Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>> =
            Arc::new(RwLock::new(None));

        // Insert a buffer — just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());

        // Build the initial pane.
        let mut providers = engine::providers::ProviderSet::new();
        providers.add_gutter_column(Box::new(LineNumberColumn::with_style(
            EngineLineNumberStyle::Hybrid,
        )));
        providers.add_highlight_source(Box::new(
            crate::ui::highlight_providers::SharedHighlighter {
                scope: bracket_scope,
                tier: engine::providers::HighlightTier::BracketMatch,
                data: Arc::clone(&bracket_hl_data),
            },
        ));
        providers.add_highlight_source(Box::new(
            crate::ui::highlight_providers::SharedHighlighter {
                scope: search_scope,
                tier: engine::providers::HighlightTier::SearchMatch,
                data: Arc::clone(&search_hl_data),
            },
        ));
        providers.add_overlay(Box::new(crate::ui::completion_overlay::CompletionOverlay {
            data: Arc::clone(&completion_view),
        }));

        let settings = EditorSettings::default();

        let pane = engine::pane::Pane {
            providers,
            ..engine::pane::Pane::new(buffer_id)
        };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let jump_list_capacity = settings.jump_list_capacity;
        let history_capacity = settings.history_capacity;

        // Seed per-pane state from the buffer's history-root selections.
        let mut per_pane_bufs: SecondaryMap<BufferId, PaneBufferState> = SecondaryMap::new();
        per_pane_bufs.insert(buffer_id, crate::editor::pane_state::fresh_from_buf(&doc));
        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_state.insert(pane_id, per_pane_bufs);
        let mut pane_transient: SecondaryMap<PaneId, PaneTransient> = SecondaryMap::new();
        pane_transient.insert(pane_id, PaneTransient::default());

        // Bake theme now that all scopes are interned.
        engine_view.theme.bake(&engine_view.registry);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        Ok(Self {
            buffers,
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            kill_ring: KillRing::new(),
            clipboard: clipboard::SystemClipboard::new(),
            register_prefix: None,
            last_command: None,
            last_paste: None,
            should_quit: false,
            minibuf: None,
            completion: None,
            completion_view,
            status_msg: None,
            message_log: MessageLog::new(),
            settings,
            registry: CommandRegistry::with_defaults(),
            keymap: Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            force_full_redraw: false,
            last_repeatable_action: None,
            insert_session: None,
            explicit_count: false,
            search: crate::core::search_state::SearchState::default(),
            pane_jumps: {
                let mut m = SecondaryMap::new();
                m.insert(
                    pane_id,
                    crate::core::jump_list::JumpList::new(jump_list_capacity),
                );
                m
            },
            history: crate::core::minibuf_history::HistoryStore::new(history_capacity),
            pane_state,
            pane_transient,
            engine_view,
            focused_pane_id: pane_id,
            bracket_hl_data,
            search_hl_data,
            motion_format_scratch: engine::format::FormatScratch::new(),
            visual_move_target_cols: Vec::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
            scripting: None,
            builtin_cmd_names: std::collections::HashSet::new(),
            languages: crate::editor::syntax::LanguageRegistry::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
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
            // An inline-output command toggled the alt-screen, invalidating ratatui's
            // diff cache; force a full repaint so the editor chrome is restored cleanly.
            if std::mem::take(&mut self.force_full_redraw) {
                let _ = term.clear();
            }

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
                let cursor_char = self.pane_state[self.focused_pane_id][self.focused_buffer_id()]
                    .selections
                    .primary()
                    .head;
                let (vp, gutter_w) = {
                    let pane = &self.engine_view.panes[self.focused_pane_id];
                    let gw = crate::cursor::gutter_width(
                        pane.providers.gutter_columns(),
                        self.doc().text().len_lines(),
                    );
                    (pane.viewport.clone(), gw)
                };
                let content_width = vp.width.saturating_sub(gutter_w).max(1);
                let wrap_mode = self.doc().overrides.wrap_mode(&self.settings).resolve(content_width);
                let tab_width = self.doc().overrides.tab_width(&self.settings);
                let whitespace = self.doc().overrides.whitespace(&self.settings);
                crate::cursor::screen_pos(
                    &vp,
                    self.doc().text().rope(),
                    cursor_char,
                    &wrap_mode,
                    tab_width,
                    &whitespace,
                    &mut ctx,
                )
                .map(|(col, row)| (col + gutter_w, row))
            } else {
                None
            };

            // The statusline provider borrows `self` — create it before the
            // draw closure so the lifetime is tied to this stack frame.
            let statusline = crate::ui::statusline::HumeStatusline { editor: self };

            // Split borrows: `engine_view` and `doc` are disjoint fields of
            // `self`. Extract the rope and pane settings to render before
            // moving `engine_view` into the draw closure.
            let rope: &ropey::Rope = self.doc().text().rope();
            let buffer_id = self.focused_buffer_id();
            let pane_id = self.focused_pane_id;
            // Resolve mode and display settings once — passed to the engine via
            // closure so the engine never stores editor-domain state on Pane.
            let pane_settings = {
                let (raw_wrap, len_lines) = (
                    self.doc().overrides.wrap_mode(&self.settings),
                    self.doc().text().len_lines(),
                );
                let pane = &self.engine_view.panes[self.focused_pane_id];
                let wrap_mode = raw_wrap.resolve(pane.content_width(len_lines));
                let tab_width = self.doc().overrides.tab_width(&self.settings);
                let whitespace = self.doc().overrides.whitespace(&self.settings);
                PaneRenderSettings {
                    mode: self.mode,
                    wrap_mode,
                    tab_width,
                    whitespace,
                }
            };
            let engine_view = &self.engine_view;
            // Open the synchronized-output envelope so the terminal defers
            // display until after every byte of this frame has been written.
            // Terminals that don't support DEC 2026 silently ignore the
            // sequence — hence `let _ =` rather than `?`.
            let _ = crate::os::terminal::begin_synchronized_update();
            term.draw(|frame| {
                engine_view.render(
                    frame.area(),
                    frame.buffer_mut(),
                    |bid| if bid == buffer_id { Some(rope) } else { None },
                    |pid| {
                        if pid == pane_id {
                            pane_settings.clone()
                        } else {
                            PaneRenderSettings::default()
                        }
                    },
                    Some(&statusline),
                    &mut ctx,
                );
                if let Some((col, row)) = cursor_screen {
                    frame.set_cursor_position((col, row));
                }
            })?;

            // ── 2b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let _ = crate::os::terminal::set_cursor_shape(self.mode);
            if last_cursor_color_mode != Some(self.mode) {
                let _ = crate::os::terminal::set_cursor_color_for_mode(self.mode);
                last_cursor_color_mode = Some(self.mode);
            }
            // Close the synchronized-output envelope: the terminal now atomically
            // paints the complete frame — clear + cells + cursor shape in one shot.
            let _ = crate::os::terminal::end_synchronized_update();

            // ── 3. Event ──────────────────────────────────────────────────────
            match event::read()? {
                // Release events arrive only with kitty keyboard protocol
                // (REPORT_EVENT_TYPES flag). Ignore them — we act on Press and
                // Repeat (held key). Without kitty all events are Press anyway.
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_key(key);
                    self.sync_search_cache();
                }
                Event::Key(_) => {}
                Event::Mouse(mouse) => {
                    self.handle_mouse(mouse);
                }
                Event::Resize(_, _) => {
                    // Drain any additional resize events that are already queued
                    // so a drag (which emits one event per delta) collapses into a
                    // single render on the next iteration. Viewport dimensions are
                    // re-read at loop top, so only the final size matters.
                    // Non-resize events that arrive during the drain are handled
                    // inline so they are never lost.
                    while event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Resize(_, _) => continue,
                            Event::Key(key) if key.kind != KeyEventKind::Release => {
                                self.handle_key(key);
                                self.sync_search_cache();
                                break;
                            }
                            Event::Key(_) => break,
                            Event::Mouse(mouse) => {
                                self.handle_mouse(mouse);
                                break;
                            }
                            _ => break,
                        }
                    }
                }
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
            // `last_repeatable_action` is saved/restored so replay does not corrupt dot-repeat.
            self.drain_replay_queue();
            // One cache update covers the entire replay batch — the search
            // cache only changes when the buffer revision changes, so calling
            // it per-key would redundantly clone the regex on every iteration.
            self.sync_search_cache();
            if self.should_quit {
                break;
            }
        }
        // Restore the user's default cursor shape and colour before returning to the shell.
        crate::os::terminal::reset_cursor_shape()?;
        let _ = crate::os::terminal::set_cursor_color_for_mode(EditorMode::Normal); // emits reset sequence
        Ok(())
    }

    /// Paint one frame immediately — called before `init_scripting` so the
    /// editor chrome is visible during Steel engine init instead of a blank
    /// alt-screen.  Skips the statusline and cursor-position overlay (those
    /// are only live inside the event loop) but renders the full buffer view.
    pub(crate) fn draw_once(&mut self, term: &mut Term) -> io::Result<()> {
        let mut ctx = RenderContext::new();
        let size = term.size()?;
        self.prepare_frame(size.width, size.height, &mut ctx);

        let rope = self.doc().text().rope();
        let buffer_id = self.focused_buffer_id();
        let pane_id = self.focused_pane_id;
        let pane_settings = {
            let len_lines = self.doc().text().len_lines();
            let raw_wrap = self.doc().overrides.wrap_mode(&self.settings);
            let tab_width = self.doc().overrides.tab_width(&self.settings);
            let whitespace = self.doc().overrides.whitespace(&self.settings);
            let pane = &self.engine_view.panes[self.focused_pane_id];
            let wrap_mode = raw_wrap.resolve(pane.content_width(len_lines));
            PaneRenderSettings { mode: self.mode, wrap_mode, tab_width, whitespace }
        };
        let engine_view = &self.engine_view;
        let _ = crate::os::terminal::begin_synchronized_update();
        term.draw(|frame| {
            engine_view.render(
                frame.area(),
                frame.buffer_mut(),
                |bid| if bid == buffer_id { Some(rope) } else { None },
                |pid| {
                    if pid == pane_id {
                        pane_settings.clone()
                    } else {
                        PaneRenderSettings::default()
                    }
                },
                None,
                &mut ctx,
            );
        })?;
        let _ = crate::os::terminal::end_synchronized_update();
        Ok(())
    }

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// `sync_all_pane_mirrors` is the **single sync point** for `pane.selections`
    /// and `pane.primary_idx` — it covers every pane in one pass.  No other code
    /// path writes those fields.  Highlight and statusline shared buffers are also
    /// written here, immediately before every `render()` call.  Mode and display
    /// settings are resolved lazily via the `get_pane_settings` closure passed to
    /// `render()`.
    pub(super) fn prepare_frame(
        &mut self,
        terminal_width: u16,
        terminal_height: u16,
        ctx: &mut RenderContext,
    ) {
        // 1. Sync viewport dimensions.
        // Engine reserves 1 row for the statusline; the pane gets the rest.
        {
            let vp = self.viewport_mut();
            vp.width = terminal_width;
            vp.height = terminal_height.saturating_sub(1);
        }

        // 2. Sync selection mirrors for every pane.
        self.sync_all_pane_mirrors();

        // 3. Sync line-number style provider (depends on buffer overrides).
        {
            let ln_style = self.doc().overrides.line_number_style(&self.settings);
            self.engine_view.panes[self.focused_pane_id]
                .providers
                .sync_line_number_style(ln_style);
        }

        // 4. Scroll so the primary cursor stays visible.
        let cursor_char = self.pane_state[self.focused_pane_id][self.focused_buffer_id()]
            .selections
            .primary()
            .head;
        let scrolloff = self.settings.scrolloff;
        let tab_width = self.doc().overrides.tab_width(&self.settings);
        let whitespace = self.doc().overrides.whitespace(&self.settings);
        {
            let buf_id = self.focused_buffer_id();
            let raw_wrap = self.doc().overrides.wrap_mode(&self.settings);
            let len_lines = self.buffers.get(buf_id).text().len_lines();
            let rope = self.buffers.get(buf_id).text().rope();
            let wrap_mode = {
                let pane = &self.engine_view.panes[self.focused_pane_id];
                raw_wrap.resolve(pane.content_width(len_lines))
            };
            let pane = &mut self.engine_view.panes[self.focused_pane_id];
            scroll_into_view(
                pane,
                rope,
                cursor_char,
                &mut ctx.cursor_format,
                &wrap_mode,
                tab_width,
                &whitespace,
                scrolloff,
            );
        }

        // 5. Reparse any visible buffer whose text changed since the last frame.
        self.reparse_stale_buffers();

        // 6. Sync highlight data (search matches, bracket matches) to shared
        //    Arc buffers read by the highlight providers during rendering.
        self.update_highlight_providers();

        // 7. Sync completion-popup view to the shared Arc for `CompletionOverlay`.
        self.sync_completion_view();
    }

    /// Sync every engine pane's selection mirror from the authoritative `pane_state`.
    ///
    /// The engine requires `pane.selections` sorted by `head` (not by `start()` as
    /// `SelectionSet` stores internally); `primary_idx` is re-located by matching
    /// the primary's head value after the sort.  This is the **single sync point** —
    /// no other code path writes `pane.selections` or `pane.primary_idx`.
    ///
    /// Called once per frame from `prepare_frame`, before `render()`.
    pub(crate) fn sync_all_pane_mirrors(&mut self) {
        let Self {
            pane_state,
            engine_view,
            ..
        } = &mut *self;
        for (pid, pane) in engine_view.panes.iter_mut() {
            if let Some(pbs) = pane_state.get(pid).and_then(|m| m.get(pane.buffer_id)) {
                write_pane_mirror(pane, &pbs.selections);
            }
        }
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    pub(crate) fn viewport(&self) -> &engine::pane::ViewportState {
        &self.engine_view.panes[self.focused_pane_id].viewport
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut engine::pane::ViewportState {
        &mut self.engine_view.panes[self.focused_pane_id].viewport
    }

    // ── Search accessors ──────────────────────────────────────────────────────

    /// Accessor for the focused buffer's active search pattern.
    pub(crate) fn search_pattern(&self) -> Option<&SearchPattern> {
        self.buffers
            .get(self.focused_buffer_id())
            .search_pattern
            .as_ref()
    }

    /// Accessor for the focused buffer's match cache.
    #[cfg(test)]
    pub(crate) fn search_matches(&self) -> &SearchMatches {
        &self.buffers.get(self.focused_buffer_id()).search_matches
    }

    /// Accessor for the focused pane's search cursor (match count, wrapped flag).
    pub(crate) fn current_search_cursor(&self) -> &SearchCursor {
        &self.pane_state[self.focused_pane_id][self.focused_buffer_id()].search_cursor
    }

    /// Mutable accessor for the focused pane's search cursor.
    pub(crate) fn current_search_cursor_mut(&mut self) -> &mut SearchCursor {
        let bid = self.focused_buffer_id();
        &mut self.pane_state[self.focused_pane_id][bid].search_cursor
    }

    /// Recompute the match list and pane search cursor for the focused buffer,
    /// if stale. No-op when no search is active.
    pub(super) fn sync_search_cache(&mut self) {
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        search_ops::sync_search_cache(&mut self.buffers, &mut self.pane_state, pid, bid);
    }

    /// Write per-frame highlight data to the shared `Arc<RwLock<...>>` buffers
    /// read by `BracketMatchHighlighter` and `SearchMatchHighlighter`.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode.
    pub(super) fn update_highlight_providers(&mut self) {
        let buf = self.doc().text();

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
                let matches = &self
                    .buffers
                    .get(self.focused_buffer_id())
                    .search_matches
                    .matches;
                let first = matches.partition_point(|&(start, _)| start < top_char);
                for &(start, end_incl) in &matches[first..] {
                    let (line, byte_start) = char_to_line_byte(buf, start);
                    if line > bot_line {
                        break;
                    }
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
                let head = self.pane_state[self.focused_pane_id][self.focused_buffer_id()]
                    .selections
                    .primary()
                    .head;
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

    /// Write the current completion state into the shared `CompletionView` Arc
    /// so `CompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    pub(super) fn sync_completion_view(&self) {
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if self.completion.is_none()
            && self
                .completion_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }
        use unicode_width::UnicodeWidthChar as _;
        use unicode_width::UnicodeWidthStr as _;
        let view = self.completion.as_ref().map(|state| {
            let anchor_col = self
                .minibuf
                .as_ref()
                .map(|mb| {
                    let pad: u16 = 1;
                    let prompt_w = mb.prompt.width().unwrap_or(1) as u16;
                    let safe_end = state.span_start.min(mb.input.len());
                    let token_col = mb.input[..safe_end].width() as u16;
                    pad + prompt_w + token_col
                })
                .unwrap_or(0);
            crate::ui::completion_overlay::CompletionView {
                rows: state.candidates.iter().map(|c| c.display.clone()).collect(),
                selected: state.selected,
                anchor_col,
                border: self.settings.popup_border,
            }
        });
        *self.completion_view.write().expect("RwLock not poisoned") = view;
    }

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// For Insert mode entry and exit use [`begin_insert_session`] and
    /// [`end_insert_session`] instead — they manage the undo group and
    /// dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: EditorMode) {
        let old = self.mode;
        self.mode = mode;
        if old != mode
            && self
                .scripting
                .as_ref()
                .is_some_and(|h| !h.hooks.is_empty_for(HookId::OnModeChange))
        {
            let old_val = mode_name(old)
                .into_steelval()
                .expect("mode str into_steelval");
            let new_val = mode_name(mode)
                .into_steelval()
                .expect("mode str into_steelval");
            self.fire_hook_silent(HookId::OnModeChange, &[old_val, new_val]);
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Scroll the pane viewport so `cursor_char` stays within the visible area.
///
/// Calls both the vertical and horizontal `ensure_cursor_visible` helpers in
/// one shot. Used by `prepare_frame` for both the scratch-view path and the
/// normal document path.
#[allow(clippy::too_many_arguments)]
pub(super) fn scroll_into_view(
    pane: &mut Pane,
    rope: &ropey::Rope,
    cursor_char: usize,
    scratch: &mut engine::format::FormatScratch,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scrolloff: usize,
) {
    use super::scroll;
    scroll::ensure_cursor_visible(
        &mut pane.viewport,
        rope,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
        scrolloff,
    );
    scroll::ensure_cursor_visible_horizontal(
        &mut pane.viewport,
        rope,
        cursor_char,
        wrap_mode,
        tab_width,
        whitespace,
        scratch,
    );
}

/// Map an `EditorMode` to the Steel-facing string name used in hook arguments.
pub(super) fn mode_name(m: EditorMode) -> &'static str {
    match m {
        EditorMode::Normal => "normal",
        EditorMode::Insert => "insert",
        EditorMode::Extend => "extend",
        EditorMode::Command => "command",
        EditorMode::Search => "search",
        EditorMode::Select => "select",
    }
}

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` where `byte_in_line` is the byte offset
/// from the start of the line — suitable for building highlight spans that the
/// engine expects in line-relative byte coordinates.
pub(super) fn char_to_line_byte(buf: &crate::core::text::Text, char_pos: usize) -> (usize, usize) {
    let line = buf.char_to_line(char_pos);
    let line_start_byte = buf.char_to_byte(buf.line_to_char(line));
    let byte = buf.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}
