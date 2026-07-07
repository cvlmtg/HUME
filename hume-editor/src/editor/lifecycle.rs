use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use hume_engine::pane::{Pane, WhitespaceConfig, WrapMode};
use hume_engine::pipeline::{BufferId, EngineView, PaneId, PaneRenderSettings, RenderContext};
use hume_engine::types::EditorMode;

use super::search::SearchCursor;
#[cfg(test)]
use super::search::{SearchMatches, SearchPattern};
use crate::editor::search;
use crate::ops::pair::find_bracket_pair;
use hume_editing::lines::line_end_exclusive;
use hume_platform::terminal::Term;

use super::{Editor, Mode};

/// Project a `SelectionSet` into an engine pane's head-sorted selection mirror.
///
/// `SelectionSet` stores selections in `start()` order; the engine asserts they
/// are sorted by `head` (see `populate_sorted_sels`).  The two orderings differ
/// whenever a selection is backward (`anchor > head`).  `primary_idx` is
/// re-located after the sort by matching the primary's unique head value.
pub(super) fn write_pane_mirror(
    pane: &mut hume_engine::pane::Pane,
    sels: &hume_editing::selection::SelectionSet,
) {
    use hume_engine::types::Selection as EngineSelection;
    let primary_head = sels.primary().head();
    pane.selections.clear();
    pane.selections.extend(
        sels.iter_head_sorted()
            .into_iter()
            .map(|s| EngineSelection {
                anchor: s.anchor(),
                head: s.head(),
            }),
    );
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
        use super::clipboard;
        use super::keymap::Keymap;
        use super::message_log::MessageLog;
        use super::registry::CommandRegistry;
        use crate::editor::buffer::Buffer;
        use crate::editor::buffer::store::BufferStore;
        use crate::editor::pane_state::{PaneBufferState, PaneTransient, PaneView};
        use crate::ops::register::{KillRing, RegisterSet};
        use crate::settings::EditorSettings;
        use crate::ui::build_pane;
        use hume_editing::selection::{Selection, SelectionSet};
        use hume_editing::text::Text;
        use hume_engine::pipeline::{LayoutTree, SharedBuffer};
        use slotmap::SecondaryMap;
        use std::collections::VecDeque;

        let startup_cwd = std::env::current_dir().unwrap_or_default();
        let mut doc = match file_path {
            Some(ref path) => Buffer::from_file(path)?,
            None => Buffer::new(Text::empty(), SelectionSet::single(Selection::collapsed(0))),
        };
        // Record the user-typed path (symlinks unresolved) for the FilePath statusline
        // element. Buffer::from_file only stores the canonicalized path; we capture the
        // original here before it is discarded.
        if let Some(ref path) = file_path {
            doc.set_display_path(Some(hume_platform::path::absolute_unresolved(
                path,
                &startup_cwd,
            )));
        }

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);

        // Shared completion-popup data, written once per frame and read by
        // every pane's providers (see `build_pane`). Highlight data is per-pane
        // (see `PaneHighlights`) — allocated fresh inside `build_pane`.
        let completion_view: Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>> =
            Arc::new(RwLock::new(None));

        // Insert a buffer — just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());

        let settings = EditorSettings::default();

        // Build the initial pane. Every later split-created pane goes through
        // the same `build_pane` (see `commands::open_pane`).
        let (pane, highlights) = build_pane(
            &mut engine_view.registry,
            &completion_view,
            settings.wrap_mode,
            buffer_id,
        );
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let jump_list_capacity = settings.jump_list_capacity;
        let history_capacity = settings.history_capacity;

        // Seed per-pane state from the buffer's history-root selections.
        let mut per_pane_bufs: SecondaryMap<BufferId, PaneBufferState> = SecondaryMap::new();
        per_pane_bufs.insert(buffer_id, crate::editor::pane_state::fresh_from_buf(&doc));
        let mut pane_buf_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_buf_state.insert(pane_id, per_pane_bufs);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        Ok(Self {
            state: super::EditorState {
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
                status_msg: None,
                summary_ttl: 0,
                message_log: MessageLog::new(),
                settings,
                registry: CommandRegistry::with_defaults(),
                keymap: Keymap::default(),
                last_find: None,
                force_full_redraw: false,
                dispatch_inline_output: false,
                last_repeatable_action: None,
                selection_recipe: Vec::new(),
                insert_session: None,
                autoindent_pending: false,
                explicit_count: false,
                pending_ctrl_extend: false,
                search: super::search::SearchState::default(),
                panes: {
                    let mut jumps = SecondaryMap::new();
                    jumps.insert(pane_id, super::jump_list::JumpList::new(jump_list_capacity));
                    let mut transient = SecondaryMap::new();
                    transient.insert(pane_id, PaneTransient::default());
                    let mut pane_highlights = SecondaryMap::new();
                    pane_highlights.insert(pane_id, highlights);
                    PaneView {
                        state: pane_buf_state,
                        transient,
                        jumps,
                        highlights: pane_highlights,
                    }
                },
                history: super::minibuf::history::HistoryStore::new(history_capacity),
                focused_pane_id: pane_id,
                motion_format_scratch: hume_engine::format::FormatScratch::new(),
                visual_move_target_cols: Vec::new(),
                macro_recording: None,
                macro_pending: None,
                replay_queue: VecDeque::new(),
                pending_repeat: None,
                skip_macro_record: false,
                is_replaying: false,
                mouse_drag_anchor: None,
                languages: hume_treesitter::registry::LanguageRegistry::new(),
                cwd: std::env::current_dir().unwrap_or_default(),
                pending_hooks: Vec::new(),
                pending_steel_calls: Vec::new(),
                trigger_chars: std::collections::HashMap::new(),
                decorations: super::decorations::DecorationStores::default(),
                steel_prompt_callback: None,
                lsp_completion: None,
                completion_view,
            },
            view: engine_view,
            kitty_enabled: false,
            scripting: None,
            builtin_cmd_names: std::collections::HashSet::new(),
            parse_worker: Box::new(hume_treesitter::parse_worker::ThreadedParseBackend::new()),
            parse_worker_disconnect_logged: false,
            timer_wheel: super::timers::TimerWheel::new(),
            timer_payloads: std::collections::HashMap::new(),
            viewport_debounce: std::collections::HashMap::new(),
            last_viewport_key: std::collections::HashMap::new(),
            lsp: super::lsp::LspState::new_threaded(),
            tui_active: false,
            #[cfg(test)]
            inline_output_entered: false,
        })
    }

    /// Process one key event — dispatch it, sync the search cache, drain any
    /// macro replay, sync again.
    ///
    /// This is the single, non-test path for feeding one keystroke to the editor
    /// from outside the interactive event loop (e.g. headless key-runner).  The
    /// interactive loop handles hook draining itself via [`handle_event`]; here
    /// we use [`handle_key`] directly so the caller doesn't need a scripting host.
    pub(crate) fn step(&mut self, key: crossterm::event::KeyEvent) {
        self.handle_key(key);
        self.sync_search_cache();
        self.drain_replay_queue();
        self.sync_search_cache();
    }

    /// Single interactive input boundary: dispatch one terminal event and drain hooks.
    ///
    /// All interactive input flows through here — key events and mouse events alike.
    /// Hooks enqueued during dispatch (mode changes, `:write`, buffer open/close,
    /// language set) fire once at the tail, never mid-event. New input paths must
    /// route through this method so they cannot accidentally skip the drain.
    pub(crate) fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) => self.handle_key(k),
            Event::Mouse(m) => self.handle_mouse(m),
            _ => {}
        }
        self.drain_hooks();
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Prepare the frame: sync all editor state to the engine pane.
    /// 2. Render.
    /// 3. Block until the next terminal event.
    /// 4. Dispatch the event.
    pub(crate) fn run(&mut self, term: &mut Term) -> io::Result<()> {
        // Marks that this Editor now owns the terminal — dispatch's
        // inline-output bracket (mod.rs) checks this to skip alt-screen
        // toggling and the "press any key" block outside the event loop.
        self.tui_active = true;
        // Render context lives here — allocated once, reused every frame.
        // It must be outside `self` so `render_into` can borrow `self`
        // immutably while `ctx` is borrowed mutably alongside it.
        let mut ctx = RenderContext::new();
        let mut last_cursor_color_mode: Option<EditorMode> = None;
        loop {
            // An inline-output command toggled the alt-screen, invalidating ratatui's
            // diff cache; force a full repaint so the editor chrome is restored cleanly.
            if std::mem::take(&mut self.state.force_full_redraw) {
                let _ = term.clear();
            }

            // ── 1. Prepare frame (single sync point) ─────────────────────────
            let size = term.size()?;
            self.prepare_frame(size.width, size.height, &mut ctx);

            // ── 2. Render ─────────────────────────────────────────────────────
            // Compute terminal cursor position before the draw closure to avoid
            // split-borrow conflicts: pane borrows and rope borrows must end
            // before `&mut self.view` is captured by the closure.
            let cursor_screen = if let Some(mb) = &self.state.minibuf {
                // Minibuf active (Command / Search): place the terminal cursor
                // in the statusline at the minibuf edit position.
                let statusline_row = size.height.saturating_sub(1);
                Some((mb.statusline_cursor_col(), statusline_row))
            } else if self.state.mode().cursor_is_bar() {
                // Insert / Select: place the terminal cursor at the document head.
                let cursor_char = self.state.panes.state[self.state.focused_pane_id]
                    [self.focused_buffer_id()]
                .selections
                .primary()
                .head();
                let (pane_settings_cursor, gutter_w) =
                    self.resolve_pane_settings(self.state.focused_pane_id);
                let vp = self.view.panes[self.state.focused_pane_id].viewport.clone();
                // `prepare_frame` ran earlier this iteration and stored the
                // terminal area; recompute the focused pane's origin from it
                // so the bar cursor lands inside the pane, not at the
                // origin. The focused pane is always a live layout leaf (see
                // `close_focused_pane`/`split_pane_onto`), so this can't miss.
                let (ox, oy) = self
                    .view
                    .pane_rect(self.state.focused_pane_id)
                    .map(|r| (r.x, r.y))
                    .expect("focused pane must have a rect after prepare_frame");
                super::cursor::screen_pos(
                    &vp,
                    self.doc().text().rope(),
                    cursor_char,
                    &pane_settings_cursor.wrap_mode,
                    pane_settings_cursor.tab_width,
                    &pane_settings_cursor.whitespace,
                    &mut ctx,
                )
                .map(|(col, row)| (col + gutter_w + ox, row + oy))
            } else {
                None
            };

            // Open the synchronized-output envelope so the terminal defers
            // display until after every byte of this frame has been written.
            // Terminals that don't support DEC 2026 silently ignore the
            // sequence — hence `let _ =` rather than `?`.
            let _ = hume_platform::terminal::begin_synchronized_update();
            term.draw(|frame| {
                self.render_into(frame.area(), frame.buffer_mut(), &mut ctx);
                if let Some((col, row)) = cursor_screen {
                    frame.set_cursor_position((col, row));
                }
            })?;

            // ── 2b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let _ = hume_platform::terminal::set_cursor_shape(self.state.mode().cursor_is_bar());
            if last_cursor_color_mode != Some(self.state.mode()) {
                // Command/Search place the cursor on a white statusline background;
                // use black so it remains visible. All other modes reset to default.
                let black = matches!(self.state.mode(), EditorMode::Command | EditorMode::Search);
                let _ = hume_platform::terminal::set_cursor_color(black);
                last_cursor_color_mode = Some(self.state.mode());
            }
            // Close the synchronized-output envelope: the terminal now atomically
            // paints the complete frame — clear + cells + cursor shape in one shot.
            let _ = hume_platform::terminal::end_synchronized_update();

            // ── 3. Event ──────────────────────────────────────────────────────
            // While any async source (parse worker now; LSP/timers later) has
            // pending work, poll with a bounded timeout so it's drained and
            // painted without waiting for input. Idle falls through to the
            // blocking read so we never burn CPU while the editor is at rest.
            if let Some(timeout) = self.wake_timeout()
                && !event::poll(timeout)?
            {
                continue;
            }
            match event::read()? {
                // Release events arrive only with kitty keyboard protocol
                // (REPORT_EVENT_TYPES flag). Ignore them — we act on Press and
                // Repeat (held key). Without kitty all events are Press anyway.
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_event(Event::Key(key));
                    self.sync_search_cache();
                }
                Event::Key(_) => {}
                Event::Mouse(mouse) => {
                    self.handle_event(Event::Mouse(mouse));
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
                                self.handle_event(Event::Key(key));
                                self.sync_search_cache();
                                break;
                            }
                            Event::Key(_) => break,
                            Event::Mouse(mouse) => {
                                self.handle_event(Event::Mouse(mouse));
                                break;
                            }
                            _ => break,
                        }
                    }
                }
                _ => {}
            }

            if self.state.should_quit {
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
            if self.state.should_quit {
                break;
            }
        }
        // Give every running LSP server a chance to exit cleanly (shutdown
        // request, then exit notification) before the process ends —
        // ServerHandle::drop would otherwise SIGKILL them.
        self.lsp_shutdown_all(Duration::from_millis(500));
        // Restore the user's default cursor shape and colour before returning to the shell.
        hume_platform::terminal::reset_cursor_shape()?;
        let _ = hume_platform::terminal::set_cursor_color(false); // emits reset sequence
        Ok(())
    }

    /// Resolve any pane's render settings and gutter width.
    ///
    /// Returns `(PaneRenderSettings, gutter_w)`. Single source of truth for
    /// wrap_mode / tab_width / whitespace settings across all render paths.
    /// `tab_width` / `whitespace` resolve from that pane's buffer overrides
    /// (document facts); `wrap_mode` resolves from the pane itself — its SSOT
    /// is `Pane::wrap_mode`, not the buffer. `mode` is a per-focus fact: only
    /// the focused pane owns the real terminal cursor, so it alone gets the
    /// live editor mode; other panes are forced to a block-cursor mode so
    /// their fake cursor stays visible instead of turning transparent.
    fn resolve_pane_settings(&self, pid: PaneId) -> (PaneRenderSettings, u16) {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        let len_lines = doc.text().len_lines();
        let gutter_w = super::cursor::gutter_width(pane.providers.gutter_columns(), len_lines);
        let wrap_mode = pane.wrap_mode.resolve(pane.content_width(len_lines));
        let tab_width = doc.overrides.tab_width(&self.state.settings);
        let whitespace = doc.overrides.whitespace(&self.state.settings);
        let mode = if pid == self.state.focused_pane_id {
            self.state.mode()
        } else {
            EditorMode::Normal
        };
        (
            PaneRenderSettings {
                mode,
                wrap_mode,
                tab_width,
                whitespace,
            },
            gutter_w,
        )
    }

    /// Render one frame into `buf`. Single home for the rope / syntax /
    /// pane-settings closures shared by the event loop and `render_to_buf`.
    fn render_into(
        &self,
        area: ratatui::layout::Rect,
        buf: &mut ratatui::buffer::Buffer,
        ctx: &mut RenderContext,
    ) {
        // The statusline provider borrows `self` immutably; it's built here so
        // its lifetime is tied to this call, not stored across the draw closure.
        let statusline = crate::ui::statusline::HumeStatusline { editor: self };
        self.view.render(
            area,
            buf,
            |bid| self.state.buffers.try_get(bid).map(|b| b.text().rope()),
            |pid| self.resolve_pane_settings(pid).0,
            &statusline,
            self.state.focused_pane_id,
            self.state.settings.pane_dividers,
            ctx,
        );
    }

    /// Render the current frame into a ratatui `Buffer` without a live terminal.
    ///
    /// Calls `prepare_frame` so pane mirrors are synced and parse trees are up
    /// to date before rendering.  Used by snapshot tests to lock down styled
    /// output without a live terminal.
    #[cfg(test)]
    pub(crate) fn render_to_buf(&mut self, rect: ratatui::layout::Rect) -> ratatui::buffer::Buffer {
        let mut buf = ratatui::buffer::Buffer::empty(rect);
        let mut ctx = RenderContext::new();
        self.prepare_frame(rect.width, rect.height, &mut ctx);
        self.render_into(rect, &mut buf, &mut ctx);
        buf
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
        // Re-bake the theme if any scope was interned since the last bake —
        // the single per-frame chokepoint that makes forgetting to bake after
        // an `intern`/`intern_runtime` call harmless (see `bake_if_stale`).
        self.view.theme.bake_if_stale(&self.view.registry);

        // Shared rect list every per-pane step below drives off — partitioned
        // through the same `EngineView::pane_area` that `render` uses, so
        // viewport dims and drawn rects never disagree even when a tab bar is
        // present.
        let terminal_area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: terminal_width,
            height: terminal_height,
        };
        let pane_area = self.view.pane_area(terminal_area);
        let reserve_seam = self.state.settings.pane_dividers;
        let mut rects = Vec::new();
        self.view
            .layout
            .collect_rects_into(pane_area, reserve_seam, &mut rects);

        // 1. Sync viewport dimensions for every pane.
        for &(pid, rect) in &rects {
            let vp = &mut self.view.panes[pid].viewport;
            vp.width = rect.width;
            vp.height = rect.height;
        }

        // 2. Sync selection mirrors for every pane.
        self.sync_all_pane_mirrors();

        // 3. Sync line-number style provider for every pane (depends on that
        //    pane's own buffer overrides).
        for &(pid, _) in &rects {
            let buf_id = self.view.panes[pid].buffer_id;
            let ln_style = self
                .state
                .buffers
                .get(buf_id)
                .overrides
                .line_number_style(&self.state.settings);
            self.view.panes[pid]
                .providers
                .sync_line_number_style(ln_style);
        }

        // 4. Scroll every pane so its primary cursor stays visible.
        let scrolloff = self.state.settings.scrolloff;
        for &(pid, _) in &rects {
            let buf_id = self.view.panes[pid].buffer_id;
            let doc = self.state.buffers.get(buf_id);
            let tab_width = doc.overrides.tab_width(&self.state.settings);
            let whitespace = doc.overrides.whitespace(&self.state.settings);
            let len_lines = doc.text().len_lines();
            let rope = doc.text().rope();
            let cursor_char = self.state.panes.state[pid][buf_id]
                .selections
                .primary()
                .head();
            let wrap_mode = {
                let pane = &self.view.panes[pid];
                pane.wrap_mode.resolve(pane.content_width(len_lines))
            };
            let pane = &mut self.view.panes[pid];
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

            // B7: a real visible-range change (scroll command, cursor-follow
            // during typing, or a resize that altered height) debounces
            // OnViewportChange. This is bookkeeping over scroll_into_view's
            // *result*, not part of computing what to render — the hook
            // itself never fires from here, only the coalescer timer gets
            // (re)armed; the actual fire happens later via the async-source
            // drain, same as every other timer.
            let viewport = &self.view.panes[pid].viewport;
            let key = (viewport.top_line, viewport.height);
            if self.last_viewport_key.insert(pid, key) != Some(key) {
                self.debounce_viewport_change(pid);
            }
        }

        // 5. Drain completed async work (parse results, LSP), then evaluate
        //    any Steel calls that work queued (LSP request/timer callbacks).
        self.drain_async_sources();
        self.drain_pending_steel_calls();

        // 6. Sync highlight data (search matches, bracket matches) to shared
        //    Arc buffers read by the highlight providers during rendering.
        self.update_highlight_providers();

        // 7. Sync completion-popup view to the shared Arc for `CompletionOverlay`.
        self.sync_completion_view();

        // 8. Store the terminal area + divider setting: pane-focus/split
        //    commands have no terminal handle between frames, so they
        //    recompute geometry from these via `EngineView::pane_rects`/
        //    `pane_rect` rather than trusting a stored rect list.
        self.view.last_pane_area = pane_area;
        self.view.reserve_seam = reserve_seam;
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
        let state = &mut self.state;
        let view = &mut self.view;
        for (pid, pane) in view.panes.iter_mut() {
            if let Some(pbs) = state
                .panes
                .state
                .get(pid)
                .and_then(|m| m.get(pane.buffer_id))
            {
                write_pane_mirror(pane, &pbs.selections);
            }
        }
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    #[cfg(test)]
    pub(crate) fn viewport(&self) -> &hume_engine::pane::ViewportState {
        &self.view.panes[self.state.focused_pane_id].viewport
    }

    /// `true` if dispatch has ever entered the inline-output terminal
    /// bracket (alt-screen toggle + "press any key") on this `Editor`. Off
    /// the event loop this must stay `false` for every `#:inline-output #t`
    /// command dispatched — see `tui_active` on `Editor`.
    #[cfg(test)]
    pub(crate) fn inline_output_entered(&self) -> bool {
        self.inline_output_entered
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut hume_engine::pane::ViewportState {
        &mut self.view.panes[self.state.focused_pane_id].viewport
    }

    // ── Search accessors ──────────────────────────────────────────────────────

    /// Accessor for the focused buffer's active search pattern (used in tests).
    #[cfg(test)]
    pub(crate) fn search_pattern(&self) -> Option<&SearchPattern> {
        self.state
            .buffers
            .get(self.focused_buffer_id())
            .search_pattern
            .as_ref()
    }

    /// Accessor for the focused buffer's match cache.
    #[cfg(test)]
    pub(crate) fn search_matches(&self) -> &SearchMatches {
        &self
            .state
            .buffers
            .get(self.focused_buffer_id())
            .search_matches
    }

    /// Accessor for the focused pane's search cursor (match count, wrapped flag).
    pub(crate) fn current_search_cursor(&self) -> &SearchCursor {
        &self.state.panes.state[self.state.focused_pane_id][self.focused_buffer_id()].search_cursor
    }

    /// Recompute the match list and pane search cursor for the focused buffer,
    /// if stale. No-op when no search is active.
    pub(super) fn sync_search_cache(&mut self) {
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        search::ops::sync_search_cache(
            &mut self.state.buffers,
            &mut self.state.panes.state,
            pid,
            bid,
        );
    }

    /// Write per-frame highlight data to every pane's own `Arc<RwLock<...>>`
    /// buffers, read by that pane's `SharedHighlighter` providers.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode. Each pane's search
    /// highlights are computed from **that pane's own buffer and viewport** —
    /// panes never share highlight data (see [`crate::ui::highlight_providers::PaneHighlights`]),
    /// so a pane viewing a different buffer, or the same buffer scrolled
    /// elsewhere, never inherits another pane's matches.
    pub(super) fn update_highlight_providers(&mut self) {
        let in_insert = self.state.mode() == EditorMode::Insert;

        // Snapshot (pane, buffer) pairs up front: the loop body mutates
        // `self.state.buffers` (refreshing the search-match cache), which would
        // otherwise conflict with an active borrow of `self.view.panes`.
        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        // ── Search match highlights — one pane at a time ─────────────────────
        for &(pid, bid) in &panes {
            // Clone the Arc (not the data) so the write lock and the buffer
            // refresh below don't hold a borrow of `self.state.panes`.
            let Some(search_arc) = self
                .state
                .panes
                .highlights
                .get(pid)
                .map(|h| Arc::clone(&h.search))
            else {
                continue;
            };
            let mut data = search_arc.write().expect("RwLock not poisoned");
            data.clear();
            // Hidden in Insert mode — matches aren't actionable while typing and
            // clutter the view. Same pattern as bracket match highlights below.
            if in_insert {
                continue;
            }

            // Keep this buffer's match cache current regardless of focus — a
            // non-focused pane's buffer may carry its own active search
            // pattern that the focused-pane-only `sync_search_cache` never
            // refreshes. No-op when the cache already matches this revision.
            search::ops::update_buffer_matches(&mut self.state.buffers, bid);

            let buf = self.state.buffers.get(bid);
            let text = buf.text();
            let vp = &self.view.panes[pid].viewport;
            let top_line = vp.top_line;
            let bot_line = top_line + vp.height as usize;

            // Matches are sorted by document order. Binary-search to the first
            // match that starts at or after this pane's `top_line`.
            let top_char = text.line_to_char(top_line.min(text.len_lines().saturating_sub(1)));
            let matches = &buf.search_matches.matches;
            let first = matches.partition_point(|&(start, _)| start < top_char);
            for &(start, end_incl) in &matches[first..] {
                let start_line = text.char_to_line(start);
                if start_line > bot_line {
                    break;
                }
                // end_incl is inclusive char offset; +1 makes it exclusive.
                let end_char = (end_incl + 1).min(text.len_chars());
                push_match_highlight_lines(text, start, end_char, &mut data);
            }
        }

        // ── Bracket match highlight — cursor concept, focused pane only ──────
        // Clear every pane first: a bracket match lingers only on whichever
        // pane last had focus, so moving focus away must blank the old one.
        for &(pid, _) in &panes {
            if let Some(h) = self.state.panes.highlights.get(pid) {
                h.bracket.write().expect("RwLock not poisoned").clear();
            }
        }
        if !in_insert {
            let focused = self.state.focused_pane_id;
            if let Some(bracket_arc) = self
                .state
                .panes
                .highlights
                .get(focused)
                .map(|h| Arc::clone(&h.bracket))
            {
                let buf = self.doc().text();
                let head = self.state.panes.state[focused][self.focused_buffer_id()]
                    .selections
                    .primary()
                    .head();
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
                        bracket_arc.write().expect("RwLock not poisoned").push((
                            line,
                            byte,
                            byte + ch_len,
                        ));
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
        if self.state.completion.is_none()
            && self
                .state
                .completion_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }
        use unicode_width::UnicodeWidthStr as _;
        let view = self.state.completion.as_ref().map(|state| {
            let anchor_col = self
                .state
                .minibuf
                .as_ref()
                .map(|mb| {
                    let pad: u16 = 1;
                    let prompt_w = mb.prompt.width() as u16;
                    let safe_end = state.span_start.min(mb.input.len());
                    let token_col = mb.input[..safe_end].width() as u16;
                    pad + prompt_w + token_col
                })
                .unwrap_or(0);
            crate::ui::completion_overlay::CompletionView {
                rows: state.candidates.iter().map(|c| c.display.clone()).collect(),
                selected: state.selected,
                anchor_col,
                border: self.state.settings.popup_border,
            }
        });
        *self
            .state
            .completion_view
            .write()
            .expect("RwLock not poisoned") = view;
    }

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// Enqueues `OnModeChange` through the unified `pending_hooks` channel
    /// (same path as the `EditorCmd` handlers); `drain_hooks` fires it after
    /// the current dispatch completes.
    ///
    /// For Insert mode entry and exit use [`begin_insert_session`] and
    /// [`end_insert_session`] instead — they manage the undo group and
    /// dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: EditorMode) {
        self.state.set_mode(mode);
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
    scratch: &mut hume_engine::format::FormatScratch,
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

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` where `byte_in_line` is the byte offset
/// from the start of the line — suitable for building highlight spans that the
/// engine expects in line-relative byte coordinates.
pub(super) fn char_to_line_byte(buf: &hume_editing::text::Text, char_pos: usize) -> (usize, usize) {
    let line = buf.char_to_line(char_pos);
    let line_start_byte = buf.char_to_byte(buf.line_to_char(line));
    let byte = buf.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}

/// Push one `(line, byte_start, byte_end)` triple per line the
/// `[start, end_char_excl)` char range touches.
///
/// A single-line match produces one triple, byte-identical to converting
/// `start`/`end_char_excl` directly with [`char_to_line_byte`]. A match that
/// crosses one or more `\n`s produces one triple per touched line, each
/// clipped to that line's own content (up to but excluding its trailing
/// `\n`). The clip point is deliberately the `\n` char's own position, not
/// `line_end_exclusive` — the latter is the *next* line's start, and
/// converting it with `char_to_line_byte` would resolve to the wrong line
/// (byte 0 of the line after), producing an inverted or nonsensical span.
fn push_match_highlight_lines(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
    data: &mut Vec<(usize, usize, usize)>,
) {
    if start >= end_char_excl {
        return;
    }
    let last_char = end_char_excl - 1;
    let start_line = buf.char_to_line(start);
    let end_line = buf.char_to_line(last_char);

    for line in start_line..=end_line {
        // Every content line ends with a '\n' — HUME buffers always end with
        // a structural trailing '\n', so this position always exists and
        // still belongs to `line` in ropey's line model.
        let line_newline = line_end_exclusive(buf, line) - 1;
        let seg_start = start.max(buf.line_to_char(line));
        let seg_end = end_char_excl.min(line_newline);
        let (_, byte_start) = char_to_line_byte(buf, seg_start);
        let (_, byte_end) = char_to_line_byte(buf, seg_end);
        data.push((line, byte_start, byte_end));
    }
}
