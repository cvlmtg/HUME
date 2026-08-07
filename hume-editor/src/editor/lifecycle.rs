use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use termina::event::{Event as TerminalEvent, KeyEvent, KeyEventKind};

use hume_engine::pipeline::{BufferId, EngineView, PaneId, RenderContext};
use hume_engine::types::EditorMode;

use hume_platform::terminal::{SharedTerm, Term};

use super::event::EditorEvent;
use super::{Editor, Mode};

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
        use crate::settings::EditorSettings;
        use crate::ui::build_pane;
        use hume_editing::selection::{Selection, SelectionSet};
        use hume_editing::text::Text;
        use hume_engine::pipeline::LayoutTree;
        use hume_ops::register::{KillRing, RegisterSet};
        use slotmap::SecondaryMap;
        use std::collections::VecDeque;

        let startup_cwd = std::env::current_dir()?;
        let mut doc = match file_path {
            // Missing file, valid basename: `hume newfile.txt` opens an
            // empty buffer bound to the path instead of exiting — same
            // `:w`-creates-it semantics as `:e` on a missing file (see
            // `Buffer::from_file_or_new`, `Editor::open_or_dedup`).
            Some(ref path) => Buffer::from_file_or_new(path, &startup_cwd)?,
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
                paste_stamp: None,
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
                message_logged_this_input: false,
                last_entered_buffer: None,
                mouse_drag_anchor: None,
                cwd: startup_cwd,
                lsp_completion_dismiss_pending: false,
                completion_menu_view,
                minibuf_completion_view,
                diagnostic_scopes: None,
                inlay_hint_scope: None,
                virtual_text_fallback_scope: None,
                bracket_match_scope: None,
                search_match_scope: None,
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
    /// from outside the interactive event loop (e.g. headless key-runner). Unlike
    /// [`Self::handle_input`], `step` does not itself call [`Self::settle`] — the
    /// caller (`hume_editor::run_keys`) calls it once per key, mirroring how the
    /// interactive loop settles once per iteration rather than once per input
    /// handler. The one exception is [`Self::drain_replay_queue`], which settles
    /// internally so a macro's buffer-enter diff is observed before its
    /// `is_replaying` guard drops — see that function's doc. Here we use
    /// [`Self::handle_key`] directly so the caller doesn't need a scripting host.
    pub(crate) fn step(&mut self, key: KeyEvent) {
        self.handle_key(key);
        self.sync_search_cache();
        self.drain_replay_queue();
        self.sync_search_cache();
    }

    /// Single interactive input boundary: dispatch one terminal event.
    ///
    /// All interactive input flows through here — key events and mouse events
    /// alike. This no longer drains queued work itself, nor diffs focus
    /// itself (SPEC.md §4): `Editor::run`'s loop calls `settle()` once per
    /// iteration, at the top, and `settle()`'s own fixpoint is where a focus
    /// change made here — or by a hook handler, or by non-interactive Steel/
    /// LSP code — is observed and turned into `OnBufferEnter`. New input
    /// paths should still route through here: it's what marks
    /// `message_logged_this_input` for `settle()`'s disk check to honour
    /// below.
    ///
    /// Sets `message_logged_this_input` whenever this dispatch itself logged
    /// a new warning or error — a command that fails after moving focus
    /// (`:qa` landing on the first dirty buffer) must keep its own message on
    /// screen instead of losing it to an unrelated disk-change confirm the
    /// next `settle()` might open. `settle()` clears the flag once that
    /// drain has run; see `EditorState::message_logged_this_input`'s doc for
    /// why the window spans both calls.
    pub(crate) fn handle_input(&mut self, ev: TerminalEvent) {
        let totals_before = self.state.message_log.totals();
        match ev {
            TerminalEvent::Key(k) => self.handle_key(k),
            TerminalEvent::Mouse(m) => self.handle_mouse(m),
            TerminalEvent::Paste(s) => self.handle_terminal_paste(s),
            // Regaining focus is one of the external-file-change check's
            // trigger points (alongside buffer-enter and `:checktime`) — see
            // `DiskCheckTrigger::Ambient`. `FocusOut` needs no handling:
            // there's nothing to check until focus returns.
            TerminalEvent::FocusIn => self.state.queue_event(EditorEvent::OnFocusGained),
            _ => {}
        }
        self.state.message_logged_this_input = self.state.message_log.totals() != totals_before;
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Sync viewport geometry, settle (drain async sources and the merged
    ///    work queue to quiescence — see `Editor::settle`'s doc; this is
    ///    what closes the stranded-events bug, SPEC.md §3), observe
    ///    `should_quit`, then prepare the frame: sync all editor state to
    ///    the engine pane.
    /// 2. Render.
    /// 3. Block until the next terminal event.
    /// 4. Dispatch the event.
    ///
    /// **Invariant not independently unit-testable** (SPEC.md §7): `settle()`
    /// always runs, and `should_quit` is always observed, before this loop's
    /// `prepare_frame`/draw. `run` itself needs a live terminal and event
    /// reader, so this is verified by this function's own structure below
    /// plus `tests/events.rs`' `:wq`-fires-`OnBufferSave` regression test,
    /// which covers the drain but not the loop ordering — recorded here
    /// rather than faked with a shape-assertion test.
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

            // ── 1. Sync geometry, then settle (single sync point) ────────────
            // `sync_viewport_dims` must run before `settle`: `drain_due_timers`
            // fires `OnViewportChange` off each pane's *current* bounds, and
            // this is what makes them current before that drain runs.
            let size = term.size()?;
            self.sync_viewport_dims(size.width, size.height);
            self.settle();
            // NEW observation point, downstream of `settle()`. `should_quit`
            // is also checked after dispatch below (`:508`, `continue` rather
            // than `break`) — that keeps the loop going for exactly one more
            // iteration, which reaches `settle()` here before this check
            // breaks it. That's what makes `:wq` correct: it queues
            // `OnBufferSave` and sets `should_quit` in the same dispatch, and
            // without this second pass through `settle()` the hook would
            // never fire — the old code observed `should_quit` immediately
            // after dispatch, before any drain ran.
            if self.state.should_quit {
                break;
            }
            self.prepare_frame(&mut ctx);

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
                // Insert / Select: place the terminal cursor at the document
                // head, where `prepare_frame`'s scroll step already resolved it
                // — the row map that decided *where to scroll* had to answer
                // this question anyway, so re-deriving it here would walk the
                // same rows a second time.
                let (_, gutter_w) = self.resolve_pane_settings(self.state.focused_pane_id);
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
                ctx.cursor_screen
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

            // ── 3. Terminal event ─────────────────────────────────────────────
            // Blocks until a matching event is available, a wake from a
            // background thread (parse worker, LSP transport, SIGWINCH — the
            // reader's source routes it internally), or the nearest async
            // source's deadline — whichever comes first. Idle (no deadline)
            // blocks indefinitely, so we never burn CPU while the editor is
            // at rest. `Ok(false)` covers both a timeout and a waker
            // interrupt — either way, loop back to the top: `settle()`
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
                TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_input(TerminalEvent::Key(key));
                    self.sync_search_cache();
                }
                TerminalEvent::Key(_) => {}
                TerminalEvent::Mouse(mouse) => {
                    self.handle_input(TerminalEvent::Mouse(mouse));
                }
                TerminalEvent::Paste(text) => {
                    self.handle_input(TerminalEvent::Paste(text));
                    self.sync_search_cache();
                }
                TerminalEvent::WindowResized(_) => {
                    // Drain any additional resize events that are already queued
                    // so a drag (which emits one event per delta) collapses into a
                    // single render on the next iteration. Viewport dimensions are
                    // re-read at loop top, so only the final size matters.
                    // Non-resize events that arrive during the drain are handled
                    // inline so they are never lost.
                    while reader.poll(Some(Duration::ZERO), |_| true)? {
                        match reader.read(|_| true)? {
                            TerminalEvent::WindowResized(_) => continue,
                            // A window manager can resize and refocus in the
                            // same gesture (snapping a tile, say) — the
                            // `_ => break` catch-all below would otherwise
                            // swallow this without raising `OnFocusGained`.
                            TerminalEvent::FocusIn => {
                                self.handle_input(TerminalEvent::FocusIn);
                                break;
                            }
                            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                                self.handle_input(TerminalEvent::Key(key));
                                self.sync_search_cache();
                                break;
                            }
                            TerminalEvent::Key(_) => break,
                            TerminalEvent::Mouse(mouse) => {
                                self.handle_input(TerminalEvent::Mouse(mouse));
                                break;
                            }
                            TerminalEvent::Paste(text) => {
                                self.handle_input(TerminalEvent::Paste(text));
                                self.sync_search_cache();
                                break;
                            }
                            _ => break,
                        }
                    }
                }
                // Regaining focus raises `OnFocusGained` (see `handle_input`,
                // which is what actually queues it) — the external-file-change
                // sweep is that event's Rust reaction, not a direct call here.
                TerminalEvent::FocusIn => self.handle_input(TerminalEvent::FocusIn),
                // CSI/OSC/DCS protocol responses: nothing in the run loop
                // needs them. The `|_| true` filter guarantees they can't
                // pile up unread in the reader's buffer either way.
                _ => {}
            }

            // Not a `break`: this loop's top now owns the one quit-observation
            // point that runs after `settle()` (see this function's doc) —
            // breaking here instead would strand a hook a quitting dispatch
            // just queued (`:wq`'s `OnBufferSave`). `continue` still skips
            // `drain_replay_queue` below, which is what this check is really
            // for: a macro containing `:q` must not keep replaying past the quit.
            if self.state.should_quit {
                continue;
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
        }
        // Terminal restore and LSP shutdown happen in `hume_editor::run`,
        // after this returns: `restore_for_exit` is the one function allowed
        // to write the unwind escape sequences (it gates on `claim_exit`, the
        // process-wide single-restorer race with the terminator thread — see
        // its doc), so writing them here too would double every escape that
        // isn't idempotent, notably the kitty keyboard-stack pop.
        Ok(())
    }
}
