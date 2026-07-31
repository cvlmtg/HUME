use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::select::{FdSet, select};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::time::{TimeVal, TimeValLike};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook::flag::{register_conditional_shutdown, register_usize};
use signal_hook::low_level::pipe::register;

/// Probe for kitty keyboard protocol support on Unix.
///
/// Opens `/dev/tty` directly on its own side channel — independent of the
/// [`SharedTerm`](crate::terminal::SharedTerm) event reader, so probe replies
/// can never race or interleave with it — builds a [`super::TtyChannel`]
/// over it, and delegates the query/response loop to [`super::run_probe`].
///
/// Must be called after `enable_raw_mode()`.
pub(super) fn probe_kitty_support() -> io::Result<bool> {
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    super::run_probe(&mut TtyChannel { file: tty })
}

/// [`super::ProbeChannel`] backed by an open `/dev/tty` `File`, using
/// `poll(2)` to wait for input with a deadline.
struct TtyChannel {
    file: std::fs::File,
}

impl super::ProbeChannel for TtyChannel {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    fn wait_until(&mut self, deadline: Instant) -> io::Result<bool> {
        // `select(2)`, not `poll(2)`: macOS's `poll` does not report readiness
        // on `/dev/tty` — the same reason termina's own event source
        // (`event/source/unix.rs`) and this module's `wait_readable_pair`
        // both use `select` for terminal fds. EINTR retry and the
        // deadline-vs-remaining-budget recompute live in `wait_readable_pair`.
        let remaining = deadline.saturating_duration_since(Instant::now());
        wait_readable(self.file.as_fd(), Some(remaining))
    }
}

// ── Terminator: signals + terminal hangup ─────────────────────────────────
//
// Two independent reasons the process should tear down the terminal and
// exit, unified onto one thread:
//
// - SIGINT/SIGTERM/SIGHUP/SIGQUIT arrive normally. Delivered via a
//   `signal_hook` self-pipe — the same technique `termina` itself already
//   uses internally for SIGWINCH (`event/source/unix.rs`): the real signal
//   handler only writes one byte to a pipe, and this thread's `select`
//   treats that pipe exactly like any other fd it's waiting on.
// - A pty teardown (e.g. `vhs` closing the master after a recording) is not
//   guaranteed to deliver SIGHUP at all — hume is rarely the session leader
//   of the tty it runs under. Left undetected, the tty read fd sits at
//   permanent EOF, which termina maps to `Ok(None)` (not an error, not an
//   event), so `EventReader::poll`'s idle (no-timeout) wait spins inside
//   termina forever without ever returning to the editor's run loop. No fix
//   is possible from inside that loop, so this thread also watches
//   `/dev/tty` directly on its own independent fd, when one is available —
//   see below.
//
// Whichever fires first wins, but the two paths diverge after that: a
// signal can still ask the main loop to quit gracefully (its event reader
// is alive), while a hangup cannot (the reader is pinned at tty EOF and
// will never see a wake) and force-exits immediately. See `Trigger`. Once a
// signal has fired, the thread stops watching the tty for the rest of the
// grace window — a hangup arriving in that window is not observed, so the
// main loop can spin until the grace deadline force-exits it. Bounded (by
// `QUIT_GRACE`) and rare (both would have to happen within the same
// window), so left as-is rather than plumbed through as a third state.
//
// One invariant governs `spawn_terminator`'s setup order: a replaced signal
// disposition must exist only while something is able to act on it.
// `signal_hook` offers no way to restore a disposition once replaced —
// `unregister` removes a callback without touching `SIG_DFL`, so a signal
// that arrives afterward is silently swallowed forever, not delivered
// (documented in `signal-hook-registry`'s own source). That rules out
// "register, then unregister on failure" as a way to recover. Two
// mechanisms enforce the invariant instead: the draining thread is spawned
// *before* any disposition is replaced, so a spawn failure leaves the
// kernel's defaults untouched; and a `register_conditional_shutdown`
// fallback, armed the instant the thread stops draining (return or panic),
// covers the thread-dies-later case that ordering alone can't reach. The
// tty is watched on a best-effort basis throughout — opening it, and every
// I/O error on it afterward, degrades to "no hangup watch" rather than
// touching signal service, since a process with no controlling terminal
// has nothing to lose there but must still be killable.

/// Signals that ask the process to terminate. SIGQUIT is included so `kill
/// -QUIT` restores the terminal (raw mode, alt screen) before exiting,
/// trading away the default core dump — nothing here relies on one.
const SIGNALS: [i32; 4] = [SIGINT, SIGTERM, SIGHUP, SIGQUIT];

/// How long to sleep after observing real pending input on the tty before
/// re-checking hangup status. Long enough that we don't spin while a
/// keystroke sits in the shared tty queue waiting for the main loop to read
/// it; short enough that a hangup right after a keystroke is still caught
/// quickly.
const INPUT_THROTTLE: Duration = Duration::from_millis(100);

/// Consecutive zero-timeout confirmations required before a
/// readable-with-zero-bytes tty fd is treated as a genuine hangup rather
/// than a transient drain race with termina's own reader on the same tty
/// queue.
const CONFIRMATIONS: u32 = 3;

/// Delay between confirmation checks.
const CONFIRMATION_DELAY: Duration = Duration::from_millis(20);

/// Which of the terminator's two wake sources fired.
#[derive(Debug, PartialEq, Eq)]
enum Trigger {
    /// One of [`SIGNALS`] arrived. The main loop can still be asked to quit
    /// gracefully — its event reader is alive and will see the wake.
    Signal,
    /// The controlling terminal hung up. The main loop's event reader is
    /// pinned at tty EOF and will never observe a wake, so there is no
    /// graceful route: the caller must force-exit directly.
    Hangup,
}

/// This crate's exit code before any of the exit-fidelity tracking here
/// existed — a fixed 130 (`SIGINT`'s own `128 + signo`), used today as the
/// fallback when there's no real signal number to derive one from: the
/// zero/unknown-signal case in [`exit_code_for_signal`], and the pty-hangup
/// case (a hangup is not a signal at all, so it has no `signo` to map).
const CONVENTIONAL_EXIT_CODE: i32 = 130;

/// Maps a signal number to the conventional "killed by signal" exit code
/// (`128 + signo`). `0` — no signal recorded yet on `signal_flag` — falls
/// back to [`CONVENTIONAL_EXIT_CODE`].
fn exit_code_for_signal(signo: usize) -> i32 {
    if signo == 0 {
        CONVENTIONAL_EXIT_CODE
    } else {
        128 + signo as i32
    }
}

/// Arms the `register_conditional_shutdown` fallback the instant the
/// terminator thread stops draining the signal pipe — a return or a panic.
/// Held as the thread body's first binding so every exit path, including an
/// unwind, runs `Drop` before the thread is gone. The two paths that
/// terminate the process directly (`force_exit`, via `process::exit`) never
/// reach it — `process::exit` runs no destructors, so a signal that
/// triggers a clean force-exit or hangup never needlessly flips this flag.
struct OrphanGuard(Arc<AtomicBool>);

impl Drop for OrphanGuard {
    fn drop(&mut self) {
        // SeqCst to match the load `register_conditional_shutdown`'s
        // handler performs on the same flag.
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Spawn a detached thread that terminates the process on one of [`SIGNALS`],
/// or on the controlling terminal hanging up with no signal delivered at all.
///
/// Opens its own `/dev/tty` fd — independent of the
/// [`SharedTerm`](crate::terminal::SharedTerm) event reader used by the main
/// loop — so hangup detection never consumes a byte of real input (same
/// independence as [`probe_kitty_support`]'s side channel). The open is
/// best-effort: a process with no controlling terminal (a `setsid` wrapper,
/// some CI/pty harnesses, containers) gets signal handling with no hangup
/// watch rather than losing both.
///
/// `request_quit` is called with the exit code the process should use —
/// `128 + signo` for whichever of [`SIGNALS`] fired. The thread then waits up
/// to [`crate::QUIT_GRACE`] for the main loop to exit the process on its own
/// (graceful LSP shutdown) before force-exiting with the same code — or with
/// a second signal's code, if one arrives inside the window. A hangup
/// force-exits with `130` immediately — see [`Trigger::Hangup`]. If the
/// thread itself is lost (spawn failure, or a later permanent I/O error), a
/// `register_conditional_shutdown` fallback still terminates the process on
/// the next signal, without a graceful LSP shutdown or terminal restore —
/// see the module-level comment above for why this is the best available
/// fallback under `signal_hook`'s no-`unsafe` API.
pub(super) fn spawn_terminator(
    term: crate::terminal::SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> io::Result<()> {
    // A process with no controlling terminal cannot receive a pty hangup, so
    // there is nothing here to lose — but it can still be signalled, and
    // that must keep working regardless. Never let this failure cost us
    // signal handling (see the module-level comment above).
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok();

    // Self-pipe: the actual signal handlers (installed by `register`,
    // async-signal-safe) only write a byte here; all the real work — acting
    // on it, or not, exactly once — happens on the thread below, under no
    // signal-handler restrictions. Non-blocking so the thread can fully
    // drain it (see `drain_signal_pipe`) instead of a single bounded read
    // that could leave a queued byte from a second signal unread.
    let (sig_read, sig_write) = UnixStream::pair()?;
    sig_read.set_nonblocking(true)?;

    // Sees whichever of `SIGNALS` last ran its flag-set action — read after
    // draining the pipe to turn "a signal happened" into "which one", for
    // `exit_code_for_signal`. Shared across all four registrations: if two
    // land close together this only tells us the most recent, which is fine
    // — it's read for the process's exit status, not to attribute causality.
    let signal_flag = Arc::new(AtomicUsize::new(0));
    // Set by `OrphanGuard` once this thread can no longer drain the pipe;
    // read by every signal's `register_conditional_shutdown` fallback below.
    let orphaned = Arc::new(AtomicBool::new(false));

    // The thread is spawned *before* any signal disposition is touched, so a
    // `Builder::spawn` failure returns `Err` with the kernel's defaults
    // completely untouched — no disposition is ever replaced with nothing
    // able to act on it.
    std::thread::Builder::new()
        .name("hume-terminator".into())
        .spawn({
            let signal_flag = Arc::clone(&signal_flag);
            let orphaned = Arc::clone(&orphaned);
            move || {
                // A handler that's installed but masked never runs — the same
                // "no one can act on this" hazard the registration order below
                // exists to avoid. neovim's `signal_init()` clears the mask
                // process-wide for the same reason; scoped to this thread only,
                // so LSP children are unaffected — `std::process::Command`
                // inherits the *spawning* thread's mask rather than resetting
                // it before `exec`.
                let mask: SigSet = SIGNALS
                    .iter()
                    .filter_map(|&s| Signal::try_from(s).ok())
                    .collect();
                let _ = mask.thread_unblock();

                let _guard = OrphanGuard(orphaned);
                match run_terminator_blocking(
                    tty.as_ref().map(std::fs::File::as_fd),
                    sig_read.as_fd(),
                    INPUT_THROTTLE,
                ) {
                    Ok(Trigger::Signal) => {
                        let code = exit_code_for_signal(signal_flag.load(Ordering::Acquire));
                        request_quit(code);
                        // A second signal during the grace window cuts the wait
                        // short instead of forcing the user to wait it out, and its
                        // own code (if different) wins.
                        let code = wait_for_second_signal(
                            sig_read.as_fd(),
                            &signal_flag,
                            crate::QUIT_GRACE,
                        )
                        .unwrap_or(code);
                        crate::force_exit(&term, code);
                    }
                    Ok(Trigger::Hangup) => crate::force_exit(&term, CONVENTIONAL_EXIT_CODE),
                    // A permanent I/O failure means termination coverage is
                    // lost; exit this thread quietly and let `OrphanGuard`'s
                    // fallback cover future signals instead.
                    Err(_) => {}
                }
            }
        })?;

    for &signal in &SIGNALS {
        // The conditional-shutdown fallback is registered first so it
        // short-circuits ahead of the flag/pipe actions below once
        // `orphaned` is set — signal-hook-registry runs a signal's actions
        // in registration order. It terminates the process directly
        // (`libc::_exit`, async-signal-safe) with the same `128 + signo`
        // code the normal path would have used, once this thread is
        // confirmed gone. `register_conditional_default` (re-raise with
        // `SIG_DFL`) is the wrong tool here — it would restore SIGQUIT's
        // core dump, which `SIGNALS`' own doc comment deliberately trades
        // away.
        register_conditional_shutdown(
            signal,
            exit_code_for_signal(signal as usize),
            Arc::clone(&orphaned),
        )?;
        // Registration order matters within one signal's own action list —
        // the flag must be set before the pipe byte is written, so a reader
        // woken by the pipe never observes a stale flag (signal-hook's
        // documented self-pipe ordering rule).
        register_usize(signal, Arc::clone(&signal_flag), signal as usize)?;
        // `register` takes ownership of the fd it's given (closing it on
        // deregistration), so each signal needs its own dup — handing the
        // same fd to multiple registrations would leave `sig_write`'s `Drop`
        // racing signal-hook's internal close of that same descriptor
        // number, and a later `open`/`socket` reusing it while a queued
        // signal still points at it.
        register(signal, sig_write.try_clone()?)?;
    }
    drop(sig_write);

    Ok(())
}

/// Blocks until either a registered signal fires or the terminal at
/// `tty_fd` hangs up, then returns which one. `tty_fd` is `None` when no
/// controlling terminal was available to watch (see [`spawn_terminator`]) —
/// signals alone are served in that case. Returns `Err` only when the signal
/// pipe itself is permanently gone (every write end closed) or the initial
/// wait on it fails; a broken `tty_fd` degrades to `None` instead of
/// failing, since losing the hangup watch must never cost signal service.
/// Never touches the process — kept separate from [`spawn_terminator`] so it
/// can be driven directly in tests. `input_throttle` is [`INPUT_THROTTLE`] in
/// production; tests that assert on how fast a signal interrupts it inject a
/// much larger value instead, so their pass/fail margin isn't pinned to the
/// same constant the timing assertion is checking.
fn run_terminator_blocking(
    mut tty_fd: Option<BorrowedFd<'_>>,
    sig_fd: BorrowedFd<'_>,
    input_throttle: Duration,
) -> io::Result<Trigger> {
    loop {
        let (tty_ready, sig_ready) = match tty_fd {
            Some(tty) => match wait_readable_pair(tty, sig_fd, None) {
                Ok(pair) => pair,
                // `select` reports one errno for the whole set and can't say
                // which fd caused it. The tty is the fd that can genuinely
                // go away underneath us; drop it and keep serving signals —
                // a repeat failure on the signal-only wait below is a real
                // permanent failure.
                Err(_) => {
                    tty_fd = None;
                    continue;
                }
            },
            None => (false, wait_readable(sig_fd, None)?),
        };
        if sig_ready {
            // `select` reporting readable doesn't guarantee bytes are still
            // there to read by the time we get to it (a concurrent read, or
            // a spurious wakeup, could have emptied it first) — only an
            // actual drained byte counts as a real signal; otherwise loop
            // back and keep waiting, the same "don't trust readable alone"
            // posture `hangup_status` takes on the tty fd below.
            match drain_signal_pipe(sig_fd) {
                Drained::Signal => return Ok(Trigger::Signal),
                Drained::Empty => continue,
                // Every write end is gone — no handler can ever wake this
                // thread again. A permanent failure, not a spin: the caller
                // exits and `OrphanGuard`'s fallback takes over.
                Drained::Closed => return Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            }
        }
        if let Some(tty) = tty_fd
            && tty_ready
        {
            match hangup_status(tty) {
                // A plain sleep here would blind the signal fd for the
                // whole throttle window — while the user types continuously
                // that's a chain of blind windows, delaying every signal.
                // Wait on `sig_fd` instead: a signal arriving mid-throttle
                // wakes us immediately and loops back to the `sig_ready`
                // drain above; the throttle still elapses before we look at
                // the tty again either way.
                Ok(Status::Input) => {
                    let _ = wait_readable(sig_fd, Some(input_throttle));
                }
                Ok(Status::Live) => {}
                Ok(Status::Hangup) => return Ok(Trigger::Hangup),
                // A broken watcher fd is not a hangup — drop it and keep
                // serving signals.
                Err(_) => tty_fd = None,
            }
        }
    }
}

/// Outcome of draining the signal pipe.
#[derive(Debug, PartialEq, Eq)]
enum Drained {
    /// At least one handler's byte was read — a real signal.
    Signal,
    /// Readable with nothing left to read: `select` can report readable
    /// after a concurrent drain or a spurious wakeup.
    Empty,
    /// Every write end is gone (`read` hit EOF before any byte arrived): no
    /// handler can ever wake this thread again.
    Closed,
}

/// Drains every byte currently queued on the (non-blocking) signal pipe,
/// reporting which of [`Drained`]'s cases the drain turned out to be.
fn drain_signal_pipe(fd: BorrowedFd<'_>) -> Drained {
    let mut buf = [0u8; 64];
    let mut any = false;
    loop {
        match nix::unistd::read(fd, &mut buf) {
            Ok(0) => {
                return if any {
                    Drained::Signal
                } else {
                    Drained::Closed
                };
            }
            Ok(_) => any = true,
            Err(Errno::EINTR) => continue,
            // EWOULDBLOCK (drained) or a permanent fd error either way.
            Err(_) => return if any { Drained::Signal } else { Drained::Empty },
        }
    }
}

/// Waits out the remainder of `grace` for a second signal to arrive on
/// `sig_fd`, returning the exit code mapped from whichever signal `flag`
/// then holds, or `None` if the window elapses first with nothing new.
fn wait_for_second_signal(
    sig_fd: BorrowedFd<'_>,
    flag: &AtomicUsize,
    grace: Duration,
) -> Option<i32> {
    let deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match wait_readable(sig_fd, Some(remaining)) {
            Ok(true) => match drain_signal_pipe(sig_fd) {
                Drained::Signal => {
                    return Some(exit_code_for_signal(flag.load(Ordering::Acquire)));
                }
                // A readable report with nothing actually drained is a
                // spurious wakeup, same reasoning as
                // `run_terminator_blocking`'s sig_ready branch — loop back
                // and recompute the remaining budget.
                Drained::Empty => continue,
                // A closed pipe reads ready forever; sleep out the rest of
                // the window instead of busy-selecting against it.
                Drained::Closed => {
                    std::thread::sleep(remaining);
                    return None;
                }
            },
            Ok(false) => continue,
            Err(_) => return None,
        }
    }
}

/// What a readable `fd` turned out to mean.
#[derive(Debug, PartialEq, Eq)]
enum Status {
    /// Real bytes are waiting — not a hangup.
    Input,
    /// Momentarily readable-with-zero-bytes because termina's own reader
    /// drained the shared tty queue between our `select` and our
    /// `FIONREAD` — not a hangup.
    Live,
    /// Confirmed: the terminal hung up.
    Hangup,
}

/// Whether `err` genuinely means the controlling terminal itself is gone, as
/// opposed to a fault in this watcher's own descriptor. `EIO` is the
/// documented signal a Linux pty slave surfaces after its master closes;
/// `ENXIO` covers losing the device out from under the fd on platforms that
/// don't surface `EIO` the same way. `EBADF` is deliberately excluded: it
/// means *this* fd is invalid — a double-close or fd-stealing bug in this
/// process, never evidence the terminal hung up — so it takes the `Err` path
/// below and drops the tty watch instead of force-exiting on a fault that
/// isn't the terminal's. Anything else (`ENOTTY`, `EAGAIN`, ...) is the same:
/// something is wrong with this fd, not that the terminal hung up.
fn is_tty_gone(err: rustix::io::Errno) -> bool {
    use rustix::io::Errno as TtyErrno;
    matches!(err, TtyErrno::IO | TtyErrno::NXIO)
}

/// Outcome of one `FIONREAD` probe on `fd`: empty (nothing queued), has data
/// waiting, or a confirmed hangup per [`is_tty_gone`]. Shared by
/// [`hangup_status`] and [`confirm_hangup`] — the one place that classifies
/// a `retry_on_intr`/`ioctl_fionread` result, so the two callers can never
/// disagree on what a given errno means.
fn fionread_outcome(fd: BorrowedFd<'_>) -> io::Result<FionreadOutcome> {
    match rustix::io::retry_on_intr(|| rustix::io::ioctl_fionread(fd)) {
        Ok(0) => Ok(FionreadOutcome::Empty),
        Ok(_) => Ok(FionreadOutcome::HasData),
        Err(e) if is_tty_gone(e) => Ok(FionreadOutcome::Gone),
        Err(e) => Err(e.into()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FionreadOutcome {
    Empty,
    HasData,
    Gone,
}

/// Classifies a readable `fd`.
///
/// `EINTR` — plausible here, since four signal handlers plus termina's own
/// `SIGWINCH` handler are live in this process — is retried rather than
/// misread as a hangup (inside [`fionread_outcome`]'s `retry_on_intr`). Only
/// a confirmed [`FionreadOutcome::Gone`] is treated as a hangup; anything
/// else propagates as `Err` so the caller (this watcher's own wait loop)
/// drops the tty from its wait set and keeps serving signals, instead of
/// force-exiting on a transient fault that was never proof the terminal went
/// away.
fn hangup_status(fd: BorrowedFd<'_>) -> io::Result<Status> {
    match fionread_outcome(fd)? {
        FionreadOutcome::Empty => Ok(if confirm_hangup(fd)? {
            Status::Hangup
        } else {
            Status::Live
        }),
        FionreadOutcome::HasData => Ok(Status::Input),
        FionreadOutcome::Gone => Ok(Status::Hangup),
    }
}

/// Distinguishes a genuine hangup from a momentary drain race.
///
/// The watcher's fd and termina's input fd share one kernel tty queue, so a
/// single readable-with-zero-bytes observation is ambiguous: it's what a
/// hangup looks like, but it's also what a live fd looks like for the
/// instant between termina consuming a byte and our `FIONREAD` call. Only a
/// hangup stays that way; re-check a few times with a zero timeout and treat
/// any check that finds the fd not-readable, or newly non-empty, as proof it
/// was a drain.
fn confirm_hangup(fd: BorrowedFd<'_>) -> io::Result<bool> {
    for _ in 0..CONFIRMATIONS {
        std::thread::sleep(CONFIRMATION_DELAY);
        if !wait_readable(fd, Some(Duration::ZERO))? {
            return Ok(false);
        }
        match fionread_outcome(fd)? {
            FionreadOutcome::Empty => {}
            FionreadOutcome::HasData => return Ok(false),
            FionreadOutcome::Gone => return Ok(true),
        }
    }
    Ok(true)
}

/// `select(2)`-based readiness wait for a single `fd`. A thin wrapper over
/// [`wait_readable_pair`] (passing `fd` as both members — `BorrowedFd` is
/// `Copy`, and inserting the same fd twice into an `FdSet` is harmless) so
/// there's one retry/rebuild loop for both the single- and dual-fd cases.
fn wait_readable(fd: BorrowedFd<'_>, timeout: Option<Duration>) -> io::Result<bool> {
    wait_readable_pair(fd, fd, timeout).map(|(ready, _)| ready)
}

/// `select(2)`-based readiness wait for two fds at once — the terminator
/// thread's core primitive, so a signal is never delayed behind an idle tty
/// wait. `select`, not `poll(2)` — macOS's `poll` does not report readiness
/// on `/dev/tty`, which is why termina itself (`event/source/unix.rs`) uses
/// `select` for the terminal's main input fd, and why [`TtyChannel`] above
/// goes through [`wait_readable`] instead of calling `poll` directly; this
/// matches that choice for the same fd family. `timeout:
/// None` blocks indefinitely. Returns which of `a`/`b` are readable.
fn wait_readable_pair(
    a: BorrowedFd<'_>,
    b: BorrowedFd<'_>,
    timeout: Option<Duration>,
) -> io::Result<(bool, bool)> {
    // Deadline computed once, up front, from the caller's relative budget.
    let deadline = timeout.map(|d| Instant::now() + d);
    loop {
        let mut set = FdSet::new();
        set.insert(a);
        set.insert(b);
        // Recomputed against `deadline` on every pass, including after
        // `EINTR` — reusing the original relative `timeout` on each retry
        // would let a burst of signals (e.g. repeated SIGWINCH) stretch the
        // wait arbitrarily past what the caller asked for. `saturating_`
        // clamps to zero rather than skipping the call: a just-elapsed
        // deadline still gets one real non-blocking `select`, matching
        // `Duration::ZERO`'s poll-once semantics instead of returning a
        // false negative without ever checking.
        let mut timeout = deadline.map(|dl| {
            TimeVal::milliseconds(dl.saturating_duration_since(Instant::now()).as_millis() as i64)
        });
        match select(None, &mut set, None, None, timeout.as_mut()) {
            Ok(_) => return Ok((set.contains(a), set.contains(b))),
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::from(e)),
        }
    }
}

#[cfg(test)]
mod terminator_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    // `UnixStream::pair()` reproduces the primitives `run_terminator_blocking`
    // observes without needing a real pty or a real signal: dropping one end
    // makes the peer select-readable with `FIONREAD == 0` — the same shape a
    // master-closed tty presents to `select`/`FIONREAD` — and writing a byte
    // to one end stands in for a `signal_hook` self-pipe write, since we're
    // only testing this module's own multiplexing logic, not signal-hook's
    // (separately tested, upstream) job of relaying a real signal to a fd.

    /// An inert stand-in for the signal self-pipe: never written to, so it
    /// never reports readable. Keeps both ends alive so it can't spuriously
    /// look hung-up either. The read end is non-blocking, matching
    /// `spawn_terminator`'s real `sig_read` — `drain_signal_pipe` reads it to
    /// `WouldBlock`, which would hang forever on a blocking fd once emptied.
    fn idle_sig_pipe() -> (UnixStream, UnixStream) {
        let (sig_read, sig_write) = UnixStream::pair().expect("socketpair");
        sig_read.set_nonblocking(true).expect("set_nonblocking");
        (sig_read, sig_write)
    }

    /// A regression to the drain-less spin `terminator_exits_instead_of_spinning_when_the_pipe_closes`
    /// guards against — a `run_terminator_blocking` call that never returns —
    /// is a real failure mode for this module (observed directly: a sabotage
    /// run of `detects_signal_with_tty_idle` sat at 100% CPU for three days
    /// before being mistaken for a live bug). A direct call on the test
    /// thread would hang `cargo test` forever on that regression instead of
    /// failing it. This runs the call on its own thread and fails the test if
    /// it hasn't returned within `bound`, so a spin becomes a fast, visible
    /// test failure. Takes the fds by value — they must outlive the spawned
    /// thread — while each test keeps its own peer/writer handle so it can
    /// still act on the connection after the call starts.
    fn run_bounded(
        tty: Option<UnixStream>,
        sig_read: UnixStream,
        input_throttle: Duration,
        bound: Duration,
    ) -> Trigger {
        let handle = std::thread::spawn(move || {
            run_terminator_blocking(
                tty.as_ref().map(UnixStream::as_fd),
                sig_read.as_fd(),
                input_throttle,
            )
        });
        let deadline = Instant::now() + bound;
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "run_terminator_blocking did not return within {bound:?} — \
                 regression to the drain-less spin this module was built to avoid"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle
            .join()
            .expect("terminator thread panicked")
            .expect("run_terminator_blocking returned an error")
    }

    /// Hang-detector bound for [`run_bounded`] — generous on purpose since it
    /// only needs to catch a genuine spin, not assert on latency; the timing
    /// assertions in this module (e.g. `INPUT_THROTTLE`, `CONFIRMATIONS *
    /// CONFIRMATION_DELAY`) already cover how fast a success must be.
    const SPIN_BOUND: Duration = Duration::from_secs(2);

    #[test]
    fn detects_hangup_when_peer_closes() {
        let (fd, peer) = UnixStream::pair().expect("socketpair");
        drop(peer);
        let (sig_read, _sig_write) = idle_sig_pipe();
        assert_eq!(
            run_bounded(Some(fd), sig_read, INPUT_THROTTLE, SPIN_BOUND),
            Trigger::Hangup,
            "hangup must be detected once peer closes"
        );
    }

    #[test]
    fn detects_signal_with_tty_idle() {
        let (tty_fd, _tty_peer) = UnixStream::pair().expect("socketpair");
        let (sig_read, mut sig_write) = idle_sig_pipe();
        sig_write.write_all(b"x").expect("write");
        assert_eq!(
            run_bounded(Some(tty_fd), sig_read, INPUT_THROTTLE, SPIN_BOUND),
            Trigger::Signal,
            "signal must be detected while the tty is idle and still open"
        );
    }

    /// With no controlling terminal at all (`tty_fd` `None`, matching
    /// `spawn_terminator`'s `/dev/tty` open failing and degrading instead of
    /// propagating), a signal must still be served.
    #[test]
    fn terminator_serves_signals_without_a_tty() {
        let (sig_read, mut sig_write) = idle_sig_pipe();
        sig_write.write_all(b"x").expect("write");
        assert_eq!(
            run_bounded(None, sig_read, INPUT_THROTTLE, SPIN_BOUND),
            Trigger::Signal,
            "signal must be detected with no tty to watch"
        );
    }

    /// Regression test for the spin the actor-before-disposition reorder
    /// would otherwise introduce: once every write end of the signal pipe is
    /// gone, the pipe reads EOF-ready forever, and a `bool`-returning drain
    /// that treated "nothing read" as "keep waiting" would burn 100% CPU
    /// instead of ever returning. Bounded by a timeout so a regression fails
    /// the test rather than hanging the suite.
    #[test]
    fn terminator_exits_instead_of_spinning_when_the_pipe_closes() {
        let (tty_fd, _tty_peer) = UnixStream::pair().expect("socketpair");
        let (sig_read, sig_write) = idle_sig_pipe();
        drop(sig_write);

        let handle = std::thread::spawn(move || {
            run_terminator_blocking(Some(tty_fd.as_fd()), sig_read.as_fd(), INPUT_THROTTLE)
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            handle.is_finished(),
            "a permanently closed signal pipe must return promptly, not spin forever"
        );
        assert!(
            handle.join().expect("thread panicked").is_err(),
            "a permanently closed signal pipe is a real failure, not a trigger"
        );
    }

    /// Validity check: with the peer still open, hangup must not be
    /// detected. Without this, `detects_hangup_when_peer_closes` above could
    /// pass even if `run_terminator_blocking` returned `Ok(Trigger::Hangup)`
    /// unconditionally.
    #[test]
    fn does_not_report_hangup_while_peer_is_open() {
        let (fd, _peer) = UnixStream::pair().expect("socketpair");
        assert!(
            !wait_readable(fd.as_fd(), Some(Duration::ZERO)).expect("select"),
            "an idle, still-open pair must never be readable"
        );
        assert!(!confirm_hangup(fd.as_fd()).expect("fionread"));
    }

    #[test]
    fn pending_input_is_not_mistaken_for_hangup() {
        let (fd, mut peer) = UnixStream::pair().expect("socketpair");
        peer.write_all(b"x").expect("write");

        assert!(wait_readable(fd.as_fd(), Some(Duration::ZERO)).expect("select"));
        assert_eq!(hangup_status(fd.as_fd()).expect("status"), Status::Input);
    }

    #[test]
    fn hangup_status_classifies_input_and_hangup() {
        // Input: unread byte present, peer still open.
        let (fd, mut peer) = UnixStream::pair().expect("socketpair");
        peer.write_all(b"x").expect("write");
        assert_eq!(hangup_status(fd.as_fd()).expect("status"), Status::Input);

        // Hangup: peer closed, nothing unread.
        let (fd2, peer2) = UnixStream::pair().expect("socketpair");
        drop(peer2);
        assert_eq!(hangup_status(fd2.as_fd()).expect("status"), Status::Hangup);
    }

    /// `is_tty_gone` is the only place that decides which `FIONREAD` errnos
    /// mean a real hangup; a real `ENXIO` can't be produced from safe code
    /// (no dangling `BorrowedFd` without `unsafe`), so this is the direct
    /// test for that decision. Independent oracle: each assertion is keyed
    /// to the documented hangup set (`EIO`/`ENXIO`) rather than to whatever
    /// the implementation currently does, so a version that over- or
    /// under-classifies fails it either way. `EBADF` asserts `false`
    /// deliberately: it means this fd is invalid, never that the terminal
    /// hung up, so it must take the `Err` path instead of force-exiting.
    #[test]
    fn is_tty_gone_classifies_the_documented_errnos() {
        use rustix::io::Errno;
        assert!(is_tty_gone(Errno::IO));
        assert!(is_tty_gone(Errno::NXIO));
        assert!(!is_tty_gone(Errno::BADF));
        assert!(!is_tty_gone(Errno::INTR));
        assert!(!is_tty_gone(Errno::AGAIN));
        assert!(!is_tty_gone(Errno::NOTTY));
    }

    /// Reproduces the actual race `confirm_hangup` guards against: our fd and
    /// a second fd over the *same* socket (standing in for termina's own
    /// reader on the shared tty queue) both wake on the same byte; the
    /// background thread races to drain it first. If it wins, our `select`
    /// can still observe readable-with-zero-bytes for an instant — but the
    /// peer never closes, so `hangup_status` must never call that `Hangup`.
    ///
    /// Exercises `hangup_status` directly (not `confirm_hangup` in
    /// isolation) so it proves the guard is wired into the real decision
    /// path: a version of `hangup_status` that skipped confirmation (`Ok(0)
    /// => Ok(Status::Hangup)` unconditionally) fails this test.
    #[test]
    fn concurrent_drain_is_never_mistaken_for_hangup() {
        let (fd, mut peer) = UnixStream::pair().expect("socketpair");
        let drainer_fd = fd.try_clone().expect("clone");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let drainer = std::thread::spawn(move || {
            let mut buf = [0u8; 1];
            while !stop_flag.load(Ordering::Relaxed) {
                if wait_readable(drainer_fd.as_fd(), Some(Duration::from_millis(2)))
                    .unwrap_or(false)
                {
                    let _ = (&drainer_fd).read(&mut buf);
                }
            }
        });

        for _ in 0..100 {
            peer.write_all(b"x").expect("write");
            if wait_readable(fd.as_fd(), Some(Duration::from_millis(5))).unwrap_or(false) {
                let status = hangup_status(fd.as_fd()).expect("status");
                assert_ne!(
                    status,
                    Status::Hangup,
                    "peer is still open — a race with the concurrent drainer must never be reported as hangup"
                );
            }
        }

        stop.store(true, Ordering::Relaxed);
        drop(peer); // unblocks the drainer's final wait_readable via EOF
        drainer.join().expect("drainer thread panicked");
    }

    #[test]
    fn wait_readable_pair_classifies_neither_either_and_both() {
        let (a, mut a_peer) = UnixStream::pair().expect("socketpair");
        let (b, mut b_peer) = UnixStream::pair().expect("socketpair");

        assert_eq!(
            wait_readable_pair(a.as_fd(), b.as_fd(), Some(Duration::ZERO)).expect("select"),
            (false, false),
        );

        a_peer.write_all(b"x").expect("write");
        assert_eq!(
            wait_readable_pair(a.as_fd(), b.as_fd(), Some(Duration::ZERO)).expect("select"),
            (true, false),
        );

        b_peer.write_all(b"y").expect("write");
        assert_eq!(
            wait_readable_pair(a.as_fd(), b.as_fd(), Some(Duration::ZERO)).expect("select"),
            (true, true),
        );
    }

    /// The signal path must win immediately, without paying the tty-hangup
    /// path's `CONFIRMATIONS * CONFIRMATION_DELAY` confirmation cost — a
    /// version that ran hangup-style confirmation on the signal fd too would
    /// still pass `detects_signal_with_tty_idle` (just slower), so this
    /// bounds the time.
    #[test]
    fn signal_is_detected_without_hangup_confirmation_delay() {
        let (tty_fd, _tty_peer) = UnixStream::pair().expect("socketpair");
        let (sig_read, mut sig_write) = idle_sig_pipe();
        sig_write.write_all(b"x").expect("write");

        let start = std::time::Instant::now();
        assert_eq!(
            run_bounded(Some(tty_fd), sig_read, INPUT_THROTTLE, SPIN_BOUND),
            Trigger::Signal,
            "signal detected"
        );
        let confirmation_cost = CONFIRMATION_DELAY * CONFIRMATIONS;
        assert!(
            start.elapsed() < confirmation_cost,
            "signal detection took {:?}, as long as paying the hangup path's confirmation delay ({:?}) — it should return immediately",
            start.elapsed(),
            confirmation_cost
        );
    }

    /// A signal arriving while the tty is continuously readable (real
    /// typing) must not wait out the throttle — the `Status::Input` arm must
    /// stay watching `sig_fd`, not blind-sleep. A plain
    /// `thread::sleep(input_throttle)` there would still pass every other
    /// test in this module (none keep the tty readable across the wait) but
    /// fails this one on timing.
    ///
    /// Injects a throttle far larger than production's `INPUT_THROTTLE`
    /// instead of using it directly: a blind-sleep regression then takes
    /// seconds, not ~100ms, so the elapsed-time assertion below can use a
    /// wide margin without losing the ability to catch it — a margin pinned
    /// to `INPUT_THROTTLE` itself left only ~90ms of slack for scheduling
    /// jitter on a loaded CI runner, which one run ate through.
    #[test]
    fn signal_during_continuous_tty_input_is_not_delayed_by_the_throttle() {
        const TEST_THROTTLE: Duration = Duration::from_secs(1);

        let (tty_fd, mut tty_peer) = UnixStream::pair().expect("socketpair");
        // Keep the tty permanently readable: write and never drain, so
        // every `hangup_status` call sees `Status::Input` again.
        tty_peer.write_all(b"x").expect("write");
        let (sig_read, mut sig_write) = idle_sig_pipe();

        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            sig_write.write_all(b"x").expect("write");
        });

        let start = Instant::now();
        assert_eq!(
            run_bounded(Some(tty_fd), sig_read, TEST_THROTTLE, SPIN_BOUND),
            Trigger::Signal,
            "signal detected despite continuous tty input"
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed < TEST_THROTTLE / 2,
            "signal took {elapsed:?}, at least half the injected throttle ({TEST_THROTTLE:?}) — \
             the throttle wait must be interruptible by a signal, not a blind sleep",
        );
        writer.join().expect("writer thread panicked");
    }

    #[test]
    fn exit_code_for_signal_maps_128_plus_signo_with_zero_fallback() {
        assert_eq!(exit_code_for_signal(SIGINT as usize), 130);
        assert_eq!(exit_code_for_signal(SIGTERM as usize), 143);
        assert_eq!(exit_code_for_signal(SIGHUP as usize), 129);
        assert_eq!(exit_code_for_signal(SIGQUIT as usize), 131);
        assert_eq!(
            exit_code_for_signal(0),
            130,
            "no signal recorded yet falls back to SIGINT's code"
        );
    }

    #[test]
    fn drain_signal_pipe_reports_whether_anything_was_read() {
        let (fd, mut peer) = UnixStream::pair().expect("socketpair");
        fd.set_nonblocking(true).expect("set_nonblocking");
        assert_eq!(
            drain_signal_pipe(fd.as_fd()),
            Drained::Empty,
            "nothing queued — must not hang and must report Empty"
        );

        peer.write_all(b"xyz").expect("write");
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Signal);
        // Fully drained: a second call finds nothing left, and a fresh
        // `select` agrees the fd is no longer readable — proves the first
        // call didn't stop after one byte and leave the rest queued.
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Empty);
        assert!(!wait_readable(fd.as_fd(), Some(Duration::ZERO)).expect("select"));
    }

    /// The three-state split this test exists to cover: closing every write
    /// end must be classified distinctly from "nothing queued right now" —
    /// `run_terminator_blocking` treats the two very differently (permanent
    /// failure vs. keep waiting).
    #[test]
    fn drain_signal_pipe_reports_closed_when_every_writer_is_gone() {
        let (fd, peer) = UnixStream::pair().expect("socketpair");
        fd.set_nonblocking(true).expect("set_nonblocking");
        drop(peer);
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Closed);
    }

    /// Zero-effect check: fails if `OrphanGuard`'s `Drop` were a no-op.
    #[test]
    fn orphan_guard_arms_the_fallback_on_return_and_on_panic() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = OrphanGuard(Arc::clone(&flag));
            assert!(
                !flag.load(Ordering::SeqCst),
                "must not arm while still held"
            );
        }
        assert!(
            flag.load(Ordering::SeqCst),
            "must arm once dropped by a normal return"
        );

        let flag = Arc::new(AtomicBool::new(false));
        let panicking_flag = Arc::clone(&flag);
        let result = std::panic::catch_unwind(move || {
            let _guard = OrphanGuard(panicking_flag);
            panic!("simulated terminator-thread panic");
        });
        assert!(result.is_err());
        assert!(
            flag.load(Ordering::SeqCst),
            "must arm when unwound by a panic"
        );
    }

    #[test]
    fn wait_for_second_signal_times_out_when_nothing_arrives() {
        let (sig_read, _sig_write) = idle_sig_pipe();
        let flag = AtomicUsize::new(0);
        let grace = Duration::from_millis(30);

        let start = Instant::now();
        assert_eq!(wait_for_second_signal(sig_read.as_fd(), &flag, grace), None);
        assert!(
            start.elapsed() >= grace,
            "must wait out the full grace window before giving up"
        );
    }

    #[test]
    fn wait_for_second_signal_returns_the_mapped_code_of_whichever_arrives() {
        let (sig_read, mut sig_write) = idle_sig_pipe();
        let flag = AtomicUsize::new(0);
        // Stands in for signal-hook's own action pair for a real second
        // signal: the flag is set, then the pipe byte written — same order
        // `spawn_terminator` registers them in.
        flag.store(SIGTERM as usize, Ordering::Release);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            sig_write.write_all(b"x").expect("write");
        });

        let start = Instant::now();
        assert_eq!(
            wait_for_second_signal(sig_read.as_fd(), &flag, Duration::from_secs(5)),
            Some(143)
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must return as soon as the second signal arrives, not wait out the full grace window"
        );
        writer.join().expect("writer thread panicked");
    }
}
