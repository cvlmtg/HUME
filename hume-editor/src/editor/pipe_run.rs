//! `pipe` runs. [`PipeRuns`] holds every run in flight; each holds its
//! buffer's text with a [`TextHold`], so the text and selections it read are
//! still there when its output lands. `cmd_pipe` starts a run. One whose
//! output the next keys need is waited for in place: by the main loop, by a
//! replay before its next key, or by `step`. Any other runs in the
//! background: `settle` polls it and lands its output when its processes
//! are done, while the user works on.

use std::io;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use hume_core::{BindMode, BufferId, PaneId, Severity};
use hume_editing::selection::SelectionSet;
use hume_platform::process::batch::{BatchProgress, Captured, ShellBatch, ShellFailure};
use hume_platform::wake::WakeCallback;
use termina::EventReader;
use termina::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, Modifiers};

use super::buffer::TextHold;
use super::commands;
use super::frame::FramePacer;
use super::keymap::WalkResult;
use super::replay::{DeferredChange, DeferredOutcome};
use super::{Editor, EditorState};

/// What a pipe does with its command's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PipeKind {
    /// Each selection is its process's input, and the output replaces it.
    Replace,
    /// No input; the output goes where typing goes at each cursor.
    Insert,
}

/// A close refused because it would stop the run in flight in this buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct PipeRunning(pub(in crate::editor) BufferId);

/// A refusal a run in flight may be the reason for, which the `!` of a typed
/// command stops before trying again.
pub(in crate::editor) trait PipeRefusal {
    /// The run in the way, when that is the reason.
    fn pipe_running(&self) -> Option<PipeRunning>;
}

impl PipeRefusal for PipeRunning {
    fn pipe_running(&self) -> Option<PipeRunning> {
        Some(*self)
    }
}

/// A run's identity. Ids grow in start order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::editor) struct RunId(u64);

/// What a run needs to land its output.
pub(in crate::editor) struct Launch {
    pub(in crate::editor) buffer: BufferId,
    /// The pane the run started from: its selections for `buffer` take the
    /// result.
    pub(in crate::editor) pane: PaneId,
    /// The selections the run started from, one process each.
    pub(in crate::editor) selections: SelectionSet,
    /// Whether each process's input ended with a line break.
    pub(in crate::editor) input_ends_line: Vec<bool>,
    pub(in crate::editor) kind: PipeKind,
    pub(in crate::editor) command: String,
    /// The change the `pipe` started, kept only if the run lands its output.
    pub(in crate::editor) settles: Option<DeferredChange>,
    /// Whether whoever started the run waits for it in place. A background
    /// run is polled by `settle` and lands when its processes are done.
    pub(in crate::editor) foreground: bool,
}

/// What a run's processes ended with: one output per selection, in
/// selection order, or the first failure.
type Outcome = Result<Vec<Captured>, ShellFailure>;

struct PipeRun {
    id: RunId,
    launch: Launch,
    hold: TextHold,
    state: RunState,
    started: Instant,
    /// Processes finished, as of the last poll.
    done: usize,
}

enum RunState {
    Running(ShellBatch),
    /// The processes are done; the outcome waits to be landed or reported.
    Ended(Outcome),
}

impl PipeRun {
    /// Collect the processes that finished, giving their slots back to
    /// `free`. The run ends at the first that failed.
    fn collect(&mut self, free: &mut usize) {
        if let RunState::Running(batch) = &mut self.state
            && let Err(failure) = batch.collect(free)
        {
            self.state = RunState::Ended(Err(failure));
        }
    }

    /// Start waiting inputs while `free` has slots, and say where the run
    /// stands.
    fn fill(&mut self, free: &mut usize, wake: &WakeCallback) -> RunPoll {
        let RunState::Running(batch) = &mut self.state else {
            return RunPoll::Ended;
        };
        match batch.fill(free, wake) {
            Ok(BatchProgress::Running { done }) => RunPoll::Running {
                changed: std::mem::replace(&mut self.done, done) != done,
            },
            Ok(BatchProgress::Finished(outputs)) => {
                self.state = RunState::Ended(Ok(outputs));
                RunPoll::Ended
            }
            Err(failure) => {
                self.state = RunState::Ended(Err(failure));
                RunPoll::Ended
            }
        }
    }

    /// The launch and, once the processes are done, their outcome, with the
    /// hold released and every process still running killed.
    fn into_parts(self) -> (Launch, Option<Outcome>) {
        let Self {
            launch,
            hold,
            state,
            ..
        } = self;
        drop(hold);
        let outcome = match state {
            RunState::Ended(outcome) => Some(outcome),
            RunState::Running(_) => None,
        };
        (launch, outcome)
    }
}

/// Where a waited run stands after [`PipeRuns::poll`].
enum RunPoll {
    /// Still running. `changed` when more processes finished since the last
    /// poll.
    Running {
        changed: bool,
    },
    Ended,
}

/// Every pipe run in flight, at most one per buffer, sharing `limit` process
/// slots.
pub(crate) struct PipeRuns {
    runs: Vec<PipeRun>,
    next: u64,
    limit: NonZeroUsize,
}

impl PipeRuns {
    pub(in crate::editor) fn new() -> Self {
        Self::with_limit(std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN))
    }

    /// No runs, sharing `limit` process slots.
    pub(in crate::editor) fn with_limit(limit: NonZeroUsize) -> Self {
        Self {
            runs: Vec::new(),
            next: 0,
            limit,
        }
    }

    /// The id the next run gets: every run started from now on has one at
    /// least this.
    pub(in crate::editor) fn mark(&self) -> RunId {
        RunId(self.next)
    }

    /// The oldest run in flight that its starter waits for in place.
    pub(in crate::editor) fn foreground(&self) -> Option<RunId> {
        self.runs
            .iter()
            .find(|run| run.launch.foreground)
            .map(|run| run.id)
    }

    /// Make run `id` one its starter waits for in place.
    fn claim(&mut self, id: RunId) {
        let run = self
            .runs
            .iter_mut()
            .find(|run| run.id == id)
            .expect("a claimed run is in flight");
        run.launch.foreground = true;
    }

    /// The buffer of the oldest run in flight that one of `panes` started:
    /// the run closing them would stop.
    pub(in crate::editor) fn launched_from(&self, panes: &[PaneId]) -> Option<BufferId> {
        self.runs
            .iter()
            .find(|run| panes.contains(&run.launch.pane))
            .map(|run| run.launch.buffer)
    }

    /// The oldest run in flight started at or after `mark`.
    fn oldest_from(&self, mark: RunId) -> Option<RunId> {
        self.runs.iter().map(|run| run.id).find(|&id| id >= mark)
    }

    /// `bid`'s run, as the statusline shows it: once it has gone on for
    /// [`PROGRESS_GRACE`], until its processes are done.
    pub(crate) fn progress(&self, bid: BufferId) -> Option<PipeProgress> {
        let now = Instant::now();
        self.runs
            .iter()
            .find(|run| run.launch.buffer == bid)
            .filter(|run| {
                matches!(run.state, RunState::Running(_)) && now >= run.started + PROGRESS_GRACE
            })
            .map(|run| PipeProgress {
                command: run.launch.command.clone(),
                done: run.done,
                total: run.launch.input_ends_line.len(),
            })
    }

    fn start(&mut self, launch: Launch, hold: TextHold, batch: ShellBatch) -> RunId {
        debug_assert!(
            self.runs
                .iter()
                .all(|run| run.launch.buffer != launch.buffer),
            "a held buffer starts no second run"
        );
        let id = RunId(self.next);
        self.next += 1;
        self.runs.push(PipeRun {
            id,
            launch,
            hold,
            state: RunState::Running(batch),
            started: Instant::now(),
            done: 0,
        });
        id
    }

    /// Poll the background runs and `waited`'s run with the slots no run
    /// holds: collect every finished process first, then start waiting
    /// inputs, `waited`'s run before the background runs, oldest first, so
    /// its starter waits for at most one other process to free a slot. A
    /// process started here fires `wake` when it finishes. A foreground run
    /// other than `waited`'s is left to its own wait. Where `waited`'s run
    /// stands, `None` without one.
    fn poll(&mut self, waited: Option<RunId>, wake: &WakeCallback) -> Option<RunPoll> {
        let held: usize = self
            .runs
            .iter()
            .map(|run| match &run.state {
                RunState::Running(batch) => batch.running(),
                RunState::Ended(_) => 0,
            })
            .sum();
        let mut free = self.limit.get() - held;
        let polled = |run: &&mut PipeRun| !run.launch.foreground || waited == Some(run.id);
        for run in self.runs.iter_mut().filter(polled) {
            run.collect(&mut free);
        }
        let waited_poll = self
            .runs
            .iter_mut()
            .find(|run| waited == Some(run.id))
            .map(|run| run.fill(&mut free, wake));
        for run in self.runs.iter_mut().filter(|run| !run.launch.foreground) {
            run.fill(&mut free, wake);
        }
        waited_poll
    }

    /// Take the oldest run matching `pred` out of the registry, as
    /// [`PipeRun::into_parts`] does.
    fn take_where(&mut self, pred: impl Fn(&PipeRun) -> bool) -> Option<(Launch, Option<Outcome>)> {
        let i = self.runs.iter().position(pred)?;
        Some(self.runs.remove(i).into_parts())
    }

    /// Take out the oldest background run whose processes are done.
    fn take_ended_background(&mut self) -> Option<(Launch, Outcome)> {
        let (launch, outcome) = self
            .take_where(|run| !run.launch.foreground && matches!(run.state, RunState::Ended(_)))?;
        Some((launch, outcome.expect("an ended run has its outcome")))
    }

    /// `bid`'s run, if any, taken out as [`Self::take_where`] does.
    #[cfg(test)]
    pub(in crate::editor) fn take_buffer(&mut self, bid: BufferId) -> Option<Launch> {
        self.take_where(|run| run.launch.buffer == bid)
            .map(|(launch, _)| launch)
    }
}

impl super::async_source::AsyncSource for PipeRuns {
    /// The end of a background run's grace, when the statusline starts to
    /// show it, and at once for one whose processes are done.
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.runs
            .iter()
            .filter(|run| !run.launch.foreground)
            .filter_map(|run| match run.state {
                RunState::Ended(_) => Some(now),
                RunState::Running(_) => {
                    let shows = run.started + PROGRESS_GRACE;
                    (shows > now).then_some(shows)
                }
            })
            .min()
    }
}

/// A pipe run, as the statusline shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PipeProgress {
    pub(crate) command: String,
    /// Processes finished.
    pub(crate) done: usize,
    /// Processes in the run, one per selection.
    pub(crate) total: usize,
}

/// What a [`PipeConsole`] saw while the run waited.
#[derive(Debug)]
pub(in crate::editor) enum WaitEvent {
    /// `Ctrl-c`: cancel the run.
    Interrupt,
    /// The terminal changed size.
    Resized,
    /// A process finished, a wake carried no input, or the timeout passed.
    Woken,
}

/// Where a pipe run reads input and draws its progress. A console blocks on
/// the editor's waker, the one every pipe process fires when it finishes.
pub(in crate::editor) trait PipeConsole {
    /// Block until input arrives, the editor's waker fires, or `timeout`
    /// passes, if there is one.
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent>;
    /// Draw a frame of `ed` as it is now. Draws nothing by default.
    fn repaint(&mut self, _ed: &mut Editor) -> io::Result<()> {
        Ok(())
    }
    /// The input this console read and left to the run loop, oldest first.
    /// None by default.
    fn take_deferred(&mut self) -> Vec<TerminalEvent> {
        Vec::new()
    }
}

/// Reads the terminal for `Ctrl-c` and resizes and draws nothing: a run
/// during replay, and the input side of the run loop's console. It reads
/// every event and keeps the rest, in order, for the run loop: a filtered
/// `EventReader::poll` that a wake interrupts holds the events it skipped
/// out of the reader's queue until a later event arrives.
pub(in crate::editor) struct ReaderConsole {
    /// Woken by the editor's waker.
    pub(in crate::editor) reader: EventReader,
    pub(in crate::editor) deferred: Vec<TerminalEvent>,
}

impl PipeConsole for ReaderConsole {
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        if !self.reader.poll(timeout, |_| true)? {
            return Ok(WaitEvent::Woken);
        }
        let event = self.reader.read(|_| true)?;
        Ok(wait_event(&event).unwrap_or_else(|| {
            self.deferred.push(event);
            WaitEvent::Woken
        }))
    }

    fn take_deferred(&mut self) -> Vec<TerminalEvent> {
        std::mem::take(&mut self.deferred)
    }
}

/// The key that stops a pipe: it cancels a waited run, and `cancel-pipe`
/// is bound to it by default.
const STOP_KEY: (KeyCode, Modifiers) = (KeyCode::Char('c'), Modifiers::CONTROL);

/// [`STOP_KEY`] as a message names it.
pub(in crate::editor) const STOP_KEY_NAME: &str = "Ctrl-c";

/// What `event` means to a waiting run, `None` for an event it leaves to
/// the main loop.
pub(in crate::editor) fn wait_event(event: &TerminalEvent) -> Option<WaitEvent> {
    let (stop_code, stop_modifiers) = STOP_KEY;
    match event {
        TerminalEvent::Key(key)
            if key.kind != KeyEventKind::Release
                && key.code == stop_code
                && key.modifiers.contains(stop_modifiers) =>
        {
            Some(WaitEvent::Interrupt)
        }
        TerminalEvent::WindowResized(_) => Some(WaitEvent::Resized),
        _ => None,
    }
}

/// No terminal: a headless run waits for its processes and cannot be
/// cancelled. Every clone reads the same waker.
#[derive(Clone)]
pub(in crate::editor) struct Detached {
    rx: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl Detached {
    /// A waker, and the console its wakes reach. Wakes fired before a read
    /// count as one.
    pub(in crate::editor) fn pair() -> (WakeCallback, Self) {
        let (tx, rx) = mpsc::sync_channel(1);
        // A full channel already holds a wake; a closed one has no console
        // left to wake.
        let wake: WakeCallback = Arc::new(move || {
            let _ = tx.try_send(());
        });
        let console = Self {
            rx: Arc::new(Mutex::new(rx)),
        };
        (wake, console)
    }
}

impl PipeConsole for Detached {
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        const OPEN: &str = "the editor's waker keeps the channel open";
        let rx = self.rx.lock().unwrap_or_else(PoisonError::into_inner);
        match timeout {
            None => rx.recv().expect(OPEN),
            Some(timeout) => match rx.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => unreachable!("{OPEN}"),
            },
        }
        Ok(WaitEvent::Woken)
    }
}

/// How a pipe run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PipeEnd {
    /// The output replaced the selections or was inserted.
    Landed,
    /// A process failed, or the output could not land; reported.
    Failed,
    /// `Ctrl-c` or a termination request. A run whose console failed ends
    /// this way too.
    Cancelled,
}

/// How long a run goes before it draws its progress, so a quick one draws
/// no frame of its own.
const PROGRESS_GRACE: Duration = Duration::from_millis(100);

/// How a wait ended.
enum Waited {
    /// The run's processes are done; its outcome is in the registry.
    Ended,
    Cancelled,
}

impl EditorState {
    /// Start `command` once per selection of `pane` on `buffer`, holding the
    /// buffer's text until the run ends. A `Replace` run feeds each
    /// selection's text to its process; an `Insert` run feeds nothing.
    /// Nothing spawns until the run is polled. An `Insert` run, and any run
    /// started while a macro records, is waited for in place: the keys after
    /// it need its output, live as on replay. Any other run goes on in the
    /// background.
    ///
    /// # Panics
    /// Panics when `buffer`'s text may not change now.
    pub(in crate::editor) fn start_pipe_run(
        &mut self,
        pane: PaneId,
        buffer: BufferId,
        command: String,
        kind: PipeKind,
        settles: Option<DeferredChange>,
    ) -> RunId {
        let pane_state = self.panes.entries.seeded(pane, buffer);
        let selections = pane_state.selections().clone();
        let inputs: Vec<String> = {
            let view = pane_state.view(self.buffers.get(buffer).text());
            match kind {
                PipeKind::Replace => view.iter().map(|sel| String::from(sel.slice())).collect(),
                PipeKind::Insert => vec![String::new(); view.len()],
            }
        };
        let launch = Launch {
            buffer,
            pane,
            selections,
            input_ends_line: inputs.iter().map(|input| input.ends_with('\n')).collect(),
            kind,
            settles,
            command,
            foreground: kind == PipeKind::Insert || self.macros.recording_register().is_some(),
        };
        let from = hume_platform::process::Launch {
            cwd: &self.cwd,
            search_path: self.dirs.search_path.as_deref(),
        };
        let batch = ShellBatch::new(&self.settings.shell, &launch.command, from, inputs);
        let hold = self.buffers.get_mut(buffer).hold_text();
        self.pipe_runs.start(launch, hold, batch)
    }

    /// Land a run that ended with `outcome`, or report its failure and
    /// change nothing. The change the `pipe` started is kept only when the
    /// output lands.
    fn finish_pipe_run(&mut self, mut launch: Launch, outcome: Outcome) -> PipeEnd {
        let settles = launch.settles.take();
        let end = match outcome {
            Ok(outputs) => commands::apply_pipe_output(self, launch, outputs),
            Err(ShellFailure(msg)) => {
                let msg = match self.background_name(&launch) {
                    Some(name) => format!("pipe in {name}: {msg}"),
                    None => msg,
                };
                self.report(Severity::Error, msg);
                PipeEnd::Failed
            }
        };
        if let Some(change) = settles {
            let outcome = match end {
                PipeEnd::Landed => DeferredOutcome::Landed,
                PipeEnd::Failed | PipeEnd::Cancelled => DeferredOutcome::Withdrawn,
            };
            self.settle_deferred(change, outcome);
        }
        end
    }

    /// End a run taken out unfinished: report it cancelled and put back the
    /// change it replaced.
    fn withdraw_pipe_run(&mut self, launch: Launch) {
        let msg = match self.background_name(&launch) {
            Some(name) => format!("pipe in {name} cancelled"),
            None => "pipe cancelled".to_string(),
        };
        self.report(Severity::Info, msg);
        if let Some(change) = launch.settles {
            self.settle_deferred(change, DeferredOutcome::Withdrawn);
        }
    }

    /// The name a message about `launch`'s run gives its buffer: a
    /// background run can end while another buffer is in view. `None` for a
    /// run its starter waits for.
    fn background_name(&self, launch: &Launch) -> Option<String> {
        (!launch.foreground).then(|| self.buffers.get(launch.buffer).display_name())
    }

    /// Whether [`STOP_KEY`] runs `cancel-pipe` in `bid`'s Normal keymap, so
    /// a message may offer it as the way to stop `bid`'s run.
    pub(in crate::editor) fn stop_key_cancels_pipe(&self, bid: BufferId) -> bool {
        let (code, modifiers) = STOP_KEY;
        let resolved = self.config.local_keymaps.resolve(
            &self.config.keymap,
            bid,
            BindMode::Normal,
            &[KeyEvent::new(code, modifiers)],
        );
        matches!(resolved.walk, WalkResult::Leaf(cmd) if cmd.name == "cancel-pipe")
    }

    /// Cancel `bid`'s run, if one is in flight: kill its processes, change
    /// nothing, report it and put back the change it replaced. Whether there
    /// was one.
    pub(in crate::editor) fn cancel_pipe_run(&mut self, bid: BufferId) -> bool {
        let Some((launch, _)) = self.pipe_runs.take_where(|run| run.launch.buffer == bid) else {
            return false;
        };
        self.withdraw_pipe_run(launch);
        true
    }

    /// Withdraw every run started at or after `mark`.
    fn withdraw_pipe_runs_from(&mut self, mark: RunId) {
        while let Some((launch, _)) = self.pipe_runs.take_where(|run| run.id >= mark) {
            self.withdraw_pipe_run(launch);
        }
    }

    /// Poll the background runs and land, or report, each one whose
    /// processes are done.
    pub(in crate::editor) fn drain_pipe_runs(&mut self) {
        self.pipe_runs.poll(None, &self.wake);
        while let Some((launch, outcome)) = self.pipe_runs.take_ended_background() {
            self.finish_pipe_run(launch, outcome);
        }
    }
}

impl Editor {
    /// Run `act`; with `force`, stop each pipe run that refuses it and run it
    /// again: the `!` of a typed command a pipe run refuses, a close or `:e`.
    /// A refusal other than a pipe run, and any refusal without `force`,
    /// comes back as it is.
    pub(in crate::editor) fn run_stopping_pipes<R: PipeRefusal>(
        &mut self,
        force: bool,
        mut act: impl FnMut(&mut Self) -> Result<(), R>,
    ) -> Result<(), R> {
        loop {
            let refused = match act(self) {
                Err(refused) if force => refused,
                done => return done,
            };
            let Some(PipeRunning(bid)) = refused.pipe_running() else {
                return Err(refused);
            };
            let stopped = self.state.cancel_pipe_run(bid);
            assert!(stopped, "a command refused for a run that is not in flight");
        }
    }

    /// Wait for run `id` while `console` reads input, then land its output,
    /// or report its failure and change nothing. `Ctrl-c` and a termination
    /// request cancel it and kill its processes. A waited run is a
    /// foreground one from here on, whoever started it. The input `console`
    /// left to the run loop joins [`Editor::input_backlog`]. A run something
    /// else took out while it was waited for, as a drawer's render proc
    /// calling `(call! "cancel-pipe" pane)` does, ends `Cancelled`: whatever
    /// took it out reported it.
    ///
    /// `Err` only when `console` fails; the run is cancelled first.
    pub(in crate::editor) fn wait_pipe_run(
        &mut self,
        id: RunId,
        console: &mut dyn PipeConsole,
    ) -> io::Result<PipeEnd> {
        self.state.pipe_runs.claim(id);
        let waited = self.wait_until_ended(id, console);
        self.input_backlog.extend(console.take_deferred());
        let Some((launch, outcome)) = self.state.pipe_runs.take_where(|run| run.id == id) else {
            return waited.map(|_| PipeEnd::Cancelled);
        };
        match waited {
            Ok(Waited::Ended) => {
                let outcome = outcome.expect("an ended run has its outcome");
                Ok(self.state.finish_pipe_run(launch, outcome))
            }
            Ok(Waited::Cancelled) => {
                self.state.withdraw_pipe_run(launch);
                Ok(PipeEnd::Cancelled)
            }
            Err(e) => {
                self.state.withdraw_pipe_run(launch);
                Err(e)
            }
        }
    }

    /// Poll run `id`, and the background runs with it, until `id`'s
    /// processes are done. A run still going after [`PROGRESS_GRACE`] draws
    /// its progress, redraws it when it changes, at most once per
    /// [`FRAME_GAP`](super::frame::FRAME_GAP), and redraws it at once when the terminal is
    /// resized; a quicker one draws nothing.
    fn wait_until_ended(&mut self, id: RunId, console: &mut dyn PipeConsole) -> io::Result<Waited> {
        let mut shown = false;
        // Whether the progress differs from what the last frame showed.
        let mut stale = true;
        let mut pacer = FramePacer::not_before(Instant::now() + PROGRESS_GRACE);
        loop {
            if self.state.terminate_requested() {
                return Ok(Waited::Cancelled);
            }
            let Some(polled) = self.state.pipe_runs.poll(Some(id), &self.state.wake) else {
                return Ok(Waited::Cancelled);
            };
            let RunPoll::Running { changed } = polled else {
                return Ok(Waited::Ended);
            };
            stale |= changed;
            if stale && pacer.wait(Instant::now()).is_none() {
                console.repaint(self)?;
                shown = true;
                stale = false;
                pacer.drew(Instant::now());
            }
            let timeout = stale.then(|| pacer.wait(Instant::now()).unwrap_or_default());
            match console.next(timeout)? {
                WaitEvent::Interrupt => return Ok(Waited::Cancelled),
                WaitEvent::Resized if shown => {
                    stale = true;
                    pacer.input();
                }
                WaitEvent::Resized | WaitEvent::Woken => {}
            }
        }
    }

    /// Wait for every run started at or after `mark`, oldest first, so the
    /// key or command that started them is followed by their output. Draws
    /// no frames, so a run shows no progress; with a terminal attached
    /// `Ctrl-c` still cancels it, and the runs after it too. A cancel stops
    /// the replay that waits, as [`EditorState::stop_replays`] says.
    pub(in crate::editor) fn wait_pipe_runs_from(&mut self, mark: RunId) {
        if self.state.pipe_runs.oldest_from(mark).is_none() {
            return;
        }
        #[cfg(test)]
        if let Some(mut console) = self.inline_pipe_console.take() {
            self.wait_runs_from(mark, console.as_mut())
                .expect("a test console never fails");
            return;
        }
        let result = match self.tui.terminal().map(|term| term.event_reader()) {
            Some(reader) => self.wait_runs_from(
                mark,
                &mut ReaderConsole {
                    reader,
                    deferred: Vec::new(),
                },
            ),
            None => {
                let mut console = self
                    .detached
                    .clone()
                    .expect("an editor with no terminal to read wakes through its own channel");
                self.wait_runs_from(mark, &mut console)
            }
        };
        if let Err(e) = result {
            self.report(
                Severity::Error,
                format!("pipe: reading the terminal failed: {e}"),
            );
        }
    }

    /// [`Self::wait_pipe_runs_from`]'s waits on `console`. A cancelled run,
    /// or a console that failed, withdraws the runs after it and stops the
    /// replays.
    fn wait_runs_from(&mut self, mark: RunId, console: &mut dyn PipeConsole) -> io::Result<()> {
        while let Some(id) = self.state.pipe_runs.oldest_from(mark) {
            match self.wait_pipe_run(id, console) {
                Ok(PipeEnd::Landed | PipeEnd::Failed) => {}
                end => {
                    self.state.withdraw_pipe_runs_from(mark);
                    self.state.stop_replays();
                    return end.map(|_| ());
                }
            }
        }
        Ok(())
    }
}
