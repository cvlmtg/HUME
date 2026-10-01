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
/// Opens `/dev/tty` directly on its own side channel (independent of the
/// [`SharedTerm`](crate::terminal::SharedTerm) event reader, so probe replies
/// can never race or interleave with it), builds a [`super::TtyChannel`]
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
        // on `/dev/tty`, the same reason termina's own event source
        // (`event/source/unix.rs`) and this module's `wait_readable` both use
        // `select` for terminal fds. EINTR retry and the
        // deadline-vs-remaining-budget recompute live in `wait_readable`.
        let remaining = deadline.saturating_duration_since(Instant::now());
        wait_readable(self.file.as_fd(), Some(remaining))
    }
}

// ── Terminator: process-termination signals ───────────────────────────────
//
// SIGINT/SIGTERM/SIGHUP/SIGQUIT arrive through a `signal_hook` self-pipe that
// this thread `select`s on. `request_quit` asks the main loop to quit, then
// the thread waits `QUIT_GRACE` before force-exiting; a second signal in the
// window is ignored. A pty teardown may not send
// SIGHUP; termina's reader then returns `UnexpectedEof` and the main loop
// exits on its own (see `hume_platform::hangup_exit_code`).
//
// A replaced signal disposition must exist only while something drains it:
// `signal_hook` cannot restore `SIG_DFL` once replaced. So the draining
// thread spawns before any disposition is replaced, and a
// `register_conditional_shutdown` fallback is armed when the thread stops.

/// Signals that ask the process to terminate. SIGQUIT is included so `kill
/// -QUIT` restores the terminal (raw mode, alt screen) before exiting,
/// trading away the default core dump; nothing here relies on one.
const SIGNALS: [i32; 4] = [SIGINT, SIGTERM, SIGHUP, SIGQUIT];

/// Fallback exit code when there's no real signal number to derive one
/// from: the zero/unknown-signal case in [`exit_code_for_signal`]. 130 is
/// `SIGINT`'s own `128 + signo`.
const CONVENTIONAL_EXIT_CODE: i32 = 130;

/// Maps a signal number to the conventional "killed by signal" exit code
/// (`128 + signo`). `0` (no signal recorded yet on `signal_flag`) falls
/// back to [`CONVENTIONAL_EXIT_CODE`].
fn exit_code_for_signal(signo: usize) -> i32 {
    if signo == 0 {
        CONVENTIONAL_EXIT_CODE
    } else {
        128 + signo as i32
    }
}

/// Arms the `register_conditional_shutdown` fallback the instant the
/// terminator thread stops draining the signal pipe, by a return or a panic.
/// Held as the thread body's first binding so every exit path, including an
/// unwind, runs `Drop` before the thread is gone. The two paths that
/// terminate the process directly (`force_exit`, via `process::exit`) never
/// reach it: `process::exit` runs no destructors, so a signal that
/// triggers a clean force-exit or hangup never needlessly flips this flag.
struct OrphanGuard(Arc<AtomicBool>);

impl Drop for OrphanGuard {
    fn drop(&mut self) {
        // SeqCst to match the load `register_conditional_shutdown`'s
        // handler performs on the same flag.
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Spawn a detached thread that terminates the process on one of [`SIGNALS`].
///
/// `request_quit` is called with the exit code the process should use
/// (`128 + signo`) and routes through the editor's normal quit path
/// (graceful LSP `shutdown`) rather than tearing the terminal down here.
/// This thread then waits up to `QUIT_GRACE` for the main loop to exit on
/// its own before force-restoring and exiting with that code anyway; a
/// second signal inside the window is ignored. If the thread
/// itself is lost (spawn failure, or a later permanent I/O error), a
/// `register_conditional_shutdown` fallback still terminates the process on
/// the next signal, without a graceful LSP shutdown or terminal restore.
/// See the module-level comment for why this is the best available fallback
/// under `signal_hook`'s no-`unsafe` API.
///
/// In raw mode the kernel does not deliver SIGINT for Ctrl-c (ISIG is
/// cleared), so this primarily covers `kill <pid>`. SIGINT stays registered
/// for the rare case something re-enables ISIG.
pub(super) fn spawn_terminator(
    term: crate::terminal::SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> io::Result<()> {
    // Self-pipe: the actual signal handlers (installed by `register`,
    // async-signal-safe) only write a byte here; all the real work (acting
    // on it, or not, exactly once) happens on the thread below, under no
    // signal-handler restrictions. Non-blocking so the thread can fully
    // drain it (see `drain_signal_pipe`) instead of a single bounded read
    // that could leave a queued byte from a second signal unread.
    let (sig_read, sig_write) = UnixStream::pair()?;
    sig_read.set_nonblocking(true)?;

    // Sees whichever of `SIGNALS` last ran its flag-set action. Read after
    // draining the pipe to turn "a signal happened" into "which one", for
    // `exit_code_for_signal`. Shared across all four registrations: if two
    // land close together this only tells us the most recent, which is fine:
    // it's read for the process's exit status, not to attribute causality.
    let signal_flag = Arc::new(AtomicUsize::new(0));
    // Set by `OrphanGuard` once this thread can no longer drain the pipe;
    // read by every signal's `register_conditional_shutdown` fallback below.
    let orphaned = Arc::new(AtomicBool::new(false));

    // The thread is spawned *before* any signal disposition is touched, so a
    // `Builder::spawn` failure returns `Err` with the kernel's defaults
    // completely untouched: no disposition is ever replaced with nothing
    // able to act on it.
    std::thread::Builder::new()
        .name("hume-terminator".into())
        .spawn({
            let signal_flag = Arc::clone(&signal_flag);
            let orphaned = Arc::clone(&orphaned);
            move || {
                // A handler that's installed but masked never runs: the same
                // "no one can act on this" hazard the registration order below
                // exists to avoid. neovim's `signal_init()` clears the mask
                // process-wide for the same reason; scoped to this thread only,
                // so LSP children are unaffected (`std::process::Command`
                // inherits the *spawning* thread's mask rather than resetting
                // it before `exec`).
                let mask: SigSet = SIGNALS
                    .iter()
                    .filter_map(|&s| Signal::try_from(s).ok())
                    .collect();
                let _ = mask.thread_unblock();

                let _guard = OrphanGuard(orphaned);
                match watch(sig_read.as_fd(), None) {
                    Ok(Watched::Signal) => {
                        let code = exit_code_for_signal(signal_flag.load(Ordering::Acquire));
                        request_quit(code);
                        wait_out_grace(sig_read.as_fd(), crate::QUIT_GRACE);
                        crate::force_exit(&term, code);
                    }
                    // An unbounded watch has no deadline to end on, so `Ended`
                    // is unreachable here (see `watch`'s own doc). Folded into
                    // the same arm as a real I/O failure: either way,
                    // termination coverage is lost; exit this thread quietly
                    // and let `OrphanGuard`'s fallback cover future signals
                    // instead.
                    Ok(Watched::Ended) | Err(_) => {}
                }
            }
        })?;

    for &signal in &SIGNALS {
        // The conditional-shutdown fallback is registered first so it
        // short-circuits ahead of the flag/pipe actions below once
        // `orphaned` is set: signal-hook-registry runs a signal's actions
        // in registration order. It terminates the process directly
        // (`libc::_exit`, async-signal-safe) with the same `128 + signo`
        // code the normal path would have used, once this thread is
        // confirmed gone. `register_conditional_default` (re-raise with
        // `SIG_DFL`) is the wrong tool here: it would restore SIGQUIT's
        // core dump, which `SIGNALS`' own doc comment trades
        // away.
        register_conditional_shutdown(
            signal,
            exit_code_for_signal(signal as usize),
            Arc::clone(&orphaned),
        )?;
        // Registration order matters within one signal's own action list:
        // the flag must be set before the pipe byte is written, so a reader
        // woken by the pipe never observes a stale flag (signal-hook's
        // documented self-pipe ordering rule).
        register_usize(signal, Arc::clone(&signal_flag), signal as usize)?;
        // `register` takes ownership of the fd it's given (closing it on
        // deregistration), so each signal needs its own dup. Handing the
        // same fd to multiple registrations would leave `sig_write`'s `Drop`
        // racing signal-hook's internal close of that same descriptor
        // number, and a later `open`/`socket` reusing it while a queued
        // signal still points at it.
        register(signal, sig_write.try_clone()?)?;
    }
    drop(sig_write);

    Ok(())
}

/// Outcome of draining the signal pipe.
#[derive(Debug, PartialEq, Eq)]
enum Drained {
    /// At least one handler's byte was read: a real signal.
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

/// What ended a [`watch`] call.
#[derive(Debug, PartialEq, Eq)]
enum Watched {
    /// One of [`SIGNALS`] arrived on `sig_fd`.
    Signal,
    /// `deadline` elapsed with nothing new. Only ever returned when a
    /// deadline was given. With `deadline: None` a permanently closed
    /// signal pipe (nothing else left to watch for) is an `Err` instead, per
    /// [`watch`]'s own contract.
    Ended,
}

/// Blocks on the registered-signal pipe `sig_fd` until a signal fires or
/// `deadline` elapses. Never touches the process. Kept separate from
/// [`spawn_terminator`] so it can be driven directly in tests. With
/// `deadline: None` this never returns [`Watched::Ended`], since an
/// unbounded wait has no deadline to end on.
///
/// `deadline: None` blocks indefinitely. A permanently closed `sig_fd` is a
/// hard failure only when `deadline` is `None` (matched by this module's own
/// `terminator_exits_instead_of_spinning_when_the_pipe_closes` test); with a
/// deadline, the caller has already committed to acting once it elapses
/// regardless, so this sleeps out the rest of the window instead of
/// returning early on that same closure
/// (`wait_out_grace_waits_out_the_window_on_a_closed_pipe` covers the
/// `Drained::Closed` case of this). A `wait_readable` `Err` gets the same
/// treatment when a deadline is given: the predecessor this function replaced
/// (`wait_for_second_signal`) returned `None` immediately on that same
/// `select` fault instead; sleeping it out here trades a faster force-exit
/// for never cutting the main thread's graceful LSP shutdown short on a
/// transient error.
fn watch(sig_fd: BorrowedFd<'_>, deadline: Option<Instant>) -> io::Result<Watched> {
    loop {
        let remaining = match deadline {
            Some(dl) => {
                let remaining = dl.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Watched::Ended);
                }
                Some(remaining)
            }
            None => None,
        };

        let sig_ready = match wait_readable(sig_fd, remaining) {
            Ok(ready) => ready,
            Err(e) => match remaining {
                // A deadline means the caller has already committed to
                // acting once it elapses regardless, so sleep out the rest
                // rather than collapsing the window to zero on a fault
                // that isn't proof there's nothing left to wait for.
                Some(r) => {
                    std::thread::sleep(r);
                    return Ok(Watched::Ended);
                }
                None => return Err(e),
            },
        };

        if sig_ready {
            // `select` reporting readable doesn't guarantee bytes are still
            // there to read by the time we get to it (a concurrent read, or
            // a spurious wakeup, could have emptied it first). Only an
            // actual drained byte counts as a real signal; otherwise loop
            // back and keep waiting.
            match drain_signal_pipe(sig_fd) {
                Drained::Signal => return Ok(Watched::Signal),
                Drained::Empty => continue,
                Drained::Closed => {
                    if deadline.is_none() {
                        // No wake source left with no deadline to wait out
                        // either: a real permanent failure, not a spin: the
                        // caller exits and `OrphanGuard`'s fallback takes
                        // over.
                        return Err(io::Error::from(io::ErrorKind::BrokenPipe));
                    }
                    if let Some(remaining) = remaining {
                        std::thread::sleep(remaining);
                    }
                    return Ok(Watched::Ended);
                }
            }
        }
    }
}

/// Waits out `grace` after a terminate trigger fired. A signal that arrives
/// meanwhile is drained and ignored: it neither ends the window early nor
/// changes the exit code, so the main thread's shutdown, including the dump
/// of unsaved buffers, runs undisturbed. `grace` is [`crate::QUIT_GRACE`] in
/// production; a parameter (rather than reading the constant directly) so
/// tests can bound their own runtime instead of waiting out the real
/// multi-second window.
fn wait_out_grace(sig_fd: BorrowedFd<'_>, grace: Duration) {
    let deadline = Instant::now() + grace;
    // `watch` never errors with a deadline given (see its own doc), but if
    // it somehow did, the grace window is over either way, which is what
    // `Ended` means, so the loop ends rather than panicking on a
    // documented-but-not-type-enforced invariant.
    while matches!(watch(sig_fd, Some(deadline)), Ok(Watched::Signal)) {}
}

/// `select(2)`-based readiness wait for a single `fd`. `select`, not
/// `poll(2)`: macOS's `poll` does not report readiness on `/dev/tty`, which
/// is why termina itself (`event/source/unix.rs`) uses `select` for the
/// terminal's main input fd; this matches that choice for the same fd
/// family. `timeout: None` blocks indefinitely.
fn wait_readable(fd: BorrowedFd<'_>, timeout: Option<Duration>) -> io::Result<bool> {
    // Deadline computed once, up front, from the caller's relative budget.
    let deadline = timeout.map(|d| Instant::now() + d);
    loop {
        let mut set = FdSet::new();
        set.insert(fd);
        // Recomputed against `deadline` on every pass, including after
        // `EINTR`. Reusing the original relative `timeout` on each retry
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
            Ok(_) => return Ok(set.contains(fd)),
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::from(e)),
        }
    }
}

#[cfg(test)]
mod terminator_tests {
    use std::net::Shutdown;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    // `UnixStream::pair()` reproduces the primitives `watch` observes without
    // needing a real signal: writing a byte to one end stands in for a
    // `signal_hook` self-pipe write, since we're only testing this module's
    // own signal-pipe logic, not signal-hook's (separately tested, upstream)
    // job of relaying a real signal to a fd.

    /// An inert stand-in for the signal self-pipe: never written to, so it
    /// never reports readable. Keeps both ends alive so it can't spuriously
    /// look closed either. The read end is non-blocking, matching
    /// `spawn_terminator`'s real `sig_read`: `drain_signal_pipe` reads it to
    /// `WouldBlock`, which would hang forever on a blocking fd once emptied.
    fn idle_sig_pipe() -> (UnixStream, UnixStream) {
        let (sig_read, sig_write) = UnixStream::pair().expect("socketpair");
        sig_read.set_nonblocking(true).expect("set_nonblocking");
        (sig_read, sig_write)
    }

    /// Retires the write end so the reader sees EOF. A plain `drop` is not
    /// enough: a child forked by a concurrent test in this same binary
    /// inherits a duplicate of the descriptor between `fork` and `exec`
    /// (`CLOEXEC` closes it at `exec`, not at `fork`), keeping the
    /// connection's write side open and turning the expected EOF into a
    /// transient `EWOULDBLOCK`. `shutdown` acts on the connection every
    /// duplicate shares, so the EOF is unconditional.
    fn retire_write_end(sig_write: UnixStream) {
        sig_write
            .shutdown(Shutdown::Write)
            .expect("shutdown write end");
    }

    /// Waits for a spawned `watch` call to finish, failing the test if it
    /// hasn't within `bound`. A regression to the drain-less spin
    /// `terminator_exits_instead_of_spinning_when_the_pipe_closes` guards
    /// against (an unbounded `watch` call that never returns) is a real
    /// failure mode for this module. Polling `is_finished()`
    /// against a deadline turns a spin into a fast, visible test failure
    /// instead of hanging `cargo test` forever, while `join()`ing rather than
    /// snapshotting once also gives an unscheduled-but-not-spinning thread
    /// its full `bound` to run; a blind post-spawn sleep can't tell the two
    /// apart.
    fn join_bounded(
        handle: std::thread::JoinHandle<io::Result<Watched>>,
        bound: Duration,
    ) -> io::Result<Watched> {
        let deadline = Instant::now() + bound;
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "watch did not return within {bound:?}: regression to the \
                 drain-less spin this module was built to avoid"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.join().expect("terminator thread panicked")
    }

    /// Takes `sig_read` by value (it must outlive the spawned thread)
    /// while each test keeps its own writer handle so it can still act on
    /// the connection after the call starts.
    fn run_bounded(sig_read: UnixStream, bound: Duration) -> Watched {
        let handle = std::thread::spawn(move || watch(sig_read.as_fd(), None));
        join_bounded(handle, bound).expect("watch returned an error")
    }

    /// Hang-detector bound for [`run_bounded`], generous on purpose since it
    /// only needs to catch a genuine spin, not assert on latency.
    const SPIN_BOUND: Duration = Duration::from_secs(2);

    #[test]
    fn detects_signal() {
        let (sig_read, mut sig_write) = idle_sig_pipe();
        sig_write.write_all(b"x").expect("write");
        assert_eq!(
            run_bounded(sig_read, SPIN_BOUND),
            Watched::Signal,
            "signal must be detected"
        );
    }

    /// Guards against a spin the actor-before-disposition setup order makes
    /// possible: once every write end of the signal pipe is
    /// gone, the pipe reads EOF-ready forever, and a `bool`-returning drain
    /// that treated "nothing read" as "keep waiting" would burn 100% CPU
    /// instead of ever returning. Bounded by a timeout so a regression fails
    /// the test rather than hanging the suite.
    #[test]
    fn terminator_exits_instead_of_spinning_when_the_pipe_closes() {
        let (sig_read, sig_write) = idle_sig_pipe();
        retire_write_end(sig_write);

        let handle = std::thread::spawn(move || watch(sig_read.as_fd(), None));
        join_bounded(handle, SPIN_BOUND)
            .expect_err("a permanently closed signal pipe is a real failure, not a trigger");
    }

    // ── wait_out_grace ───────────────────────────────────────────────────

    #[test]
    fn wait_out_grace_waits_the_full_window_when_nothing_arrives() {
        let (sig_read, _sig_write) = idle_sig_pipe();
        let grace = Duration::from_millis(30);

        let start = Instant::now();
        wait_out_grace(sig_read.as_fd(), grace);

        assert!(
            start.elapsed() >= grace,
            "must wait out the full grace window"
        );
    }

    #[test]
    fn a_second_signal_does_not_shorten_the_grace_window() {
        let (sig_read, mut sig_write) = idle_sig_pipe();
        let grace = Duration::from_millis(80);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            sig_write.write_all(b"x").expect("write");
            // Returned so the write end outlives the signal: dropping it
            // would read as a closed pipe, a different path from the one
            // under test.
            sig_write
        });

        let start = Instant::now();
        wait_out_grace(sig_read.as_fd(), grace);

        assert!(
            start.elapsed() >= grace,
            "a signal inside the window must not end it early"
        );
        drop(writer.join().expect("writer thread panicked"));
    }

    /// Covers `watch`'s `Drained::Closed`-with-deadline arm: every write end
    /// of the signal pipe going away mid-grace-window must not end the
    /// window early (unlike the `deadline: None` case tested by
    /// `terminator_exits_instead_of_spinning_when_the_pipe_closes`), since
    /// the caller has already committed to acting once the window elapses
    /// regardless.
    #[test]
    fn wait_out_grace_waits_out_the_window_on_a_closed_pipe() {
        let (sig_read, sig_write) = idle_sig_pipe();
        retire_write_end(sig_write);
        let grace = Duration::from_millis(30);

        let start = Instant::now();
        wait_out_grace(sig_read.as_fd(), grace);

        assert!(
            start.elapsed() >= grace,
            "must sleep out the rest of the window rather than returning early on the closure"
        );
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
            "nothing queued, must not hang and must report Empty"
        );

        peer.write_all(b"xyz").expect("write");
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Signal);
        // Fully drained: a second call finds nothing left, and a fresh
        // `select` agrees the fd is no longer readable. Proves the first
        // call didn't stop after one byte and leave the rest queued.
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Empty);
        assert!(!wait_readable(fd.as_fd(), Some(Duration::ZERO)).expect("select"));
    }

    /// The three-state split this test exists to cover: closing every write
    /// end must be classified distinctly from "nothing queued right now":
    /// `watch` treats the two very differently (permanent failure vs. keep
    /// waiting).
    #[test]
    fn drain_signal_pipe_reports_closed_when_every_writer_is_gone() {
        let (fd, peer) = UnixStream::pair().expect("socketpair");
        fd.set_nonblocking(true).expect("set_nonblocking");
        retire_write_end(peer);
        assert_eq!(drain_signal_pipe(fd.as_fd()), Drained::Closed);
    }

    /// `OrphanGuard`'s `Drop` must set the flag whether the scope ends by
    /// returning or by unwinding.
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
}
