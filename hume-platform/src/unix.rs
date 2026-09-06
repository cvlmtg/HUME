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
        // (`event/source/unix.rs`) and this module's `wait_readable` both use
        // `select` for terminal fds. EINTR retry and the
        // deadline-vs-remaining-budget recompute live in `wait_readable`.
        let remaining = deadline.saturating_duration_since(Instant::now());
        wait_readable(self.file.as_fd(), Some(remaining))
    }
}

// ── Terminator: process-termination signals ───────────────────────────────
//
// SIGINT/SIGTERM/SIGHUP/SIGQUIT, delivered via a `signal_hook` self-pipe —
// the same technique `termina` uses internally for SIGWINCH
// (`event/source/unix.rs`): the signal handler just writes one byte to a
// pipe, and this thread's `select` treats that pipe like any other fd.
// `request_quit` asks the main loop to quit gracefully (its event reader is
// alive and will see the wake); this thread then waits up to `QUIT_GRACE`
// for it to exit on its own before force-exiting with that code anyway, or
// with a second signal's code if one arrives inside the window.
//
// This module used to also watch `/dev/tty` directly, on its own fd
// independent of the main loop's reader, because a pty teardown (e.g. `vhs`
// closing the master after a recording) isn't guaranteed to deliver SIGHUP
// — hume is rarely the session leader of its tty — and termina 0.3.3's
// `UnixEventSource::try_read` mapped the resulting tty EOF to `Ok(None)`
// rather than an error, so the main loop's event wait spun forever without
// ever returning. termina ≥0.4.0 fixes this at the source (`try_read` now
// returns `Err(io::ErrorKind::UnexpectedEof)` on that same zero-byte read;
// upstream commit `309350ba54`), so the main loop's own reader now surfaces
// a hangup as an ordinary error and returns on its own — see
// `hume_platform::hangup_exit_code` for how the exit code that used to come
// from this thread's tty watch is now derived from that error instead. Do
// not re-add a tty watch here; the failure mode it existed for no longer
// exists upstream.
//
// Setup order enforces one invariant: a replaced signal disposition must
// exist only while something can act on it. `signal_hook` has no way to
// restore a disposition once replaced (`unregister` drops the callback
// without touching `SIG_DFL`, so a later signal is silently swallowed —
// documented in `signal-hook-registry`'s source), ruling out
// register-then-unregister-on-failure as a recovery path. Two mechanisms
// cover it instead: the draining thread spawns *before* any disposition is
// replaced, so a spawn failure leaves kernel defaults untouched; and a
// `register_conditional_shutdown` fallback, armed the instant the thread
// stops draining (return or panic), covers the thread-dies-later case.

/// Signals that ask the process to terminate. SIGQUIT is included so `kill
/// -QUIT` restores the terminal (raw mode, alt screen) before exiting,
/// trading away the default core dump — nothing here relies on one.
const SIGNALS: [i32; 4] = [SIGINT, SIGTERM, SIGHUP, SIGQUIT];

/// This crate's exit code before any of the exit-fidelity tracking here
/// existed — a fixed 130 (`SIGINT`'s own `128 + signo`), used today as the
/// fallback when there's no real signal number to derive one from: the
/// zero/unknown-signal case in [`exit_code_for_signal`].
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

/// Spawn a detached thread that terminates the process on one of [`SIGNALS`].
///
/// `request_quit` is called with the exit code the process should use —
/// `128 + signo` — and routes through the editor's normal quit path
/// (graceful LSP `shutdown`) rather than tearing the terminal down here.
/// This thread then waits up to `QUIT_GRACE` for the main loop to exit on
/// its own before force-restoring and exiting with that code anyway, or with
/// a second signal's code if one arrives inside the window. If the thread
/// itself is lost (spawn failure, or a later permanent I/O error), a
/// `register_conditional_shutdown` fallback still terminates the process on
/// the next signal, without a graceful LSP shutdown or terminal restore —
/// see the module-level comment for why this is the best available fallback
/// under `signal_hook`'s no-`unsafe` API.
///
/// In raw mode the kernel does not deliver SIGINT for Ctrl+C (ISIG is
/// cleared), so this primarily covers `kill <pid>` — SIGINT stays registered
/// for the rare case something re-enables ISIG.
pub(super) fn spawn_terminator(
    term: crate::terminal::SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> io::Result<()> {
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
                match run_terminator_blocking(sig_read.as_fd()) {
                    Ok(Watched::Signal) => {
                        let code = exit_code_for_signal(signal_flag.load(Ordering::Acquire));
                        request_quit(code);
                        let code = grace_window_exit_code(
                            sig_read.as_fd(),
                            &signal_flag,
                            code,
                            crate::QUIT_GRACE,
                        );
                        crate::force_exit(&term, code);
                    }
                    // An unbounded watch has no deadline to end on, so `Ended`
                    // is unreachable here — see `run_terminator_blocking`'s
                    // own doc. Folded into the same arm as a real I/O
                    // failure: either way, termination coverage is lost;
                    // exit this thread quietly and let `OrphanGuard`'s
                    // fallback cover future signals instead.
                    Ok(Watched::Ended) | Err(_) => {}
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

/// Blocks until one of [`SIGNALS`] fires, then returns [`Watched::Signal`]
/// — never [`Watched::Ended`], since an unbounded wait has no deadline to
/// end on (see [`watch`]'s own doc for why that variant needs a deadline to
/// be reachable at all). Returns `Err` only when the signal pipe itself is
/// permanently gone (every write end closed) or the initial wait on it
/// fails. Never touches the process — kept separate from
/// [`spawn_terminator`] so it can be driven directly in tests. A thin
/// wrapper over [`watch`] — see there for the shared logic with
/// [`grace_window_exit_code`]'s bounded wait.
fn run_terminator_blocking(sig_fd: BorrowedFd<'_>) -> io::Result<Watched> {
    watch(sig_fd, None)
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

/// What ended a [`watch`] call.
#[derive(Debug, PartialEq, Eq)]
enum Watched {
    /// One of [`SIGNALS`] arrived on `sig_fd`.
    Signal,
    /// `deadline` elapsed with nothing new. Only ever returned when a
    /// deadline was given — with `deadline: None` a permanently closed
    /// signal pipe (nothing else left to watch for) is an `Err` instead, per
    /// [`run_terminator_blocking`]'s contract.
    Ended,
}

/// Blocks on the registered-signal pipe `sig_fd` until a signal fires or
/// `deadline` elapses. [`run_terminator_blocking`] and
/// [`grace_window_exit_code`] are both thin wrappers over this: the former
/// is the very first, unbounded wait before any trigger has fired; the
/// latter is the bounded wait afterward, racing a second signal against the
/// grace window running out.
///
/// `deadline: None` blocks indefinitely. A permanently closed `sig_fd` is a
/// hard failure only when `deadline` is `None`
/// (`run_terminator_blocking`'s contract, matched by its own
/// `terminator_exits_instead_of_spinning_when_the_pipe_closes` test); with a
/// deadline, the caller has already committed to acting once it elapses
/// regardless, so this sleeps out the rest of the window instead of
/// returning early on that same closure.
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
                // acting once it elapses regardless — sleep out the rest
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
            // a spurious wakeup, could have emptied it first) — only an
            // actual drained byte counts as a real signal; otherwise loop
            // back and keep waiting.
            match drain_signal_pipe(sig_fd) {
                Drained::Signal => return Ok(Watched::Signal),
                Drained::Empty => continue,
                Drained::Closed => {
                    if deadline.is_none() {
                        // No wake source left with no deadline to wait out
                        // either — a real permanent failure, not a spin: the
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

/// Waits out the remainder of `grace` after a terminate trigger fired,
/// racing a second signal against the grace window running out, then maps
/// the outcome to the exit code [`force_exit`](crate::force_exit) should
/// use: the second signal's own mapped code if one arrives first, or
/// `fallback_code` for a plain timeout. `grace` is [`crate::QUIT_GRACE`] in
/// production; a parameter (rather than reading the constant directly) so
/// tests can bound their own runtime instead of waiting out the real
/// multi-second window.
fn grace_window_exit_code(
    sig_fd: BorrowedFd<'_>,
    signal_flag: &AtomicUsize,
    fallback_code: i32,
    grace: Duration,
) -> i32 {
    // `watch` never errors with a deadline given — see its own doc.
    let outcome = watch(sig_fd, Some(Instant::now() + grace))
        .expect("watch never errors when deadline is Some");
    match outcome {
        Watched::Signal => exit_code_for_signal(signal_flag.load(Ordering::Acquire)),
        Watched::Ended => fallback_code,
    }
}

/// `select(2)`-based readiness wait for a single `fd`. `select`, not
/// `poll(2)` — macOS's `poll` does not report readiness on `/dev/tty`, which
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
            Ok(_) => return Ok(set.contains(fd)),
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
    // observes without needing a real signal: writing a byte to one end
    // stands in for a `signal_hook` self-pipe write, since we're only
    // testing this module's own signal-pipe logic, not signal-hook's
    // (separately tested, upstream) job of relaying a real signal to a fd.

    /// An inert stand-in for the signal self-pipe: never written to, so it
    /// never reports readable. Keeps both ends alive so it can't spuriously
    /// look closed either. The read end is non-blocking, matching
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
    /// run of this module's own signal-detection test sat at 100% CPU for
    /// three days before being mistaken for a live bug). A direct call on
    /// the test thread would hang `cargo test` forever on that regression
    /// instead of failing it. This runs the call on its own thread and fails
    /// the test if it hasn't returned within `bound`, so a spin becomes a
    /// fast, visible test failure. Takes `sig_read` by value — it must
    /// outlive the spawned thread — while each test keeps its own writer
    /// handle so it can still act on the connection after the call starts.
    fn run_bounded(sig_read: UnixStream, bound: Duration) -> Watched {
        let handle = std::thread::spawn(move || run_terminator_blocking(sig_read.as_fd()));
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

    /// Regression test for the spin the actor-before-disposition reorder
    /// would otherwise introduce: once every write end of the signal pipe is
    /// gone, the pipe reads EOF-ready forever, and a `bool`-returning drain
    /// that treated "nothing read" as "keep waiting" would burn 100% CPU
    /// instead of ever returning. Bounded by a timeout so a regression fails
    /// the test rather than hanging the suite.
    #[test]
    fn terminator_exits_instead_of_spinning_when_the_pipe_closes() {
        let (sig_read, sig_write) = idle_sig_pipe();
        drop(sig_write);

        let handle = std::thread::spawn(move || run_terminator_blocking(sig_read.as_fd()));
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
}
