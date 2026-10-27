//! Starting an editor and running it: opening files and attaching a
//! terminal or the headless runner, startup cursor positions, and the event
//! loop that settles, draws and waits for input.

use hume_grid::Rect;
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use termina::event::Event as TerminalEvent;

use hume_core::{BufferId, Severity};
use hume_engine::pipeline::EngineView;

use hume_platform::screen::Screen;
use hume_platform::terminal::SharedTerm;
use hume_platform::wake::WakeCallback;

use super::Editor;
use super::frame::{FramePacer, FrameTarget};
use super::pipe_run::{Detached, PipeConsole, ReaderConsole, WaitEvent};
use super::tui::Tui;
use crate::cli::{ConfigSource, FileArg};

/// What drives an editor once [`Editor::start`] has set it up.
pub(crate) enum Driver {
    /// A terminal, with the kitty keyboard protocol when the probe found it.
    /// `wake` is what background threads call to wake `run`'s event loop
    /// (see `Editor::open`).
    Terminal {
        kitty_enabled: bool,
        wake: WakeCallback,
        /// The terminal's size when the editor starts.
        size: Rect,
    },
    /// A scripted key stream (`--keys`), with no one to answer a prompt. With
    /// no terminal to negotiate the kitty protocol, it assumes full
    /// capability: without it a Ctrl-letter key such as `<c-w>` does nothing,
    /// since `handle_normal` strips the Ctrl modifier only under kitty.
    Headless,
}

/// The screen a headless run (`--keys`, a test) lays its panes out on: a
/// statusline under an 80x24 pane.
pub(crate) const HEADLESS_TERMINAL: Rect = Rect::new(0, 0, 80, 25);

/// The run loop's console for a pipe run: `Ctrl-c` and resizes from the
/// terminal, progress drawn as full frames.
struct TerminalConsole<'f, 'a> {
    frame: &'f mut FrameTarget<'a>,
    input: ReaderConsole,
}

impl PipeConsole for TerminalConsole<'_, '_> {
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        self.input.next(timeout)
    }

    fn take_deferred(&mut self) -> Vec<TerminalEvent> {
        self.input.take_deferred()
    }

    fn repaint(&mut self, ed: &mut Editor) -> io::Result<()> {
        ed.draw_resized_frame(self.frame)
    }
}

/// Register the chrome on `view` and return the views the editor writes it
/// through: the bottom bands, the floating overlays and the tab bar. None of
/// them is per-pane, so each is registered once. The drawer and the docked
/// popup are chrome like the tab bar and statusline; the overlays paint over
/// every pane.
pub(in crate::editor) fn register_chrome(
    view: &mut EngineView,
) -> (
    hume_ui::OverlayViews,
    hume_engine::lock::SharedSlot<crate::tabline::TablineViewState>,
) {
    let views = hume_ui::OverlayViews::default();
    let tabline_view = hume_engine::lock::SharedSlot::default();
    view.set_chrome(hume_engine::pipeline::Chrome {
        tabbar: Box::new(crate::tabline::TablineWidget {
            data: tabline_view.clone(),
        }),
        bottom_bands: views.bottom_bands().in_stack_order(),
    });
    view.overlays = views.overlays();
    (views, tabline_view)
}

impl Editor {
    // ── Kitty keybinds ──────────────────────────────────────────────────────────

    /// Apply the kitty keyboard-protocol probe result atomically: set the
    /// runtime flag and re-derive the keymap via the same
    /// [`Keymap::with_defaults`](super::keymap::Keymap::with_defaults)
    /// `ConfigState::new` uses, so the kitty-only default keybinds (when
    /// enabled) are installed identically at startup and on every
    /// `:reload-config`. Called once, from [`Self::start`], so the binds can
    /// never diverge from the flag.
    ///
    /// Must run before `init_scripting`: it replaces the keymap wholesale,
    /// so calling it after `init.scm` has evaluated would discard any user
    /// `bind-key!` on top of it.
    pub(in crate::editor) fn set_kitty_support(&mut self, kitty_enabled: bool) {
        self.kitty_enabled = kitty_enabled;
        self.state.config.keymap = super::keymap::Keymap::with_defaults(kitty_enabled);
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
    ///
    /// `dirs` is the session's config, data, runtime and temp directories and
    /// `cwd` its startup working directory; the editor never reads either from
    /// the process.
    pub(in crate::editor) fn open(
        file_path: Option<hume_platform::path::AbsolutePath>,
        terminal: Rect,
        wake: WakeCallback,
        dirs: hume_platform::dirs::Dirs,
        startup_cwd: hume_platform::path::AbsolutePath,
    ) -> io::Result<Self> {
        use super::clipboard;
        use crate::editor::buffer::Buffer;
        use crate::editor::buffer::store::BufferStore;
        use crate::editor::pane_state::{PaneChrome, PaneView};
        use crate::editor::settings::EditorSettings;
        use hume_editing::text::BufferText;

        let doc = match file_path {
            // Missing file, valid basename: `hume newfile.txt` opens an
            // empty buffer bound to the path instead of exiting, same
            // `:w`-creates-it semantics as `:e` on a missing file (see
            // `Buffer::from_file_or_new`, `lifecycle::open_or_dedup_and_notify`).
            Some(path) => Buffer::from_file_or_new(crate::editor::buffer::FileTarget::new(path)?)?,
            None => Buffer::at_start(BufferText::empty()),
        };

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::editor::theme::fallback_theme();
        let (mut engine_view, focus_key) = EngineView::new(theme, terminal);
        let (views, tabline_view) = register_chrome(&mut engine_view);

        // Insert a buffer: just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(());

        let settings = EditorSettings::default();

        let history_capacity = settings.prompt_history_levels;
        let initial_mouse_mode = (settings.mouse, settings.mouse_select);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);
        let mut panes = PaneView::default();
        let unattached = panes.open(
            &mut engine_view,
            buffer_id,
            buffers.get(buffer_id),
            settings.jump_list_capacity,
            PaneChrome::Full,
        );
        let focus = super::focus::Focus::new(&mut engine_view, focus_key, unattached);

        let mut editor = Self {
            state: super::EditorState {
                dirs,
                buffers,
                buffer_positions: super::position_stores::BufferPositions::default(),
                lsp: super::lsp::LspState::new_threaded(std::sync::Arc::clone(&wake)),
                clipboard: clipboard::SystemClipboard::new(),
                settings,
                panes,
                history: super::minibuf::history::HistoryStore::new(history_capacity),
                views,
                tabline_view,
                wake: Arc::clone(&wake),
                ..super::EditorState::base(focus, startup_cwd)
            },
            view: engine_view,
            kitty_enabled: false,
            scripting: None,
            config_source: ConfigSource::Default,
            builtin_cmd_names: hume_scripting::command_name::CommandNameSet::default(),
            parse_worker: Box::new(
                hume_treesitter::parse_worker::ThreadedParseBackend::with_waker(
                    std::sync::Arc::clone(&wake),
                ),
            ),
            parse_worker_disconnect_logged: false,
            timers: super::timer_bridge::Timers::new(),
            viewport_debounce: rustc_hash::FxHashMap::default(),
            diagnostic_pull_debounce: rustc_hash::FxHashMap::default(),
            last_visible_range: rustc_hash::FxHashMap::default(),
            virtual_lines_synced: rustc_hash::FxHashMap::default(),
            tui: Tui::Off,
            applied_mouse_mode: initial_mouse_mode,
            applied_cwd: None,
            startup_positions: Vec::new(),
            detached: None,
            input_backlog: VecDeque::new(),
            #[cfg(test)]
            inline_pipe_console: None,
        };
        // This buffer predates the scripting host, so it can't route through
        // `open_buffer_and_notify`, but it must end up in the state that
        // chokepoint leaves a buffer in, or `detect_pending_languages` never
        // announces its open. Safe to queue this early: detection waits for
        // the scripting host `init_scripting` attaches, and `pending_work`
        // isn't drained until the first `settle()` (`Editor::run`'s loop /
        // `frame.rs`), after `init_scripting` has registered every hook
        // handler.
        super::buffer::lifecycle::queue_open_announcement(&mut editor.state, buffer_id);
        Ok(editor)
    }

    /// Set up an editor on the command line's files, the same way for every
    /// [`Driver`]: open the first file (or a scratch buffer), configure the
    /// driver and the config source, run `init_scripting`, then open the
    /// remaining files so their `OnBufferOpen` hooks fire. Each file's
    /// position is queued, and the first `settle()` places it.
    pub(crate) fn start(
        files: &[FileArg],
        driver: Driver,
        dirs: hume_platform::dirs::Dirs,
        config: ConfigSource,
        startup_cwd: hume_platform::path::AbsolutePath,
    ) -> io::Result<Self> {
        let (first, rest) = files
            .split_first()
            .map_or((None, &[][..]), |(f, r)| (Some(f), r));
        let (wake, detached, kitty_enabled, headless, terminal) = match driver {
            Driver::Terminal {
                kitty_enabled,
                wake,
                size,
            } => (wake, None, kitty_enabled, false, size),
            // No event loop: only a pipe run's wait reads this waker.
            Driver::Headless => {
                let (wake, detached) = Detached::pair();
                (wake, Some(detached), true, true, HEADLESS_TERMINAL)
            }
        };
        let mut editor = Self::open(
            first.map(|f| f.path.clone()),
            terminal,
            wake,
            dirs,
            startup_cwd,
        )?;
        editor.detached = detached;
        editor.set_kitty_support(kitty_enabled);
        editor.state.headless = headless;
        editor.set_config_source(config);
        editor.init_scripting(&mut Default::default());
        if let Some(pos) = first.and_then(|f| f.pos) {
            editor.queue_startup_position(editor.focused_buffer_id(), pos);
        }
        for arg in rest {
            if let (Some(bid), Some(pos)) = (editor.open_extra_file(&arg.path), arg.pos) {
                editor.queue_startup_position(bid, pos);
            }
        }
        Ok(editor)
    }

    /// Share the atomic the platform terminator thread stores the exit code
    /// into when a signal asks the editor to quit. Replaces the per-`Editor`
    /// default so `run`'s loop and `hume_editor::run` (after `run` returns)
    /// observe the same value the terminator wrote. Call once, before
    /// entering `run`.
    pub(crate) fn attach_terminate_flag(&mut self, code: Arc<std::sync::atomic::AtomicI32>) {
        self.state.terminate_exit_code = code;
    }

    /// Share the queue the panic hook fills when a background thread
    /// panics, so [`Self::report_worker_panics`] reads what
    /// `hume_editor::run` reads at exit. Call once, before entering `run`.
    pub(crate) fn attach_worker_panics(
        &mut self,
        panics: hume_platform::worker_panic::WorkerPanics,
    ) {
        self.state.worker_panics = panics;
    }

    /// Report each background-thread panic not yet reported as an Error.
    /// Called once per run-loop iteration.
    pub(crate) fn report_worker_panics(&mut self) {
        for panic in self.state.worker_panics.take_unreported() {
            self.report(Severity::Error, panic.to_string());
        }
    }

    /// Set where `init_scripting` (and every later `:reload-config`) reads
    /// its config from (`--config` / `--no-config`), instead of the default
    /// `<config_dir>/init.scm`.
    ///
    /// Must run before `init_scripting`, same as `set_kitty_support`: the
    /// source is read once resolution starts.
    pub(in crate::editor) fn set_config_source(&mut self, source: ConfigSource) {
        self.config_source = source;
    }

    /// Queue a 1-based CLI position (`hume foo.rs:12:24`) for `bid`, applied
    /// by [`Self::apply_startup_positions`] at the end of the next
    /// `settle()`.
    pub(in crate::editor) fn queue_startup_position(
        &mut self,
        bid: BufferId,
        pos: crate::cli::PathPosition,
    ) {
        self.startup_positions.push((bid, pos));
    }

    /// Apply every startup cursor placement queued by
    /// [`Self::queue_startup_position`], then center the focused buffer's
    /// viewport on its cursor the same way `zz`/`goto-location!` do. No jump
    /// entry is recorded. A startup position is the buffer's origin, not a
    /// jump away from one, so `Ctrl-o` has nothing to return to here (unlike
    /// `:goto` and `goto-location!`, which both do).
    ///
    /// Runs at the end of every `settle()`. Only [`Self::start`] queues, so
    /// the first `settle()` drains the queue and every later one finds it
    /// empty. A headless run centres against its fixed 80x25 screen.
    pub(in crate::editor) fn apply_startup_positions(&mut self) {
        let placements = std::mem::take(&mut self.startup_positions);
        if placements.is_empty() {
            return;
        }

        let focused_bid = self.focused_buffer_id();
        let pid = self.view.focused_pane();
        let mut center_focused = false;
        for (bid, pos) in placements {
            crate::editor::pane_state::park_cursor_at(
                &mut self.state.panes.entries,
                &self.state.buffers,
                &self.view.panes,
                pid,
                bid,
                pos.line,
                pos.grapheme_col,
            );
            center_focused |= bid == focused_bid;
        }
        if center_focused {
            super::commands::view_center(&mut self.state, &mut self.view, pid);
        }
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Sync viewport geometry, settle (drain async sources and the merged
    ///    work queue to quiescence, see `Editor::settle`'s doc; this keeps
    ///    a queued event from stranding), observe
    ///    `should_quit`, then prepare the frame: sync all editor state to
    ///    the engine pane.
    /// 2. Wait for a foreground pipe run the last input started and start
    ///    over (its wait draws its own frames; see `Editor::wait_pipe_run`),
    ///    or render: at once after input, otherwise once the
    ///    [`FramePacer`] says a frame is due.
    /// 3. Block until the next terminal event.
    /// 4. Dispatch the event.
    ///
    /// Whatever exit the loop takes, clipboard text still queued for OSC 52
    /// is sent before the terminal handle is released: a quit or signal exit
    /// skips `prepare_frame`, which would otherwise send it.
    ///
    /// **Invariant not independently unit-testable**: `settle()`
    /// always runs, and `should_quit` is always observed, before this loop's
    /// `prepare_frame`/draw. `run` itself needs a live terminal and event
    /// reader, so this is verified by this function's own structure below
    /// plus `tests/events/settle_queue.rs`' `:wq`-fires-`OnBufferSave` regression test,
    /// which covers the drain but not the loop ordering, recorded here
    /// rather than faked with a shape-assertion test.
    pub(crate) fn run(&mut self, term: &SharedTerm, screen: &mut Screen) -> io::Result<()> {
        // Marks that this Editor owns the terminal for the whole of
        // `run_loop`: dispatch's inline-output bracket (dispatch.rs) reads
        // it to skip alt-screen toggling and the "press any key" block
        // outside the event loop. A wrapper rather than an inline reset at
        // the end: `run_loop` has several `?`-propagated exits, and this is
        // the one point all of them pass through.
        self.tui = Tui::On(term.clone());
        let result = self.run_loop(term, screen);
        self.flush_osc52();
        self.tui = Tui::Off;
        result
    }

    /// The loop itself, described by [`Self::run`]'s doc, split out so
    /// `run` owns the `Tui::On`/`Tui::Off` bracket around every exit.
    fn run_loop(&mut self, term: &SharedTerm, screen: &mut Screen) -> io::Result<()> {
        let reader = term.event_reader();
        let mut frame = FrameTarget::new(term, screen);
        let mut pacer = FramePacer::default();
        loop {
            // A signal asked us to quit. Checked before drawing, since the
            // terminator thread's grace window is already ticking and a
            // frame here would be wasted work. Falls through to the same
            // post-loop teardown as a typed `:q` (`lsp_shutdown_all`, cursor
            // reset). Unlike `should_quit`, this bypasses dirty-buffer
            // prompts: a signal isn't a `:q`. `0` means "no termination
            // requested"; never a valid signal-termination exit code.
            if self.state.terminate_requested() {
                break;
            }

            // An inline-output command toggled the alt-screen, so what the
            // terminal shows no longer matches the front buffer; repaint in
            // full rather than diffing against a stale picture of the screen.
            if std::mem::take(&mut self.state.force_full_redraw) {
                frame.screen().invalidate();
            }

            self.report_worker_panics();

            // ── 1. Sync geometry, then settle (single sync point) ────────────
            // `resize_terminal` must run before `settle`: `drain_due_timers`
            // fires `OnViewportChange` with each pane's size, which is laid
            // out against this terminal. `prepare_frame` lays the frame out
            // after `settle`, so a bottom-band height change made during the
            // drain lands in this frame's render too.
            //
            // `frame.rs`'s `render_to_buf` mirrors this same
            // `resize_terminal` → `settle` → `prepare_frame` sequence for
            // snapshot tests. A step added to this loop needs the same step
            // added there, or every snapshot renders a frame this loop no
            // longer produces. Waiting for a pipe run is not such a step: tests
            // wait through `step`.
            let (term_width, term_height) = frame.screen().size()?;
            self.resize_terminal(term_width, term_height);
            self.settle();
            // Observed here, downstream of `settle()`, not right after
            // dispatch. `should_quit` is also checked after dispatch below
            // (`continue` rather than `break`), which keeps the loop
            // going for exactly one more iteration so it reaches `settle()`
            // here before this check breaks it. That's what makes `:wq`
            // correct: it queues `OnBufferSave` and sets `should_quit` in the
            // same dispatch, and the hook needs this second pass through
            // `settle()` to actually fire before the loop exits.
            if self.state.should_quit {
                break;
            }
            // ── 2. Draw ───────────────────────────────────────────────────────
            // A foreground pipe run the last input started is waited for
            // here, after the settle and before anything else can run. Its
            // wait draws its own frames, and the next pass draws its result.
            if let Some(run) = self.state.pipe_runs.foreground() {
                self.wait_pipe_run(
                    run,
                    &mut TerminalConsole {
                        frame: &mut frame,
                        input: ReaderConsole {
                            reader: reader.clone(),
                            deferred: Vec::new(),
                        },
                    },
                )?;
                continue;
            }
            let frame_wait = pacer.wait(Instant::now());
            if frame_wait.is_none() {
                self.draw_frame(&mut frame)?;
                pacer.drew(Instant::now());
            }

            // ── 3. Terminal event ─────────────────────────────────────────────
            // Input a pipe wait left behind comes first, oldest first.
            // Otherwise blocks until an event is available, a wake from a
            // background thread (parse worker, LSP transport, SIGWINCH: the
            // reader's source routes it internally), or the nearest async
            // source's deadline, whichever comes first. Idle (no deadline)
            // blocks indefinitely, so we never burn CPU while the editor is
            // at rest. `Ok(false)` covers both a timeout and a waker
            // interrupt. Either way, loop back to the top: `settle()`
            // drains every async source regardless of why we woke, and
            // `term.size()` re-reads the viewport (covers SIGWINCH). A frame
            // the pacer held back bounds the wait too.
            let event = match self.input_backlog.pop_front() {
                Some(event) => event,
                None => {
                    let timeout = match (self.wake_timeout(), frame_wait) {
                        (Some(source), Some(frame)) => Some(source.min(frame)),
                        (source, frame) => source.or(frame),
                    };
                    match reader.poll(timeout, |_| true) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(e) => return Err(e),
                    }
                    reader.read(|_| true)?
                }
            };
            // A parse that finished while `poll` was blocked has its result
            // waiting in the worker's channel, and the wake that would have
            // carried the loop back to `settle()` was consumed by this input
            // event. Install it here so a structural command in this
            // dispatch reads a current tree instead of rebuilding one
            // `ensure_syntax_current` would parse inline.
            self.install_parse_results();
            match event {
                TerminalEvent::WindowResized(_) => {
                    // Drain any additional resize events that are already queued
                    // so a drag (which emits one event per delta) collapses into a
                    // single render on the next iteration. Viewport dimensions are
                    // re-read at loop top, so only the final size matters.
                    // The first non-resize event ends the drain and is handled
                    // like any other, so none is lost: a window manager can
                    // resize and refocus in one gesture.
                    while reader.poll(Some(Duration::ZERO), |_| true)? {
                        match reader.read(|_| true)? {
                            TerminalEvent::WindowResized(_) => continue,
                            ev => {
                                self.dispatch_terminal_event(ev);
                                break;
                            }
                        }
                    }
                }
                ev => self.dispatch_terminal_event(ev),
            }
            pacer.input();

            // Not a `break`: this loop's top now owns the one quit-observation
            // point that runs after `settle()` (see this function's doc).
            // Breaking here instead would strand a hook a quitting dispatch
            // just queued (`:wq`'s `OnBufferSave`). `continue` still skips
            // `drain_replay_queue` below, which is what this check is really
            // for: a macro containing `:q` must not keep replaying past the quit.
            if self.state.should_quit {
                continue;
            }

            // ── 4. Drain macro replay queue ───────────────────────────────────
            // Drain after handling the terminal event so that a key that
            // populates the queue (e.g. the register name after `q`) causes
            // replay to run immediately: the results are visible on the very
            // next frame rather than requiring an additional keypress.
            self.drain_replay_queue();
            // One cache update covers the entire replay batch. The search
            // cache only changes when the buffer revision changes, so calling
            // it per-key would redundantly clone the regex on every iteration.
            self.sync_search_cache();
        }
        // Terminal restore and LSP shutdown happen in `hume_editor::run`,
        // after this returns: `restore_for_exit` is the one function allowed
        // to write the unwind escape sequences (it gates on `claim_exit`, the
        // process-wide single-restorer race with the terminator thread; see
        // its doc), so writing them here too would double every escape that
        // isn't idempotent, notably the kitty keyboard-stack pop.
        Ok(())
    }
}
