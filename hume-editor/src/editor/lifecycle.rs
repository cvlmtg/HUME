use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use termina::event::{Event, KeyEvent, KeyEventKind};

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, PaneId, PaneRenderSettings, RenderContext};
use hume_engine::types::EditorMode;

use super::buffer::Buffer;
use super::search::SearchCursor;
#[cfg(test)]
use super::search::{SearchMatches, SearchPattern};
use crate::editor::lsp::diagnostics::DiagSeverity;
use crate::editor::search;
use crate::ops::pair::find_bracket_pair;
use crate::settings::EditorSettings;
use hume_editing::lines::line_end_exclusive;
use hume_platform::terminal::{SharedTerm, Term};

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
    // ── Kitty keybinds ──────────────────────────────────────────────────────────

    /// Apply the kitty keyboard-protocol probe result atomically: set the
    /// runtime flag and re-derive the keymap via the same
    /// [`super::default_keymap_for`] `ConfigState::new` uses, so the kitty-only
    /// default keybinds (when enabled) are installed identically at startup
    /// and on every `:reload-config`. Called once at startup after the probe
    /// (and from headless `run_keys`, which assumes full capability) so the
    /// binds can never diverge from the flag.
    ///
    /// Must run before `init_scripting`: it replaces the keymap wholesale,
    /// so calling it after `init.scm` has evaluated would discard any user
    /// `bind-key!` on top of it.
    pub(crate) fn set_kitty_support(&mut self, kitty_enabled: bool) {
        self.kitty_enabled = kitty_enabled;
        self.state.config.keymap = super::default_keymap_for(kitty_enabled);
    }

    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0 in Normal mode. Terminal dimensions are
    /// placeholder values replaced on the first event-loop iteration.
    ///
    /// `wake` is the cross-thread waker background threads (LSP transport,
    /// parse worker) call after posting a result, so `run`'s event loop wakes
    /// instead of polling for completion. Works without a terminal too
    /// (headless): the loop simply never enters `run` to wait on it.
    pub(crate) fn open(
        file_path: Option<std::path::PathBuf>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        use super::clipboard;
        use super::message_log::MessageLog;
        use crate::editor::buffer::Buffer;
        use crate::editor::buffer::store::BufferStore;
        use crate::editor::pane_state::{PaneBufferState, PaneTransient, PaneView};
        use crate::ops::register::{KillRing, RegisterSet};
        use crate::settings::EditorSettings;
        use crate::ui::build_pane;
        use hume_editing::selection::{Selection, SelectionSet};
        use hume_editing::text::Text;
        use hume_engine::pipeline::LayoutTree;
        use slotmap::SecondaryMap;
        use std::collections::VecDeque;

        let startup_cwd = std::env::current_dir().unwrap_or_default();
        let mut doc = match file_path {
            Some(ref path) => Buffer::from_file(path)?,
            None => Buffer::new(Text::empty(), SelectionSet::single(Selection::collapsed(0))),
        };
        // Record the user-typed path (symlinks unresolved) for user-facing display,
        // overwriting `Buffer::from_file`'s canonical-derived default.
        if let Some(ref path) = file_path {
            doc.set_display_path(Some(hume_platform::path::display_form(
                &hume_platform::path::absolute_unresolved(path, &startup_cwd),
            )));
        }

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);

        // Shared completion-popup / hover-popup data, written once per frame
        // and read by every pane's providers (see `build_pane`). Highlight
        // data is per-pane (see `PaneHighlights`) — allocated fresh inside
        // `build_pane`.
        let minibuf_completion_view: Arc<
            RwLock<Option<crate::ui::completion_overlay::MinibufCompletionView>>,
        > = Arc::new(RwLock::new(None));
        let popup_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>> =
            Arc::new(RwLock::new(None));
        let menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>> =
            Arc::new(RwLock::new(None));
        let completion_menu_view: Arc<RwLock<Option<crate::ui::popup::PopupState>>> =
            Arc::new(RwLock::new(None));
        let picker_view: Arc<RwLock<Option<crate::ui::picker_panel::PickerViewState>>> =
            Arc::new(RwLock::new(None));
        // The drawer and the docked popup are chrome (like the tab
        // bar/statusline), not per-pane — one instance of each, registered
        // directly on `engine_view.bottom_bands` rather than through
        // `build_pane`. Only one is ever non-empty at a time in practice.
        let drawer_view: Arc<RwLock<Option<crate::ui::drawer::DrawerViewState>>> =
            Arc::new(RwLock::new(None));
        let popup_band_view: Arc<RwLock<Option<crate::ui::popup::PopupBandState>>> =
            Arc::new(RwLock::new(None));
        engine_view.bottom_bands = vec![
            Box::new(crate::ui::drawer::DrawerWidget {
                data: Arc::clone(&drawer_view),
            }),
            Box::new(crate::ui::popup::PopupBandWidget {
                data: Arc::clone(&popup_band_view),
            }),
        ];

        // Insert a buffer — just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(());

        let settings = EditorSettings::default();

        // Build the initial pane. Every later split-created pane goes through
        // the same `build_pane` (see `commands::open_pane`).
        let (pane, render_handles) = build_pane(
            &mut engine_view.registry,
            &minibuf_completion_view,
            &popup_view,
            &menu_view,
            &completion_menu_view,
            &picker_view,
            settings.wrap_mode,
            buffer_id,
        );
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let jump_list_capacity = settings.jump_list_capacity;
        let history_capacity = settings.history_capacity;
        let initial_mouse_mode = (settings.mouse_enabled, settings.mouse_select);

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
                // `kitty_enabled: false` below matches: the real probe result
                // isn't known until `set_kitty_support` runs, after `open`.
                config: super::ConfigState::new(false, 0),
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
                terminate_exit_code: Arc::new(std::sync::atomic::AtomicI32::new(0)),
                minibuf: None,
                minibuf_completion: None,
                status_msg: None,
                summary_ttl: 0,
                message_log: MessageLog::new(),
                settings,
                last_find: None,
                force_full_redraw: false,
                inline_output: super::InlineOutputDispatch::Inactive,
                #[cfg(test)]
                inline_output_entered: false,
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
                    let mut render = SecondaryMap::new();
                    render.insert(pane_id, render_handles);
                    PaneView {
                        state: pane_buf_state,
                        transient,
                        jumps,
                        render,
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
                dispatching_typed_command: false,
                is_replaying: false,
                mouse_drag_anchor: None,
                cwd: std::env::current_dir().unwrap_or_default(),
                lsp_completion_dismiss_pending: false,
                completion_menu_view,
                minibuf_completion_view,
                diagnostic_scopes: None,
                inlay_hint_scope: None,
                virtual_text_fallback_scope: None,
                runtime_scope_cache: rustc_hash::FxHashMap::default(),
                popup_view,
                popup_band_view,
                menu_view,
                drawer_view,
                picker_view,
                wake: Arc::clone(&wake),
            },
            view: engine_view,
            kitty_enabled: false,
            scripting: None,
            builtin_cmd_names: rustc_hash::FxHashSet::default(),
            parse_worker: Box::new(
                hume_treesitter::parse_worker::ThreadedParseBackend::with_waker(
                    std::sync::Arc::clone(&wake),
                ),
            ),
            parse_worker_disconnect_logged: false,
            timer_wheel: super::timers::TimerWheel::new(),
            timer_payloads: rustc_hash::FxHashMap::default(),
            viewport_debounce: rustc_hash::FxHashMap::default(),
            last_viewport_key: rustc_hash::FxHashMap::default(),
            virtual_lines_synced: rustc_hash::FxHashMap::default(),
            lsp: super::lsp::LspState::new_threaded(std::sync::Arc::clone(&wake)),
            tui_active: false,
            terminal: None,
            applied_mouse_mode: initial_mouse_mode,
        })
    }

    /// Attach the shared terminal handle `run` will read/write and the
    /// inline-output bracket will borrow. Call once, before entering `run`.
    pub(crate) fn attach_terminal(&mut self, term: SharedTerm) {
        self.terminal = Some(term);
    }

    /// Share the atomic the platform terminator thread stores the exit code
    /// into when a signal asks the editor to quit. Replaces the per-`Editor`
    /// default so `run`'s loop and `hume_editor::run` (after `run` returns)
    /// observe the same value the terminator wrote. Call once, before
    /// entering `run`.
    pub(crate) fn attach_terminate_flag(&mut self, code: Arc<std::sync::atomic::AtomicI32>) {
        self.state.terminate_exit_code = code;
    }

    /// Process one key event — dispatch it, sync the search cache, drain any
    /// macro replay, sync again.
    ///
    /// This is the single, non-test path for feeding one keystroke to the editor
    /// from outside the interactive event loop (e.g. headless key-runner).  The
    /// interactive loop handles hook draining itself via [`handle_event`]; here
    /// we use [`handle_key`] directly so the caller doesn't need a scripting host.
    pub(crate) fn step(&mut self, key: KeyEvent) {
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
            Event::Paste(s) => self.handle_terminal_paste(s),
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
        // Cloned once up front (cheap: Arc + EventReader clone) so the loop
        // below can call `&mut self` methods freely while still holding a
        // terminal handle — `self.terminal` itself is never borrowed across
        // the loop body.
        let shared = self.terminal.clone().ok_or_else(|| {
            io::Error::other("event loop requires an Editor with attach_terminal called")
        })?;
        let reader = shared.event_reader();
        // Render context lives here — allocated once, reused every frame.
        // It must be outside `self` so `render_into` can borrow `self`
        // immutably while `ctx` is borrowed mutably alongside it.
        let mut ctx = RenderContext::new();
        let mut last_cursor_color_mode: Option<EditorMode> = None;
        loop {
            // A signal asked us to quit — checked before drawing, since the
            // terminator thread's grace window is already ticking and a
            // frame here would be wasted work. Falls through to the same
            // post-loop teardown as a typed `:q` (`lsp_shutdown_all`, cursor
            // reset). Unlike `should_quit`, this bypasses dirty-buffer
            // prompts — a signal isn't a `:q`. `0` means "no termination
            // requested"; never a valid signal-termination exit code.
            if self
                .state
                .terminate_exit_code
                .load(std::sync::atomic::Ordering::Acquire)
                != 0
            {
                break;
            }

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
                let (_, gutter_w) = self.resolve_pane_settings(self.state.focused_pane_id);
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
                let mut rm = super::commands::pane_row_map(
                    self.doc(),
                    &self.state.settings,
                    &self.view.panes[self.state.focused_pane_id],
                    &mut ctx.cursor_format,
                );
                super::cursor::screen_pos(&vp, &mut rm, cursor_char)
                    .map(|(col, row)| (col + gutter_w + ox, row + oy))
            } else {
                None
            };

            // Open the synchronized-output envelope so the terminal defers
            // display until after every byte of this frame has been written.
            // Terminals that don't support DEC 2026 silently ignore the
            // sequence — hence `let _ =` rather than `?`.
            let _ = hume_platform::terminal::begin_synchronized_update(&shared);
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
            let _ = hume_platform::terminal::set_cursor_shape(
                &shared,
                self.state.mode().cursor_is_bar(),
            );
            if last_cursor_color_mode != Some(self.state.mode()) {
                // Command/Search place the cursor on a white statusline background;
                // use black so it remains visible. All other modes reset to default.
                let black = matches!(self.state.mode(), EditorMode::Command | EditorMode::Search);
                let _ = hume_platform::terminal::set_cursor_color(&shared, black);
                last_cursor_color_mode = Some(self.state.mode());
            }
            // Close the synchronized-output envelope: the terminal now atomically
            // paints the complete frame — clear + cells + cursor shape in one shot.
            let _ = hume_platform::terminal::end_synchronized_update(&shared);

            // ── 3. Event ──────────────────────────────────────────────────────
            // Blocks until a matching event is available, a wake from a
            // background thread (parse worker, LSP transport, SIGWINCH — the
            // reader's source routes it internally), or the nearest async
            // source's deadline — whichever comes first. Idle (no deadline)
            // blocks indefinitely, so we never burn CPU while the editor is
            // at rest. `Ok(false)` covers both a timeout and a waker
            // interrupt — either way, loop back to the top: `prepare_frame`
            // drains every async source regardless of why we woke, and
            // `term.size()` re-reads the viewport (covers SIGWINCH).
            match reader.poll(self.wake_timeout(), |_| true) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => return Err(e),
            }
            match reader.read(|_| true)? {
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
                Event::Paste(text) => {
                    self.handle_event(Event::Paste(text));
                    self.sync_search_cache();
                }
                Event::WindowResized(_) => {
                    // Drain any additional resize events that are already queued
                    // so a drag (which emits one event per delta) collapses into a
                    // single render on the next iteration. Viewport dimensions are
                    // re-read at loop top, so only the final size matters.
                    // Non-resize events that arrive during the drain are handled
                    // inline so they are never lost.
                    while reader.poll(Some(Duration::ZERO), |_| true)? {
                        match reader.read(|_| true)? {
                            Event::WindowResized(_) => continue,
                            // A window manager can resize and refocus in the
                            // same gesture (snapping a tile, say) — the
                            // `_ => break` catch-all below would otherwise
                            // swallow this without running the disk check.
                            Event::FocusIn => {
                                self.check_all_disk_state();
                                break;
                            }
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
                            Event::Paste(text) => {
                                self.handle_event(Event::Paste(text));
                                self.sync_search_cache();
                                break;
                            }
                            _ => break,
                        }
                    }
                }
                // Regaining focus is one of the external-file-change check's
                // trigger points (alongside buffer-enter and `:checktime`) —
                // see `Editor::check_all_disk_state`. `FocusOut` needs no
                // handling: there's nothing to check until focus returns.
                Event::FocusIn => self.check_all_disk_state(),
                // CSI/OSC/DCS protocol responses: nothing in the run loop
                // needs them. The `|_| true` filter guarantees they can't
                // pile up unread in the reader's buffer either way.
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
        self.lsp_shutdown_all(Self::SHUTDOWN_GRACE);
        // Restore the user's default cursor shape and colour before returning to the shell.
        hume_platform::terminal::reset_cursor_shape(&shared)?;
        let _ = hume_platform::terminal::set_cursor_color(&shared, false); // emits reset sequence
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
        let show_indent_guides = doc.overrides.show_indent_guides(&self.state.settings);
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
                show_indent_guides,
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
            |bid| {
                self.state
                    .buffers
                    .try_get(bid)
                    .and_then(|b| b.syntax.as_ref())
                    .map(|s| s as &dyn hume_engine::providers::SyntaxSpans)
            },
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

    /// Drop `viewport_debounce`/`last_viewport_key`/`virtual_lines_synced`
    /// entries whose pane no longer exists in `self.view.panes`. A pending
    /// debounce timer is cancelled outright (its `TimerPayload` no-ops via
    /// `fire_hook_viewport_change`'s own liveness check anyway, but there is
    /// no reason to let it sit in the wheel until it fires).
    fn prune_closed_pane_caches(&mut self) {
        let panes = &self.view.panes;
        self.last_viewport_key
            .retain(|pid, _| panes.contains_key(*pid));
        self.virtual_lines_synced
            .retain(|pid, _| panes.contains_key(*pid));
        let wheel = &mut self.timer_wheel;
        let payloads = &mut self.timer_payloads;
        self.viewport_debounce.retain(|pid, id| {
            let live = panes.contains_key(*pid);
            if !live {
                wheel.cancel(*id);
                payloads.remove(id);
            }
            live
        });
    }

    /// Re-apply the terminal's mouse-tracking mode if `mouse-enabled`/
    /// `mouse-select` changed since the last time this ran. A no-op when
    /// nothing changed (the common case, checked every frame) and when no
    /// terminal is attached (tests, headless `run_keys`).
    ///
    /// The comparison-and-update itself doesn't require a live terminal, so
    /// it stays outside the `if let Some(term)` below — this keeps
    /// `applied_mouse_mode` in sync with `state.settings` even headless,
    /// which is what makes the change-detection unit-testable without a
    /// real `SharedTerm`.
    pub(super) fn resync_mouse_mode(&mut self) {
        let desired = (
            self.state.settings.mouse_enabled,
            self.state.settings.mouse_select,
        );
        if desired == self.applied_mouse_mode {
            return;
        }
        if let Some(term) = &self.terminal {
            let _ = hume_platform::terminal::set_mouse_mode(term, desired.0, desired.1);
        }
        self.applied_mouse_mode = desired;
    }

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// `sync_all_pane_mirrors` is the **single sync point** for `pane.selections`
    /// and `pane.primary_idx` — it covers every pane in one pass.  No other code
    /// path writes those fields.  It, and the scroll pass right after it, run
    /// *after* the async/Steel drains (step 2) since those can switch a pane's
    /// `buffer_id` (picker accept, LSP goto-definition) or move its selections
    /// (timer/LSP callbacks) — syncing or scrolling any earlier would use a
    /// stale selection head against the pane's new buffer, which can be out of
    /// bounds for that rope, or leave the new buffer's cursor unvalidated
    /// against the viewport for a frame. Highlight and statusline shared
    /// buffers are also written here, immediately before every `render()` call.
    /// Mode and display settings are resolved lazily via the
    /// `get_pane_settings` closure passed to `render()`.
    pub(super) fn prepare_frame(
        &mut self,
        terminal_width: u16,
        terminal_height: u16,
        ctx: &mut RenderContext,
    ) {
        // Reclaim viewport-debounce/scroll-key/virtual-line-sync cache
        // entries for panes closed since the last frame. These three live on
        // `Editor` rather than `EditorState.panes` (unlike `jumps`/`render`/
        // `transient`/`state`, which `drop_pane_state` clears directly), so
        // this per-frame sweep is where they get reclaimed instead.
        self.prune_closed_pane_caches();

        // `mouse-enabled`/`mouse-select` are terminal modes, not per-frame
        // render state — `init` (hume-editor/src/lib.rs) only applies them
        // once at startup. This is the per-frame chokepoint that makes a
        // later `:set global mouse-enabled=…` take effect immediately
        // instead of silently doing nothing until restart (see L2 in
        // docs/LESSONS.md: resync self-triggers at the one place the value
        // is consumed, not at every write site).
        self.resync_mouse_mode();

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

        // 2. Drain completed async work (parse results, LSP), then evaluate
        //    any Steel calls that work queued (LSP request/timer callbacks).
        //    `drain_pending_steel_calls` also unconditionally consumes any
        //    deferred LSP-completion dismissal (`set_mode`'s Insert-exit arm)
        //    — this is what makes step 10's `sync_completion_menu_view` below
        //    always see an up-to-date session, with no separate call needed.
        self.drain_async_sources();
        self.drain_pending_steel_calls();

        // 3. Sync line-number style provider for every pane (depends on that
        //    pane's own buffer overrides). Must run after step 2: the drains
        //    can switch a pane's `buffer_id` (picker accept, LSP
        //    goto-definition), so syncing any earlier would apply the
        //    just-left buffer's style to the pane's new buffer for a frame.
        //    Iterates a fresh pane-id snapshot (not the frame-start `rects`)
        //    since a drained callback may have closed a pane.
        for pid in self.view.panes.keys().collect::<Vec<_>>() {
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

        // 4. Sync selection mirrors for every pane. Must run after step 2:
        //    the drains can switch a pane's `buffer_id` (picker accept, LSP
        //    goto-definition) or move its selections (timer/LSP callbacks),
        //    and render (right after this function returns) reads this
        //    mirror against the pane's *current* buffer.
        self.sync_all_pane_mirrors();

        // 5. Sync everything that decides row counts/columns for step 6's
        //    `RowMap`-driven scroll, in this order because none of them
        //    depends on this frame's viewport (a gutter/decoration change
        //    must be visible to the scroll math that positions the cursor
        //    against it, not just to the renderer one step later):
        //      5a. gutter sign data (diagnostics + plugin signs) — decides
        //          gutter width, which decides `Pane::content_width`, which
        //          decides the wrap column.
        //      5b/5c/5d. inlay hints / virtual lines / end-of-line
        //          diagnostic summaries — each a `RowMap` provider
        //          (`inline_decorations` or `virtual_lines`) that
        //          `RowMap::format_line`/`block` reads, so they change wrap
        //          row counts and columns the moment they appear.
        //    Tradeoff: 5a/5b/5d scope their own work to
        //    `visible_char_range`/`visible_line_range`, which read
        //    `viewport.top_line` — so a same-frame scroll (step 6) can leave
        //    a newly-exposed line's hints/signs unsynced until next frame.
        //    That's a one-frame cosmetic lag that self-corrects; syncing
        //    after scroll instead would let step 6's `RowMap` see row
        //    counts/columns the providers haven't caught up to yet — the
        //    scroll/render/caret disagreement this ordering avoids.
        //    `update_virtual_line_providers` has no viewport dependency, so
        //    its position here is unconditional either way.
        self.update_sign_providers();
        self.update_inlay_hint_providers();
        self.update_virtual_line_providers();
        self.update_inline_diagnostics_providers();

        // 6. Scroll every pane so its primary cursor stays visible. Must run
        //    after step 2: the drains can switch a pane's `buffer_id` mid-frame
        //    (picker accept, LSP goto-definition), and this reads buffer_id/
        //    rope/cursor together from SSOT, so it always scrolls the pane's
        //    *current* buffer instead of leaving a just-switched-to buffer's
        //    cursor unvalidated against the viewport for a frame. Iterates a
        //    fresh pane-id snapshot (not the frame-start `rects`) since a
        //    drained callback may have closed a pane.
        let scrolloff = self.state.settings.scrolloff;
        let pane_ids: Vec<PaneId> = self.view.panes.keys().collect();
        for pid in pane_ids {
            let buf_id = self.view.panes[pid].buffer_id;
            let cursor_char = self.state.panes.state[pid][buf_id]
                .selections
                .primary()
                .head();
            scroll_into_view(
                self.state.buffers.get(buf_id),
                &self.state.settings,
                &mut self.view.panes[pid],
                cursor_char,
                &mut ctx.cursor_format,
                scrolloff,
            );

            // A real visible-range change (scroll command, cursor-follow
            // during typing, or a resize that altered height) debounces
            // OnViewportChange. This is bookkeeping over scroll_into_view's
            // *result*, not part of computing what to render — the hook
            // itself never fires from here, only the coalescer timer gets
            // (re)armed; the actual fire happens later via the async-source
            // drain, same as every other timer. Arming here (after step 2's
            // drain) means a change detected this frame is picked up by
            // *next* frame's drain — one frame later than when this ran
            // pre-drain, immaterial for any nonzero debounce interval.
            let viewport = &self.view.panes[pid].viewport;
            let key = (viewport.top_line, viewport.height);
            if self.last_viewport_key.insert(pid, key) != Some(key) {
                self.debounce_viewport_change(pid);
            }
        }

        // 7. Sync highlight data (search matches, bracket matches, diagnostic
        //    underlines, extra highlights) to shared Arc buffers read by the
        //    highlight providers during rendering. Render-only — no `RowMap`
        //    consumer reads highlight scope, only the cell's styling.
        self.update_highlight_providers();

        // 8. Sync completion-popup view to the shared Arc for `MinibufCompletionOverlay`.
        self.sync_minibuf_completion_view();

        // 9. Store the terminal area + divider setting: pane-focus/split
        //    commands have no terminal handle between frames, so they
        //    recompute geometry from these via `EngineView::pane_rects`/
        //    `pane_rect` rather than trusting a stored rect list.
        self.view.last_pane_area = pane_area;
        self.view.last_terminal_area = terminal_area;
        self.view.reserve_seam = reserve_seam;

        // 10. Sync the popup-, menu-, LSP-completion-menu-, and
        //     picker-overlay views. Deliberately *after* step 9: their
        //     geometry needs the focused pane's current-frame rect via
        //     `EngineView::pane_rect` (popup/menu/completion) or
        //     `last_pane_area` directly (picker), which reads `last_pane_area`
        //     — calling this any earlier would position against last frame's
        //     geometry.
        self.sync_popup_view(ctx);
        self.sync_popup_band_view();
        self.sync_menu_view(ctx);
        self.sync_completion_menu_view(ctx);
        self.sync_picker_view();
        // The drawer has no cursor-relative geometry, so it doesn't need
        // step 10's ordering — but it's synced here unconditionally anyway
        // (self-healing), on top of every direct mutation-site call, so the
        // view can never drift from `state.config.drawer` for a frame. See
        // `EditorState::sync_drawer_view`'s doc.
        self.state.sync_drawer_view();
    }

    /// Sync every engine pane's selection mirror from the authoritative `pane_state`.
    ///
    /// The engine requires `pane.selections` sorted by `head` (not by `start()` as
    /// `SelectionSet` stores internally); `primary_idx` is re-located by matching
    /// the primary's head value after the sort.  This is the **single sync point** —
    /// no other code path writes `pane.selections` or `pane.primary_idx`.
    ///
    /// Called once per frame from `prepare_frame`, after the async/Steel
    /// drains and before `render()`.
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

    /// `true` if `ensure_inline_output_screen` has ever actually entered the
    /// inline-output terminal bracket (alt-screen toggle + "press any key")
    /// on this `Editor`. Off the event loop this must stay `false` for every
    /// `#:inline-output #t` command dispatched, output or not — see
    /// `tui_active` on `Editor`.
    #[cfg(test)]
    pub(crate) fn inline_output_entered(&self) -> bool {
        self.state.inline_output_entered
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

    /// Pane `pid`'s visible viewport as `(top_line, bottom_line)`, before any
    /// clamping to the buffer's actual line count — the shared basis for
    /// [`Self::visible_char_range`] and [`Self::visible_line_range`].
    fn visible_line_bounds(&self, pid: PaneId) -> (usize, usize) {
        let vp = &self.view.panes[pid].viewport;
        let top_line = vp.top_line;
        (top_line, top_line + vp.height as usize)
    }

    /// The char range of `bid`'s content currently visible in pane `pid` —
    /// shared by every per-frame write side that pulls a bounded slice from a
    /// Rust-side store (diagnostics, decorations) instead of the whole buffer.
    /// HUME buffers always end with a structural `\n`, so `len_lines() >= 1`
    /// always holds.
    fn visible_char_range(&self, pid: PaneId, bid: BufferId) -> std::ops::Range<usize> {
        let (top_line, bot_line) = self.visible_line_bounds(pid);
        let text = self.state.buffers.get(bid).text();
        let len_lines = text.len_lines();
        let top_char = text.line_to_char(top_line.min(len_lines - 1));
        let end_char = if bot_line + 1 < len_lines {
            text.line_to_char(bot_line + 1)
        } else {
            text.len_chars()
        };
        top_char..end_char
    }

    /// The line range of `bid`'s content currently visible in pane `pid`
    /// (end-exclusive) — used by line-indexed stores (gutter signs) instead
    /// of the char-offset stores' [`Self::visible_char_range`].
    fn visible_line_range(&self, pid: PaneId, bid: BufferId) -> std::ops::Range<usize> {
        let (top_line, bot_line) = self.visible_line_bounds(pid);
        let len_lines = self.state.buffers.get(bid).text().len_lines();
        top_line.min(len_lines - 1)..(bot_line + 1).min(len_lines)
    }

    /// Interned scope ids for the four diagnostic severities, in
    /// `DiagSeverity` discriminant order (`[error, warning, info, hint]`) —
    /// resolved once and cached, since interning needs `&mut
    /// self.view.registry` but `DiagSeverity` itself lives in `self.state`.
    fn diagnostic_scopes(&mut self) -> [hume_engine::types::ScopeId; 4] {
        if let Some(scopes) = self.state.diagnostic_scopes {
            return scopes;
        }
        let scopes = [
            self.view.registry.intern("diagnostic.error"),
            self.view.registry.intern("diagnostic.warning"),
            self.view.registry.intern("diagnostic.info"),
            self.view.registry.intern("diagnostic.hint"),
        ];
        self.state.diagnostic_scopes = Some(scopes);
        scopes
    }

    /// Interned `ScopeId` for a plugin-supplied runtime scope name (extra
    /// highlights, signs, virtual lines), cached across frames so the same
    /// name string is never re-interned.
    fn runtime_scope(&mut self, name: &str) -> hume_engine::types::ScopeId {
        if let Some(&id) = self.state.runtime_scope_cache.get(name) {
            return id;
        }
        let id = self.view.registry.intern_runtime(name);
        self.state.runtime_scope_cache.insert(name.to_string(), id);
        id
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
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.highlights.search))
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
            if let Some(r) = self.state.panes.render.get(pid) {
                r.highlights
                    .bracket
                    .write()
                    .expect("RwLock not poisoned")
                    .clear();
            }
        }
        if !in_insert {
            let focused = self.state.focused_pane_id;
            if let Some(bracket_arc) = self
                .state
                .panes
                .render
                .get(focused)
                .map(|r| Arc::clone(&r.highlights.bracket))
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

        // ── Diagnostic + extra highlights — every pane ───────────────────────
        // Unlike search/bracket-match highlights, these stay visible in
        // Insert mode: an error squiggle is exactly as relevant while you're
        // editing the line it's on (most editors keep them showing).
        {
            let floor = self.state.settings.lsp_diagnostics_severity_floor;
            let diag_scopes = self.diagnostic_scopes();
            for &(pid, bid) in &panes {
                let Some((diag_arc, extra_arc)) = self.state.panes.render.get(pid).map(|r| {
                    (
                        Arc::clone(&r.highlights.diagnostics),
                        Arc::clone(&r.highlights.extra),
                    )
                }) else {
                    continue;
                };

                let visible = self.visible_char_range(pid, bid);

                // Collect raw diagnostic ranges first — this ends the
                // immutable borrow of `self.lsp` before `runtime_scope`
                // (called below for extra highlights) needs `&mut self`.
                let diags: Vec<(usize, usize, DiagSeverity)> = self
                    .lsp
                    .diagnostics_for_range(bid, visible.clone(), floor)
                    .map(|d| {
                        (
                            d.start.max(visible.start),
                            d.end.min(visible.end),
                            d.severity,
                        )
                    })
                    .collect();

                // Same for extra highlights: collect owned data before
                // resolving each source's scope name to a `ScopeId`.
                let extra_raw: Vec<(usize, usize, String)> = self
                    .state
                    .config
                    .decorations
                    .extra_highlights_for_buffer(bid)
                    .filter(|e| e.start < visible.end && e.end > visible.start)
                    .map(|e| {
                        (
                            e.start.max(visible.start),
                            e.end.min(visible.end),
                            e.scope.clone(),
                        )
                    })
                    .collect();
                let extra: Vec<(usize, usize, hume_engine::types::ScopeId)> = extra_raw
                    .into_iter()
                    .map(|(start, end, name)| (start, end, self.runtime_scope(&name)))
                    .collect();

                let buf = self.state.buffers.get(bid);
                let text = buf.text();

                {
                    let mut raw = Vec::new();
                    for (start, end, severity) in diags {
                        // Priority = severity discriminant: Error(0) beats
                        // Warning(1) beats Info(2) beats Hint(3) in overlaps.
                        push_priority_highlight_lines(
                            text,
                            start,
                            end,
                            severity as u8,
                            diag_scopes[severity as usize],
                            &mut raw,
                        );
                    }
                    let mut data = diag_arc.write().expect("RwLock not poisoned");
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }

                {
                    let mut raw = Vec::new();
                    for (start, end, scope) in extra {
                        // No severity concept for plugin-supplied spans —
                        // uniform priority; overlap ties resolve by push
                        // order (first source registered wins).
                        push_priority_highlight_lines(text, start, end, 0, scope, &mut raw);
                    }
                    let mut data = extra_arc.write().expect("RwLock not poisoned");
                    data.clear();
                    flatten_priority_overlaps(&mut raw, &mut data);
                }
            }
        }
    }

    /// Write per-frame gutter sign data (diagnostics + plugin signs) to every
    /// pane's own `Arc<RwLock<FxHashMap<line, Vec<Sign>>>>` buffers, read by
    /// that pane's `SharedSignSource`s. Stays visible in Insert mode — same
    /// reasoning as [`Self::update_highlight_providers`]'s diagnostics
    /// section. Called from `prepare_frame`'s step 5, *before* scrolling: the
    /// sign column's width feeds `Pane::content_width`, which decides the
    /// wrap column the scroll step's `RowMap` resolves against.
    pub(super) fn update_sign_providers(&mut self) {
        use hume_engine::builtins::sign_column::Sign;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        let floor = self.state.settings.lsp_diagnostics_severity_floor;
        let diag_scopes = self.diagnostic_scopes();
        for &(pid, bid) in &panes {
            let Some((diag_map, plugin_map)) = self.state.panes.render.get(pid).map(|r| {
                (
                    Arc::clone(&r.signs.diagnostics),
                    Arc::clone(&r.signs.plugin),
                )
            }) else {
                continue;
            };

            let visible = self.visible_char_range(pid, bid);
            let visible_lines = self.visible_line_range(pid, bid);

            // Compute the buffer's `signcolumn` setting up front — the
            // configured column count decides how many signs per line the
            // plugin merge keeps (the rest is dropped before the map write).
            let signcolumn = self
                .state
                .buffers
                .get(bid)
                .overrides
                .signcolumn(&self.state.settings);
            let max_plugin_signs = signcolumn.columns as usize;

            // Diagnostics: every line a diagnostic touches gets a marker;
            // the most severe diagnostic wins when several touch one line.
            // Clamped to the buffer's last valid char (same defense the
            // highlight path above takes against a stored diagnostic whose
            // offsets have drifted past the current text) — `char_to_line`
            // panics on an out-of-bounds char index.
            let diag_raw: Vec<(usize, usize, DiagSeverity)> = {
                let text = self.state.buffers.get(bid).text();
                let last_char = text.len_chars().saturating_sub(1);
                self.lsp
                    .diagnostics_for_range(bid, visible.clone(), floor)
                    .map(|d| {
                        (
                            text.char_to_line(d.start.min(last_char)),
                            text.char_to_line(d.end.saturating_sub(1).min(last_char)),
                            d.severity,
                        )
                    })
                    .collect()
            };
            let mut diag_best: rustc_hash::FxHashMap<usize, DiagSeverity> =
                rustc_hash::FxHashMap::default();
            for (start_line, end_line, severity) in diag_raw {
                for line in start_line..=end_line {
                    if !visible_lines.contains(&line) {
                        continue;
                    }
                    diag_best
                        .entry(line)
                        .and_modify(|best| {
                            if severity < *best {
                                *best = severity;
                            }
                        })
                        .or_insert(severity);
                }
            }
            {
                let mut guard = diag_map.write().expect("RwLock not poisoned");
                guard.clear();
                for (line, severity) in diag_best {
                    guard.insert(
                        line,
                        vec![Sign {
                            text: std::borrow::Cow::Borrowed("●"),
                            scope: diag_scopes[severity as usize],
                            priority: 10,
                        }],
                    );
                }
            }

            // Plugin signs (`set-signs!`): top N signs per line by priority,
            // where N = the buffer's configured `signcolumn` columns.
            // Pre-truncating to N here (rather than passing everything
            // through downstream) bounds memory — an unbounded per-line Vec
            // would get cloned every frame by
            // `SharedSignSource::signs_for_line`. Safe only because the sort
            // below is priority-only: same-priority ties resolve by the
            // input order `plugin_raw.sort_by` set just above (source name,
            // ascending), not a second tie-break rule invented here. The
            // only other explicit priority-tie decision in the sign
            // pipeline is `SignColumn::render_row_cells`'s own sort
            // (hume-engine/src/builtins/sign_column.rs, arbitrates plugin vs
            // diagnostics map by source-registration order) — this sort
            // must stay priority-only so it never overrides that.
            let mut plugin_raw: Vec<(String, usize, String, String, i64)> = self
                .state
                .config
                .decorations
                .signs_for_buffer(bid)
                .filter(|(_, e)| visible_lines.contains(&e.line))
                .map(|(source, e)| {
                    (
                        source.to_string(),
                        e.line,
                        e.text.clone(),
                        e.scope.clone(),
                        e.priority,
                    )
                })
                .collect();
            plugin_raw.sort_by(|a, b| a.0.cmp(&b.0));

            let mut plugin_all: rustc_hash::FxHashMap<usize, Vec<(String, String, i64)>> =
                rustc_hash::FxHashMap::default();
            for (_, line, text, scope, priority) in plugin_raw {
                plugin_all
                    .entry(line)
                    .or_default()
                    .push((text, scope, priority));
            }
            {
                let mut guard = plugin_map.write().expect("RwLock not poisoned");
                guard.clear();
                for (line, mut entries) in plugin_all {
                    entries.sort_by_key(|e| std::cmp::Reverse(e.2));
                    entries.truncate(max_plugin_signs);
                    let signs: Vec<Sign> = entries
                        .into_iter()
                        .map(|(text, scope_name, priority)| {
                            let scope = self.runtime_scope(&scope_name);
                            Sign {
                                text: std::borrow::Cow::Owned(text),
                                scope,
                                priority: priority.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
                            }
                        })
                        .collect();
                    guard.insert(line, signs);
                }
            }

            // Compute sign column width from the buffer's `signcolumn` setting:
            // `always` keeps it visible at the configured width; `auto` collapses
            // to zero when no signs are visible in the current viewport (diag_map/
            // plugin_map above only hold visible-line entries — a sign elsewhere
            // in the buffer, scrolled out of view, does not keep the column open).
            let has_signs = {
                let diag_empty = diag_map.read().expect("RwLock not poisoned").is_empty();
                let plugin_empty = plugin_map.read().expect("RwLock not poisoned").is_empty();
                !(diag_empty && plugin_empty)
            };
            let width = match signcolumn.mode {
                crate::settings::SignColumnMode::Always => signcolumn.width(),
                crate::settings::SignColumnMode::Auto => {
                    if has_signs {
                        signcolumn.width()
                    } else {
                        0
                    }
                }
            };
            self.view.panes[pid].providers.sync_sign_column_width(width);
        }
    }

    /// Interned `ScopeId` for `ui.virtual.inlay-hint`, cached across frames —
    /// every inlay hint shares this one scope (locked decision: no per-hint
    /// styling in v1), unlike `runtime_scope`'s plugin-name-keyed cache.
    fn inlay_hint_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.inlay_hint_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.virtual.inlay-hint");
        self.state.inlay_hint_scope = Some(id);
        id
    }

    /// Sync per-pane inlay-hint decorations from the
    /// `decorations.inlay_hints` store to each pane's `InlayHintProvider`
    /// Arc. Gated on `lsp.inlay-hints`: when off, every pane's map is
    /// cleared so a mid-session toggle takes effect immediately rather than
    /// waiting for the store to next change.
    pub(super) fn update_inlay_hint_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        if !self.state.settings.lsp_inlay_hints {
            for &(pid, _) in &panes {
                if let Some(r) = self.state.panes.render.get(pid) {
                    r.inlay_hints.write().expect("RwLock not poisoned").clear();
                }
            }
            return;
        }

        let scope = self.inlay_hint_scope();
        for &(pid, bid) in &panes {
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.inlay_hints))
            else {
                continue;
            };
            let visible = self.visible_char_range(pid, bid);
            let text = self.state.buffers.get(bid).text();

            let mut by_line: rustc_hash::FxHashMap<usize, Vec<InlineInsert>> =
                rustc_hash::FxHashMap::default();
            for entry in self.state.config.decorations.inlay_hints_for(bid) {
                if !visible.contains(&entry.pos) {
                    continue;
                }
                // `before`: byte offset of the char at `pos` itself, so the
                // hint text is spliced in immediately before it. `after`:
                // the next char boundary, so it's spliced in immediately
                // after — `char_to_line_byte` resolves a trailing `\n`'s
                // own position to the *same* line (ropey's line boundaries
                // include their `\n`), so a hint at end-of-line-content
                // never bleeds onto the following line.
                let (line, byte_offset) = if entry.before {
                    char_to_line_byte(text, entry.pos)
                } else {
                    char_to_line_byte(text, entry.pos + 1)
                };
                by_line.entry(line).or_default().push(InlineInsert {
                    byte_offset,
                    text: entry.text.clone(),
                    scope,
                });
            }

            *map.write().expect("RwLock not poisoned") = by_line;
        }
    }

    /// Sync per-pane diagnostics end-of-line decorations from the
    /// `decorations.inline_diagnostics` store to each pane's second
    /// `InlayHintProvider` Arc (`PaneRenderHandles::inline_diagnostics`).
    /// Unconditional per-frame rebuild, same as `update_inlay_hint_providers`
    /// — cheap enough that, unlike `virtual_lines`, it doesn't need a
    /// dirty-tracking generation gate to skip needless work. Both write into
    /// a pane's `inline_decorations` providers, which `RowMap::format_line`
    /// reads, so this feeds wrap row counts and columns exactly like inlay
    /// hints do — called from `prepare_frame`'s step 5, *before* scrolling.
    pub(super) fn update_inline_diagnostics_providers(&mut self) {
        use hume_engine::providers::InlineInsert;

        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        for &(pid, bid) in &panes {
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.inline_diagnostics))
            else {
                continue;
            };

            // Collected into an owned Vec, and every scope name resolved,
            // *before* borrowing buffer text below: `self.runtime_scope`
            // needs `&mut self`, which can't overlap with either the
            // immutable borrow `inline_diagnostics_for` holds on
            // `self.state.config.decorations` or the one `text` will hold on
            // `self.state.buffers`.
            let entries: Vec<(usize, String, String)> = self
                .state
                .config
                .decorations
                .inline_diagnostics_for(bid)
                .iter()
                .map(|e| (e.line, e.text.clone(), e.scope.clone()))
                .collect();
            let resolved: Vec<(usize, String, hume_engine::types::ScopeId)> = entries
                .into_iter()
                .map(|(line, text, scope_name)| (line, text, self.runtime_scope(&scope_name)))
                .collect();

            let text = self.state.buffers.get(bid).text();
            let mut by_line: rustc_hash::FxHashMap<usize, Vec<InlineInsert>> =
                rustc_hash::FxHashMap::default();
            for (line, entry_text, scope) in resolved {
                // End-of-line placement: the line's own trailing '\n' char
                // resolves to a byte offset within `line` (never the next
                // line — see `char_to_line_byte`'s doc comment on the same
                // pattern used for inlay hints' `'after` anchor).
                let line_newline = line_end_exclusive(text, line) - 1;
                let (_, byte_offset) = char_to_line_byte(text, line_newline);
                by_line.entry(line).or_default().push(InlineInsert {
                    byte_offset,
                    text: entry_text,
                    scope,
                });
            }

            *map.write().expect("RwLock not poisoned") = by_line;
        }
    }

    /// Interned `ScopeId` for `ui.virtual` — the same theme key
    /// `Theme::ui.virtual_text` (the struct field) resolves from — used as
    /// the fallback scope for a virtual-line entry with no explicit
    /// `scope`. Cached the same way as [`Self::inlay_hint_scope`].
    fn virtual_text_fallback_scope(&mut self) -> hume_engine::types::ScopeId {
        if let Some(id) = self.state.virtual_text_fallback_scope {
            return id;
        }
        let id = self.view.registry.intern("ui.virtual");
        self.state.virtual_text_fallback_scope = Some(id);
        id
    }

    /// Sync per-pane virtual-line decorations from the
    /// `decorations.virtual_lines` store to each pane's `PaneVirtualLines`
    /// Arc — a `RowMap::block` provider, so this feeds row *counts* the same
    /// way inlay hints/inline diagnostics feed wrap columns. Unlike those two,
    /// this only rebuilds when `decorations.virtual_lines_generation()`
    /// changed since the pane's last sync, since resolving each entry's scope
    /// (`runtime_scope`) is costlier to redo unconditionally every frame.
    /// Called from `prepare_frame`'s step 5, *before* scrolling, no
    /// viewport dependency to make stale.
    /// Every entry becomes an `After(line)` virtual line — no `Before`
    /// anchoring in v1 (inline diagnostics render below
    /// the line they annotate).
    pub(super) fn update_virtual_line_providers(&mut self) {
        use hume_engine::providers::{VirtualLine, VirtualLineAnchor};

        let current_gen = self.state.config.decorations.virtual_lines_generation();
        let panes: Vec<(PaneId, BufferId)> = self
            .view
            .panes
            .iter()
            .map(|(pid, pane)| (pid, pane.buffer_id))
            .collect();

        let fallback_scope = self.virtual_text_fallback_scope();
        for &(pid, bid) in &panes {
            if self.virtual_lines_synced.get(&pid) == Some(&current_gen) {
                continue;
            }
            let Some(map) = self
                .state
                .panes
                .render
                .get(pid)
                .map(|r| Arc::clone(&r.virtual_lines))
            else {
                continue;
            };

            // Collected into an owned Vec first: `self.runtime_scope` needs
            // `&mut self`, which can't overlap with the immutable borrow
            // `virtual_lines_for_buffer` holds on `self.state.config.decorations`.
            let entries: Vec<(usize, String, Option<String>)> = self
                .state
                .config
                .decorations
                .virtual_lines_for_buffer(bid)
                .map(|e| (e.line, e.text.clone(), e.scope.clone()))
                .collect();

            let mut by_line: rustc_hash::FxHashMap<usize, Vec<VirtualLine>> =
                rustc_hash::FxHashMap::default();
            for (line, text, scope_name) in entries {
                let scope = match scope_name {
                    Some(name) => self.runtime_scope(&name),
                    None => fallback_scope,
                };
                let text_len = text.len();
                by_line.entry(line).or_default().push(VirtualLine {
                    anchor: VirtualLineAnchor::After(line),
                    // Overwritten by the engine at collection time with the
                    // registration-assigned id (see `ProviderSet::add_virtual_line_source`).
                    provider_id: 0,
                    text,
                    segments: vec![(0..text_len, scope)],
                });
            }

            *map.write().expect("RwLock not poisoned") = by_line;
            self.virtual_lines_synced.insert(pid, current_gen);
        }
    }

    /// Write the current completion state into the shared `MinibufCompletionView` Arc
    /// so `MinibufCompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    pub(super) fn sync_minibuf_completion_view(&self) {
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if self.state.minibuf_completion.is_none()
            && self
                .state
                .minibuf_completion_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }
        use unicode_width::UnicodeWidthStr as _;
        let view = self.state.minibuf_completion.as_ref().map(|state| {
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
            crate::ui::completion_overlay::MinibufCompletionView {
                rows: state.candidates.iter().map(|c| c.display.clone()).collect(),
                selected: state.selected,
                anchor_col,
                border: self.state.settings.popup_border,
            }
        });
        *self
            .state
            .minibuf_completion_view
            .write()
            .expect("RwLock not poisoned") = view;
    }

    /// The focused pane's primary cursor position — the anchor char for
    /// [`Self::sync_popup_view`] and [`Self::sync_menu_view`] (unlike the
    /// LSP completion menu, which anchors at the session's token-start
    /// char instead, via a separately-computed `anchor_char`).
    fn focused_cursor_char(&self) -> usize {
        let pid = self.state.focused_pane_id;
        self.state.panes.state[pid][self.focused_buffer_id()]
            .selections
            .primary()
            .head()
    }

    /// Screen anchor (absolute cell) + geometry bounds for the focused
    /// pane, given an arbitrary buffer char position — shared by
    /// [`Self::sync_popup_view`], [`Self::sync_menu_view`], and the LSP
    /// completion menu (each passes a different `anchor_char`). Returns
    /// `(anchor, pane_rect, max_width, max_height)`; `None` when the pane
    /// has no rect yet or `anchor_char` isn't currently visible.
    fn popup_anchor_and_bounds(
        &self,
        ctx: &mut RenderContext,
        anchor_char: usize,
    ) -> Option<((u16, u16), ratatui::layout::Rect, u16, u16)> {
        let focused = self.state.focused_pane_id;
        let pane_rect = self.view.pane_rect(focused)?;
        let (_, gutter_w) = self.resolve_pane_settings(focused);
        let vp = &self.view.panes[focused].viewport;
        let buf = self.state.buffers.get(self.focused_buffer_id());
        let content_width = pane_rect.width.saturating_sub(gutter_w);
        let mut rm = super::commands::pane_row_map(
            buf,
            &self.state.settings,
            &self.view.panes[focused],
            &mut ctx.cursor_format,
        );
        let (col, row) = super::cursor::screen_pos(vp, &mut rm, anchor_char)?;
        let anchor = (col + gutter_w + pane_rect.x, row + pane_rect.y);
        // Reserve 2 cells on each axis for the popup's 1-cell frame, so
        // content + border together fit the same envelope this budget used
        // to give to content alone.
        let max_width = crate::ui::popup::MAX_POPUP_WIDTH
            .min(content_width.saturating_sub(4))
            .saturating_sub(2);
        let max_height = (pane_rect.height / 3).max(1).saturating_sub(2).max(1);
        Some((anchor, pane_rect, max_width, max_height))
    }

    /// Write the current *cursor-anchored* popup content into the shared
    /// `PopupState` Arc so `PopupOverlay` can render it during this frame.
    /// Geometry (wrap width, flip/clamp position) is resolved fresh every
    /// frame against the focused pane's *current* rect — never pre-computed
    /// at `show-popup!` call time — so a resize or scroll never leaves it
    /// stale. A docked popup (`PopupLayout::Docked`) is handled by
    /// [`Self::sync_popup_band_view`] instead — this clears `popup_view` for
    /// that case, same as when no popup is open at all.
    ///
    /// Called from `prepare_frame` after `last_pane_area` is set (step 10):
    /// `EngineView::pane_rect` reads that field, so calling this any earlier
    /// would position against the previous frame's geometry.
    pub(super) fn sync_popup_view(&mut self, ctx: &mut RenderContext) {
        let is_cursor = matches!(
            self.state.config.popup.as_ref().map(|m| &m.layout),
            Some(crate::ui::popup::PopupLayout::Cursor)
        );
        if !is_cursor
            && self
                .state
                .popup_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }
        if !is_cursor {
            *self.state.popup_view.write().expect("RwLock not poisoned") = None;
            return;
        }

        let bounds = self.popup_anchor_and_bounds(ctx, self.focused_cursor_char());
        let resolved = bounds.and_then(|(anchor, pane_rect, max_width, max_height)| {
            // Wrap the *full* text, unbounded — a scrollable popup must keep
            // every row reachable, not just the first `max_height` of them.
            // The box itself still caps at `max_height` via `outer_dims`
            // below; `scroll` (clamped against that cap) picks which window
            // of `lines` is visible. `resolve_popup_text` caches this by
            // `max_width`, so an unchanged width across frames is O(1), not
            // a re-wrap.
            let text = self.resolve_popup_text(max_width)?;
            let model_scroll = self.state.config.popup.as_ref()?.scroll;
            let (outer_w, outer_h) = crate::ui::menu_box::outer_dims(&text.lines, max_height);
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let inner_h = outer_h.saturating_sub(2) as usize;
            let scroll = model_scroll.min(text.lines.len().saturating_sub(inner_h));
            Some(crate::ui::popup::PopupState {
                lines: text.lines,
                x,
                y,
                outer_w,
                outer_h,
                selected: None,
                scroll,
                styled_rows: text.styled_rows,
                border: self.state.settings.popup_border,
            })
        });

        *self.state.popup_view.write().expect("RwLock not poisoned") = resolved;
    }

    /// Resolve (or reuse the cached) wrap+highlight of the open popup's text
    /// at `max_width` — shared by [`Self::sync_popup_view`] (cursor layout)
    /// and [`Self::sync_popup_band_view`] (docked layout), which never run in
    /// the same frame (mutually exclusive on `PopupModel::layout`), so there
    /// is one cache and one width per frame. See
    /// [`crate::ui::popup::ResolvedPopupText`] for the invalidation contract.
    ///
    /// Returns `None` only if no popup is open — should not happen at either
    /// call site (both gated on `self.state.config.popup` being `Some`), but mirrors
    /// the `Option`-chaining style of the surrounding sync functions rather
    /// than `.expect`-ing a caller invariant.
    fn resolve_popup_text(
        &mut self,
        max_width: u16,
    ) -> Option<crate::ui::popup::ResolvedPopupText> {
        let theme = &self.view.theme;
        let model = self.state.config.popup.as_mut()?;
        let stale = model.resolved.as_ref().is_none_or(|r| r.width != max_width);
        if stale {
            let (lines, styled_rows) = if let Some(popup_syntax) = model.syntax.as_ref() {
                let base_style = theme
                    .resolve_by_name(hume_engine::types::Scope("ui.popup"))
                    .into();
                let runs = popup_syntax.styled_runs(&model.text, theme, base_style);
                let rows = crate::ui::popup::wrap_styled(&runs, max_width);
                let lines: Vec<String> = rows
                    .iter()
                    .map(|row| row.iter().map(|(s, _)| s.as_str()).collect())
                    .collect();
                (std::sync::Arc::new(lines), Some(std::sync::Arc::new(rows)))
            } else {
                (
                    std::sync::Arc::new(crate::ui::popup::wrap_text(&model.text, max_width)),
                    None,
                )
            };
            model.resolved = Some(crate::ui::popup::ResolvedPopupText {
                width: max_width,
                lines,
                styled_rows,
            });
        }
        model.resolved.clone()
    }

    /// Write the current *docked* popup content into the shared
    /// `PopupBandState` Arc so `PopupBandWidget` can render it during this
    /// frame — the `PopupLayout::Docked` counterpart of
    /// [`Self::sync_popup_view`]. Unlike the cursor layout, geometry isn't
    /// resolved here: only content (wrapped lines + scroll clamp), mirroring
    /// the drawer's chrome contract — the engine resolves the band's actual
    /// position/height from `height(max)` at render time.
    ///
    /// Wraps against `last_terminal_area` (the raw, un-subtracted terminal
    /// area), not `last_pane_area` — the band spans the full width the
    /// engine will actually render into (`EngineView::render`'s bottom-band
    /// block), same convention `drawer_visible_rows` already relies on for
    /// its height ceiling.
    pub(super) fn sync_popup_band_view(&mut self) {
        let is_docked = matches!(
            self.state.config.popup.as_ref().map(|m| &m.layout),
            Some(crate::ui::popup::PopupLayout::Docked)
        );
        if !is_docked
            && self
                .state
                .popup_band_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }
        if !is_docked {
            *self
                .state
                .popup_band_view
                .write()
                .expect("RwLock not poisoned") = None;
            return;
        }

        let area = self.view.last_terminal_area;
        let max_width = area.width.saturating_sub(2);
        let resolved = self.resolve_popup_text(max_width).map(|text| {
            let model_scroll = self.state.config.popup.as_ref().map_or(0, |m| m.scroll);
            // Shares `crate::ui::popup::band_capacity` with
            // `PopupBandWidget::height`, so the scroll clamp always agrees
            // with what the engine will next paint (same pattern as
            // `drawer_visible_rows`).
            let max_rows = area.height / 2;
            let capacity = crate::ui::popup::band_capacity(text.lines.len(), max_rows);
            let inner_h = capacity.saturating_sub(2) as usize;
            let scroll = model_scroll.min(text.lines.len().saturating_sub(inner_h));
            crate::ui::popup::PopupBandState {
                lines: text.lines,
                scroll,
                styled_rows: text.styled_rows,
                border: self.state.settings.popup_border,
            }
        });

        *self
            .state
            .popup_band_view
            .write()
            .expect("RwLock not poisoned") = resolved;
    }

    /// Write the current menu content into the shared `PopupState` Arc so
    /// `PopupOverlay` can render it during this frame — same geometry rules
    /// as [`Self::sync_popup_view`], but items are shown one-per-line as-is
    /// (no word-wrap: menu entries are short labels, not prose) and
    /// `selected` marks the highlighted row.
    pub(super) fn sync_menu_view(&self, ctx: &mut RenderContext) {
        if self.state.config.menu.is_none()
            && self
                .state
                .menu_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }

        let resolved = self.state.config.menu.as_ref().and_then(|model| {
            let (anchor, pane_rect, _max_width, _max_height) =
                self.popup_anchor_and_bounds(ctx, self.focused_cursor_char())?;
            let lines: Vec<String> = model.items.clone();
            let (outer_w, outer_h) =
                crate::ui::menu_box::outer_dims(&lines, crate::ui::menu_box::MAX_MENU_ROWS);
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let selected = if lines.is_empty() {
                None
            } else {
                Some(model.selected.min(lines.len() - 1))
            };
            Some(crate::ui::popup::PopupState {
                lines: std::sync::Arc::new(lines),
                x,
                y,
                outer_w,
                outer_h,
                selected,
                scroll: 0, // ignored: a menu windows around `selected`, not `scroll`
                styled_rows: None, // menus never highlight per-span, only per-row
                border: self.state.settings.popup_border,
            })
        });

        *self.state.menu_view.write().expect("RwLock not poisoned") = resolved;
    }

    /// Write the LSP completion menu into the shared `PopupState` Arc —
    /// same widget as [`Self::sync_menu_view`] (unwrapped rows,
    /// selected-row styling), but anchored at the completion session's
    /// token-start char rather than the live cursor (which drifts as the
    /// user types further into the token). Called every frame from
    /// `prepare_frame`'s step 10, same as [`Self::sync_popup_view`]/
    /// [`Self::sync_menu_view`] and for the same reason: it needs
    /// `EngineView::pane_rect`, which reads `last_pane_area` — only current
    /// after step 9 runs.
    pub(super) fn sync_completion_menu_view(&self, ctx: &mut RenderContext) {
        if self.lsp.completion.is_none()
            && self
                .state
                .completion_menu_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }

        let resolved = self.lsp.completion.as_ref().and_then(|session| {
            let (anchor, pane_rect, _max_width, _max_height) =
                self.popup_anchor_and_bounds(ctx, session.anchor())?;
            let lines: Vec<String> = session
                .top(usize::MAX)
                .iter()
                .map(completion_row_label)
                .collect();
            let (outer_w, outer_h) =
                crate::ui::menu_box::outer_dims(&lines, crate::ui::menu_box::MAX_MENU_ROWS);
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let selected = if lines.is_empty() {
                None
            } else {
                let idx = self.lsp.completion_ui.as_ref().map_or(0, |ui| ui.selected);
                Some(idx.min(lines.len() - 1))
            };
            Some(crate::ui::popup::PopupState {
                lines: std::sync::Arc::new(lines),
                x,
                y,
                outer_w,
                outer_h,
                selected,
                scroll: 0, // ignored: a menu windows around `selected`, not `scroll`
                styled_rows: None, // menus never highlight per-span, only per-row
                border: self.state.settings.popup_border,
            })
        });

        *self
            .state
            .completion_menu_view
            .write()
            .expect("RwLock not poisoned") = resolved;
    }

    /// Write the open picker session into the shared `PickerViewState` Arc
    /// so `PickerOverlay` can paint it this frame. Same step-10 timing as
    /// `sync_popup_view`/`sync_menu_view`/`sync_completion_menu_view` (needs
    /// `last_pane_area`, set in step 9) but, unlike them, centers in the
    /// panes region rather than anchoring at the cursor — no `RenderContext`
    /// needed.
    ///
    /// Takes `&mut self`: before snapshotting the visible window, it calls
    /// `PickerSession::move_selection(0, geo.list_rows)` — a delta-0 move is
    /// a pure scroll-clamp against the *current* geometry, so a terminal
    /// resize between the last keystroke and this frame self-heals here
    /// rather than leaving a stale scroll offset from a taller frame.
    pub(super) fn sync_picker_view(&mut self) {
        if self.state.config.picker.is_none()
            && self
                .state
                .picker_view
                .read()
                .expect("RwLock not poisoned")
                .is_none()
        {
            return;
        }

        let geo = crate::ui::picker_panel::panel_geometry(self.view.last_pane_area);
        let resolved = match (self.state.config.picker.as_mut(), geo) {
            (Some(session), Some(geo)) => {
                session.move_selection(0, geo.list_rows);
                let rows: Vec<String> = session.window(geo.list_rows).map(str::to_string).collect();
                let selected_row =
                    (!rows.is_empty()).then(|| session.selected() - session.scroll());
                Some(crate::ui::picker_panel::PickerViewState {
                    prompt: session.prompt().to_string(),
                    query: session.query().to_string(),
                    rows,
                    selected_row,
                    matched: session.matched_len(),
                    total: session.total_len(),
                    pending: session.is_pending(),
                    x: geo.x,
                    y: geo.y,
                    width: geo.width,
                    height: geo.height,
                    border: self.state.settings.popup_border,
                })
            }
            _ => None,
        };

        *self.state.picker_view.write().expect("RwLock not poisoned") = resolved;
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
/// Calls the clamp and both the vertical and horizontal `ensure_cursor_visible`
/// helpers in one shot, over a single row map — so the three agree on the row
/// list by construction, and a line's format is reused across them.
pub(super) fn scroll_into_view(
    doc: &Buffer,
    settings: &EditorSettings,
    pane: &mut Pane,
    cursor_char: usize,
    scratch: &mut hume_engine::format::FormatScratch,
    scrolloff: usize,
) {
    use super::scroll;
    let (mut rm, viewport) = super::commands::pane_row_map_mut(doc, settings, pane, scratch);
    // Self-heal a viewport top left stale by a write site that doesn't
    // validate it (`recall_scroll`, an LSP jump) before the cursor-follow
    // logic below reads it — see `clamp_viewport_top`'s doc.
    scroll::clamp_viewport_top(viewport, &mut rm);
    scroll::ensure_cursor_visible(viewport, &mut rm, cursor_char, scrolloff);
    scroll::ensure_cursor_visible_horizontal(viewport, &mut rm, cursor_char);
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

/// Formats one `completion_top` row (a decoded `{label, kind, detail}`
/// hashmap) as `"label  detail"`, uniformly styled — per-part dimming would
/// need segment-styled rows, which no card requires.
fn completion_row_label(item: &serde_json::Value) -> String {
    let label = item
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match item.get("detail").and_then(|v| v.as_str()) {
        Some(detail) if !detail.is_empty() => format!("{label}  {detail}"),
        _ => label.to_string(),
    }
}

/// Yield `(line, byte_start, byte_end)` for each line the *non-empty*
/// `[start, end_char_excl)` char range touches, clipped to that line's own
/// content (up to but excluding its trailing `\n`). Caller must check
/// `start < end_char_excl` first.
///
/// A single-line range yields one triple, byte-identical to converting
/// `start`/`end_char_excl` directly with [`char_to_line_byte`]. A multi-line
/// range yields one triple per touched line. The clip point is deliberately
/// the `\n` char's own position, not `line_end_exclusive` — the latter is
/// the *next* line's start, which `char_to_line_byte` would resolve to the
/// wrong line (byte 0 of the line after).
///
/// Shared by [`push_match_highlight_lines`] (search/bracket matches, one
/// scope per provider) and [`push_priority_highlight_lines`]
/// (diagnostics/extra highlights, one scope + priority per range) — same
/// per-line splitting math, only the tuple shape differs.
fn line_segments(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let last_char = end_char_excl - 1;
    let start_line = buf.char_to_line(start);
    let end_line = buf.char_to_line(last_char);
    (start_line..=end_line).map(move |line| {
        // Every content line ends with a '\n' — HUME buffers always end with
        // a structural trailing '\n', so this position always exists and
        // still belongs to `line` in ropey's line model.
        let line_newline = line_end_exclusive(buf, line) - 1;
        let seg_start = start.max(buf.line_to_char(line));
        let seg_end = end_char_excl.min(line_newline);
        let (_, byte_start) = char_to_line_byte(buf, seg_start);
        let (_, byte_end) = char_to_line_byte(buf, seg_end);
        (line, byte_start, byte_end)
    })
}

/// Push one `(line, byte_start, byte_end)` triple per line the
/// `[start, end_char_excl)` char range touches. See [`line_segments`].
fn push_match_highlight_lines(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
    data: &mut Vec<(usize, usize, usize)>,
) {
    if start >= end_char_excl {
        return;
    }
    data.extend(line_segments(buf, start, end_char_excl));
}

/// Push one `(line, byte_start, byte_end, priority, scope)` quintuple per
/// line the `[start, end_char_excl)` char range touches. See
/// [`line_segments`]; `priority` and `scope` are carried through unchanged
/// for [`flatten_priority_overlaps`] to resolve same-line overlaps from
/// (lower `priority` wins — see that function).
fn push_priority_highlight_lines(
    buf: &hume_editing::text::Text,
    start: usize,
    end_char_excl: usize,
    priority: u8,
    scope: hume_engine::types::ScopeId,
    data: &mut Vec<(usize, usize, usize, u8, hume_engine::types::ScopeId)>,
) {
    if start >= end_char_excl {
        return;
    }
    data.extend(
        line_segments(buf, start, end_char_excl).map(|(l, s, e)| (l, s, e, priority, scope)),
    );
}

/// Flattens overlapping same-line `(start, end, priority, scope)` spans
/// (already split per-line by [`push_priority_highlight_lines`]) into the
/// sorted, non-overlapping sequence the engine's `HighlightSource` contract
/// requires — a single `HighlightSource`'s own output must not overlap
/// itself (cross-tier layering, e.g. diagnostics vs. search matches, is
/// handled automatically by the engine's per-tier `HighlightStack`; this
/// only resolves overlaps *within* one tier, e.g. two diagnostics on the
/// same line). Lower `priority` wins overlapping regions (ties keep
/// whichever was pushed first) — same event-sweep shape as
/// `flatten_overlaps` in `hume-treesitter/src/highlight.rs`
/// (nested tree-sitter injection layers), adapted for scope-carrying
/// diagnostic/extra-highlight spans instead of syntax layers. `raw` need
/// not be pre-sorted; drained (left empty) on return.
fn flatten_priority_overlaps(
    raw: &mut Vec<(usize, usize, usize, u8, hume_engine::types::ScopeId)>,
    out: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    if raw.is_empty() {
        return;
    }
    raw.sort_by_key(|&(line, start, _, _, _)| (line, start));

    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].0;
        let mut j = i;
        while j < raw.len() && raw[j].0 == line {
            j += 1;
        }
        flatten_one_line(&raw[i..j], line, out);
        i = j;
    }
    raw.clear();
}

/// One line's worth of `(_, start, end, priority, scope)` spans (the `line`
/// field is ignored — the caller already grouped by it) → flattened,
/// non-overlapping `(line, start, end, scope)` output. See
/// [`flatten_priority_overlaps`].
fn flatten_one_line(
    group: &[(usize, usize, usize, u8, hume_engine::types::ScopeId)],
    line: usize,
    out: &mut Vec<(usize, usize, usize, hume_engine::types::ScopeId)>,
) {
    if group.len() == 1 {
        let (_, start, end, _, scope) = group[0];
        out.push((line, start, end, scope));
        return;
    }

    // Event sweep: (pos, is_end, seq, priority, scope). `seq` is the span's
    // index within `group`, used to pop the exact matching stack entry.
    // End events sort before start events at the same position so a
    // closing span is popped before a new one at the same byte is pushed.
    let mut events: Vec<(usize, bool, u32, u8, hume_engine::types::ScopeId)> =
        Vec::with_capacity(group.len() * 2);
    for (seq, &(_, start, end, priority, scope)) in group.iter().enumerate() {
        let seq = seq as u32;
        events.push((start, false, seq, priority, scope));
        events.push((end, true, seq, priority, scope));
    }
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Sorted ascending by (priority, seq) — the lowest-priority (highest-
    // severity) active span is always at `stack[0]`.
    let mut stack: Vec<(u8, u32, hume_engine::types::ScopeId)> = Vec::new();
    let mut pos = 0usize;
    for &(event_pos, is_end, seq, priority, scope) in &events {
        if let Some(&(_, _, active_scope)) = stack.first()
            && pos < event_pos
        {
            out.push((line, pos, event_pos, active_scope));
        }
        pos = event_pos;

        if is_end {
            if let Some(idx) = stack
                .iter()
                .position(|&(p, s, _)| p == priority && s == seq)
            {
                stack.remove(idx);
            }
        } else {
            let insert_at = stack.partition_point(|&(p, s, _)| (p, s) < (priority, seq));
            stack.insert(insert_at, (priority, seq, scope));
        }
    }
}
