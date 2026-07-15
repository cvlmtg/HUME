//! Cross-thread wake primitive for the main event loop.
//!
//! [`EventWait::wait`] blocks the main thread until terminal input is
//! (probably) ready, a background thread calls [`EventWaker::wake`], or a
//! deadline elapses. This replaces the old fixed poll-cadence wakeups
//! (`PENDING_POLL`/`LSP_HEARTBEAT` in `hume-editor`) with arrival-driven
//! wakes plus real deadlines only: background threads (the LSP transport,
//! the parse worker) hold a cloned [`EventWaker`] and call `wake()` right
//! after posting a result, so the loop drains it on the next iteration
//! instead of rechecking every few milliseconds.
//!
//! Backed by `poll(2)` over `[tty fd, self-pipe]` on Unix and
//! `WaitForMultipleObjects` over `[console handle, auto-reset event]` on
//! Windows. `wake()` is a cheap, lossy, idempotent signal: multiple wakes
//! before the next `wait()` coalesce into one `Woken` outcome, which is
//! always safe because callers re-drain every async source on each loop
//! iteration regardless of why they woke.

use std::io;
use std::time::Duration;

/// Why [`EventWait::wait`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// Terminal input is probably readable. Advisory, not a guarantee:
    /// crossterm may have buffered a partial escape sequence, or (on
    /// Windows) the signaled console record may be one crossterm filters
    /// out entirely — confirm with a zero-timeout crossterm poll before
    /// reading.
    Input,
    /// A background thread called [`EventWaker::wake`], or (Unix) the wait
    /// was interrupted by a signal (SIGWINCH or otherwise). Spurious wakes
    /// are a legal outcome here — callers must tolerate them by simply
    /// re-checking their own state.
    Woken,
    /// The caller's timeout elapsed with nothing else happening.
    TimedOut,
}

/// Construct a wait/wake pair. `EventWait` lives on the main thread and is
/// not `Clone` — only one thread waits at a time. `EventWaker` is
/// `Clone + Send + Sync`; hand a clone to every background thread that can
/// produce inbound work (LSP transport, parse worker) so it can wake the
/// waiting thread after posting a result.
///
/// Never fails to construct the wake side; the input side degrades
/// gracefully when no terminal is available (headless: `wait` simply never
/// returns [`WaitOutcome::Input`]).
pub fn event_wait_pair() -> io::Result<(EventWait, EventWaker)> {
    let (wait, waker) = imp::event_wait_pair()?;
    Ok((EventWait(wait), EventWaker(waker)))
}

/// The wait side of an [`event_wait_pair`]. Lives on the main thread.
pub struct EventWait(imp::EventWait);

impl EventWait {
    /// Block until terminal input is (probably) ready, a wake arrives, or
    /// `timeout` elapses. `None` blocks indefinitely.
    pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<WaitOutcome> {
        self.0.wait(timeout)
    }
}

/// The wake side of an [`event_wait_pair`]. Cheap to clone — every
/// background thread that can produce inbound work holds one.
#[derive(Clone)]
pub struct EventWaker(imp::EventWaker);

impl EventWaker {
    /// Signal the waiting thread. Never blocks, never fails: an already-full
    /// wake channel means a wake is already pending (the waiting thread will
    /// see it), and a closed one means nothing is waiting to be woken.
    pub fn wake(&self) {
        self.0.wake();
    }
}

// ---------------------------------------------------------------------------
// Unix: poll(2) over [tty fd, self-pipe], SIGWINCH routed onto the same pipe.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::io::{self, IsTerminal, Read, Write};
    use std::os::fd::{AsFd, BorrowedFd};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::time::Duration;

    use nix::errno::Errno;
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use signal_hook::SigId;
    use signal_hook::consts::SIGWINCH;

    use super::WaitOutcome;

    pub(super) fn event_wait_pair() -> io::Result<(EventWait, EventWaker)> {
        let (wake_read, wake_write) = UnixStream::pair()?;
        wake_read.set_nonblocking(true)?;
        wake_write.set_nonblocking(true)?;

        // SIGWINCH doesn't make the tty readable — crossterm learns of
        // resize through its own signal-hook registration inside its own
        // mio poll (see `input_source`'s VERSION COUPLING note). Register a
        // second one onto our own pipe — signal-hook supports multiple
        // registrations per signal, so this composes with crossterm's
        // without interfering — so a resize also wakes us. `Drop`
        // unregisters it below.
        let sig_pipe_end = wake_write.try_clone()?;
        let sig_id = signal_hook::low_level::pipe::register(SIGWINCH, sig_pipe_end)?;

        let input = input_source()?;

        Ok((
            EventWait {
                input,
                wake_read,
                sig_id: Some(sig_id),
            },
            EventWaker(Arc::new(wake_write)),
        ))
    }

    #[derive(Debug, PartialEq, Eq)]
    enum InputKind {
        Stdin,
        Tty,
        None,
    }

    /// VERSION COUPLING: replicates crossterm 0.29's Unix event source
    /// input-fd choice (`terminal/sys/file_descriptor.rs::tty_fd`, roughly
    /// lines 123-135): stdin if it's a terminal, else `/dev/tty`, else no
    /// pollable input (headless). We must poll the same descriptor crossterm
    /// reads from, or input could arrive on a descriptor we never wake for —
    /// a crossterm upgrade that changes this choice would silently break
    /// input wakeups here. Split out as a pure decision over the two facts
    /// it depends on so the rule itself is unit-testable.
    fn choose_input_kind(stdin_is_terminal: bool, tty_openable: bool) -> InputKind {
        if stdin_is_terminal {
            InputKind::Stdin
        } else if tty_openable {
            InputKind::Tty
        } else {
            InputKind::None
        }
    }

    fn input_source() -> io::Result<Option<InputSource>> {
        let stdin_is_terminal = io::stdin().is_terminal();
        // Only open /dev/tty when stdin isn't the input source (skip the
        // open on the common terminal-stdin path). Open failure is headless,
        // not an error — `wait` simply never reports `Input`.
        let tty = if stdin_is_terminal {
            None
        } else {
            std::fs::File::open("/dev/tty").ok()
        };
        Ok(match choose_input_kind(stdin_is_terminal, tty.is_some()) {
            InputKind::Stdin => Some(InputSource::Stdin(io::stdin())),
            InputKind::Tty => Some(InputSource::Tty(
                tty.expect("tty_openable ⇒ Some, per choose_input_kind"),
            )),
            InputKind::None => None,
        })
    }

    enum InputSource {
        Stdin(std::io::Stdin),
        Tty(std::fs::File),
        #[cfg(test)]
        Fake(UnixStream),
    }

    impl InputSource {
        fn as_fd(&self) -> BorrowedFd<'_> {
            match self {
                InputSource::Stdin(s) => s.as_fd(),
                InputSource::Tty(f) => f.as_fd(),
                #[cfg(test)]
                InputSource::Fake(s) => s.as_fd(),
            }
        }
    }

    pub(super) struct EventWait {
        input: Option<InputSource>,
        wake_read: UnixStream,
        /// `None` only in the `#[cfg(test)]` helper constructor, which skips
        /// real SIGWINCH registration — the fd-swap tests don't exercise it
        /// and real registrations would churn process-global signal state
        /// for no benefit.
        sig_id: Option<SigId>,
    }

    impl EventWait {
        pub(super) fn wait(&mut self, timeout: Option<Duration>) -> io::Result<WaitOutcome> {
            let input_fd = self.input.as_ref().map(InputSource::as_fd);
            let ready = wait_on_fds(input_fd, self.wake_read.as_fd(), timeout)?;

            // Clear-before-act: drain any pending wake byte(s) before
            // deciding the outcome, regardless of which outcome wins. The
            // EINTR arm in `wait_on_fds` also reports `wake = true` without
            // a guarantee there's an actual byte to drain — harmless,
            // `drain` no-ops on `WouldBlock`.
            if ready.wake {
                drain(&mut self.wake_read);
            }

            if ready.input {
                return Ok(WaitOutcome::Input);
            }
            if ready.wake {
                return Ok(WaitOutcome::Woken);
            }
            if ready.input_closed {
                return Err(io::Error::other("terminal input closed"));
            }
            Ok(WaitOutcome::TimedOut)
        }
    }

    impl Drop for EventWait {
        fn drop(&mut self) {
            if let Some(id) = self.sig_id {
                signal_hook::low_level::unregister(id);
            }
        }
    }

    #[derive(Clone)]
    pub(super) struct EventWaker(Arc<UnixStream>);

    impl EventWaker {
        pub(super) fn wake(&self) {
            // Nonblocking 1-byte write; every error is ignored on purpose
            // (see the crate-level doc): `WouldBlock` means a wake is
            // already pending, anything else means the wait side is gone.
            let _ = (&*self.0).write(&[0u8]);
        }
    }

    /// Which of the (at most two) polled fds were reported readable, plus
    /// whether the input fd looks permanently dead (`ERR`/`HUP`/`NVAL`).
    #[derive(Debug, Clone, Copy, Default)]
    struct RawReady {
        input: bool,
        wake: bool,
        input_closed: bool,
    }

    /// The testable core of `wait`: one `poll(2)` call over 1-2 fds, with no
    /// terminal/thread/signal state — tests substitute plain `UnixStream`
    /// pairs for the tty fd.
    fn wait_on_fds(
        input: Option<BorrowedFd<'_>>,
        wake: BorrowedFd<'_>,
        timeout: Option<Duration>,
    ) -> io::Result<RawReady> {
        let poll_timeout = to_poll_timeout(timeout);
        match input {
            Some(input_fd) => {
                let mut pfds = [
                    PollFd::new(input_fd, PollFlags::POLLIN),
                    PollFd::new(wake, PollFlags::POLLIN),
                ];
                if poll_once(&mut pfds, poll_timeout)? {
                    return Ok(RawReady {
                        wake: true,
                        ..RawReady::default()
                    });
                }
                let input_r = pfds[0].revents().unwrap_or(PollFlags::empty());
                let wake_r = pfds[1].revents().unwrap_or(PollFlags::empty());
                Ok(RawReady {
                    input: input_r.contains(PollFlags::POLLIN),
                    wake: wake_r.contains(PollFlags::POLLIN),
                    input_closed: input_r
                        .intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
                        && !input_r.contains(PollFlags::POLLIN),
                })
            }
            None => {
                let mut pfds = [PollFd::new(wake, PollFlags::POLLIN)];
                if poll_once(&mut pfds, poll_timeout)? {
                    return Ok(RawReady {
                        wake: true,
                        ..RawReady::default()
                    });
                }
                Ok(RawReady {
                    wake: pfds[0]
                        .revents()
                        .unwrap_or(PollFlags::empty())
                        .contains(PollFlags::POLLIN),
                    ..RawReady::default()
                })
            }
        }
    }

    /// Runs `poll(2)`. POSIX never restarts `poll` after a signal handler,
    /// so EINTR is treated as an interruption the caller should handle as a
    /// spurious wake (`Ok(true)`) rather than retried here — retrying would
    /// silently extend the caller's timeout budget past what it asked for.
    /// `Ok(false)` means poll completed normally; the caller reads
    /// `revents`.
    fn poll_once(pfds: &mut [PollFd], timeout: PollTimeout) -> io::Result<bool> {
        match poll(pfds, timeout) {
            Ok(_) => Ok(false),
            Err(Errno::EINTR) => Ok(true),
            Err(e) => Err(io::Error::from(e)),
        }
    }

    fn to_poll_timeout(timeout: Option<Duration>) -> PollTimeout {
        match timeout {
            None => PollTimeout::NONE,
            // nix's `TryFrom<Duration>` covers up to `i32::MAX` milliseconds
            // (~24.8 days) — every real deadline in this codebase (LSP
            // request timeouts, timer wheel entries) is far under that, so
            // the fallback to `PollTimeout::MAX` on overflow is unreachable
            // in practice; it just avoids a panic if it ever were.
            Some(d) => PollTimeout::try_from(d).unwrap_or(PollTimeout::MAX),
        }
    }

    /// Drains every byte currently sitting in the wake pipe. Never blocks:
    /// stops at the first `WouldBlock`. A read error on our own pipe (should
    /// not happen for a live pair) is treated the same as empty — nothing
    /// left to act on.
    fn drain(stream: &mut UnixStream) {
        let mut buf = [0u8; 64];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    #[cfg(test)]
    fn for_test(fake_input: Option<UnixStream>) -> (EventWait, EventWaker) {
        let (wake_read, wake_write) = UnixStream::pair().expect("wake socketpair");
        wake_read.set_nonblocking(true).expect("nonblocking");
        wake_write.set_nonblocking(true).expect("nonblocking");
        let input = fake_input.map(|s| {
            s.set_nonblocking(true).expect("nonblocking");
            InputSource::Fake(s)
        });
        (
            EventWait {
                input,
                wake_read,
                sig_id: None,
            },
            EventWaker(Arc::new(wake_write)),
        )
    }

    /// Like `for_test(None)` but registers the real process-global SIGWINCH
    /// handler onto the wake pipe, exactly as `event_wait_pair` does. Keeps
    /// the real signal_hook wiring under test while leaving the controlling
    /// tty out of the poll set — the tty's readiness is environment-dependent
    /// and would otherwise race `Woken` against `Input`.
    #[cfg(test)]
    fn for_test_sigwinch() -> io::Result<(EventWait, EventWaker)> {
        let (wake_read, wake_write) = UnixStream::pair()?;
        wake_read.set_nonblocking(true)?;
        wake_write.set_nonblocking(true)?;
        let sig_pipe_end = wake_write.try_clone()?;
        let sig_id = signal_hook::low_level::pipe::register(SIGWINCH, sig_pipe_end)?;
        Ok((
            EventWait {
                input: None,
                wake_read,
                sig_id: Some(sig_id),
            },
            EventWaker(Arc::new(wake_write)),
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Serializes every test that touches the process-global SIGWINCH
        // registration; see for_test_sigwinch. Mirrors CWD_MUTEX in
        // hume-editor tests.
        static SIGWINCH_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

        #[test]
        fn choose_input_kind_matches_crossterm_rule() {
            // Oracle: crossterm 0.29 tty_fd — stdin if it's a terminal, else
            // /dev/tty, else headless.
            assert_eq!(choose_input_kind(true, false), InputKind::Stdin);
            assert_eq!(
                choose_input_kind(true, true),
                InputKind::Stdin,
                "stdin wins even when /dev/tty is also openable",
            );
            assert_eq!(choose_input_kind(false, true), InputKind::Tty);
            assert_eq!(
                choose_input_kind(false, false),
                InputKind::None,
                "headless: no pollable input source",
            );
        }

        #[test]
        fn input_source_resolves_without_panic() {
            // Under cargo's harness stdin is not a terminal, so this drives
            // the else-branch and the `.expect` glue: result is Tty
            // (controlling terminal present) or None (headless) — never an
            // error, never a panic.
            assert!(input_source().is_ok());
        }

        #[test]
        fn wake_from_thread_returns_woken() {
            let (mut wait, waker) = for_test(None);
            let handle = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                waker.wake();
            });
            let outcome = wait.wait(Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(outcome, WaitOutcome::Woken);
            handle.join().expect("waker thread");
        }

        #[test]
        fn timeout_returns_timed_out() {
            let (mut wait, _waker) = for_test(None);
            let outcome = wait.wait(Some(Duration::from_millis(10))).expect("wait");
            assert_eq!(outcome, WaitOutcome::TimedOut);
        }

        #[test]
        fn input_ready_returns_input() {
            let (fake_input, mut counterpart) = UnixStream::pair().expect("input socketpair");
            let (mut wait, _waker) = for_test(Some(fake_input));
            counterpart.write_all(b"x").expect("write");
            let outcome = wait.wait(Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(outcome, WaitOutcome::Input);
        }

        #[test]
        fn input_wins_but_wake_is_drained() {
            // Exercised at the `wait_on_fds` + `drain` level (not the full
            // `EventWait`): once `EventWait::wait` reports `Input`, the
            // input fd itself is never consumed by design (the run loop
            // reads it via crossterm afterward), so a second `wait()` call
            // on the same `EventWait` would see the same unread input byte
            // and report `Input` again — that would falsely look like a
            // drain failure. Testing the two load-bearing pieces directly
            // avoids that confound.
            let (input_read, mut input_write) = UnixStream::pair().expect("input pair");
            let (mut wake_read, wake_write) = UnixStream::pair().expect("wake pair");
            wake_read.set_nonblocking(true).expect("nonblocking");
            wake_write.set_nonblocking(true).expect("nonblocking");

            input_write.write_all(b"x").expect("write input");
            (&wake_write).write_all(&[0u8]).expect("write wake");

            let ready = wait_on_fds(
                Some(input_read.as_fd()),
                wake_read.as_fd(),
                Some(Duration::from_secs(1)),
            )
            .expect("poll");
            assert!(ready.input, "input should be ready");
            assert!(ready.wake, "wake should be ready too");

            drain(&mut wake_read);
            let mut buf = [0u8; 1];
            assert!(
                matches!(wake_read.read(&mut buf), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
                "wake pipe must be empty after drain"
            );

            // Draining must not have broken future wakes.
            (&wake_write).write_all(&[0u8]).expect("write wake again");
            let ready_again =
                wait_on_fds(None, wake_read.as_fd(), Some(Duration::from_secs(1))).expect("poll");
            assert!(ready_again.wake, "wake pipe must still work after a drain");
        }

        #[test]
        fn many_wakes_coalesce_to_one() {
            let (mut wait, waker) = for_test(None);
            for _ in 0..100 {
                waker.wake();
            }
            let first = wait.wait(Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(first, WaitOutcome::Woken);
            let second = wait.wait(Some(Duration::from_millis(10))).expect("wait");
            assert_eq!(
                second,
                WaitOutcome::TimedOut,
                "100 wakes must coalesce, not queue up 100 separate wakeups"
            );
        }

        #[test]
        fn sigwinch_wakes() {
            // Real SIGWINCH must wake the poll. `raise(SIGWINCH)` runs the
            // signal-hook handler synchronously on this thread, writing a
            // byte to the registered wake pipe before `wait` polls — so with
            // no tty in the poll set the outcome is deterministically Woken.
            // The mutex keeps any future SIGWINCH-registering test from
            // overlapping (signal-hook delivers to every registered pipe
            // process-wide).
            let _guard = SIGWINCH_TEST_MUTEX
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let (mut wait, _waker) = for_test_sigwinch().expect("for_test_sigwinch");
            signal_hook::low_level::raise(SIGWINCH).expect("raise SIGWINCH");
            let outcome = wait.wait(Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(outcome, WaitOutcome::Woken);
        }

        #[test]
        fn poll_timeout_clamps() {
            assert_eq!(to_poll_timeout(None), PollTimeout::NONE);
            assert_eq!(
                to_poll_timeout(Some(Duration::from_millis(2))),
                PollTimeout::try_from(2u32).expect("2ms fits")
            );
            // Duration::MAX massively exceeds i32::MAX ms -> falls back to
            // MAX rather than panicking.
            assert_eq!(to_poll_timeout(Some(Duration::MAX)), PollTimeout::MAX);
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: WaitForMultipleObjects over [console handle, auto-reset event].
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
    };

    use super::WaitOutcome;

    pub(super) fn event_wait_pair() -> io::Result<(EventWait, EventWaker)> {
        // SAFETY: all arguments are default/null per Win32 documentation —
        // no security attributes, auto-reset (bManualReset = FALSE),
        // initially unsignaled (bInitialState = FALSE), unnamed.
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let event = Arc::new(EventHandle(handle));

        // SAFETY: `STD_INPUT_HANDLE` is a valid standard-handle constant;
        // `GetStdHandle` with it never fails destructively. A null or
        // invalid result just means "no console" (headless), handled below.
        let stdin_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let stdin = (stdin_handle != INVALID_HANDLE_VALUE && !stdin_handle.is_null())
            .then_some(stdin_handle);

        Ok((
            EventWait {
                stdin,
                event: Arc::clone(&event),
            },
            EventWaker(event),
        ))
    }

    /// RAII wrapper around a Win32 event `HANDLE` so it closes exactly
    /// once, regardless of how many `EventWaker`/`EventWait` clones share it
    /// via `Arc`.
    struct EventHandle(HANDLE);

    // SAFETY: a Win32 event handle is a process-global kernel object;
    // signaling (`SetEvent`) and waiting on it from any thread is an
    // explicitly documented, supported use — that is the entire purpose of
    // Win32 event objects.
    unsafe impl Send for EventHandle {}
    unsafe impl Sync for EventHandle {}

    impl Drop for EventHandle {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `CreateEventW` in
            // `event_wait_pair` and is closed exactly once, here (the last
            // `Arc` clone to drop).
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) struct EventWait {
        stdin: Option<HANDLE>,
        event: Arc<EventHandle>,
    }

    impl EventWait {
        pub(super) fn wait(&mut self, timeout: Option<Duration>) -> io::Result<WaitOutcome> {
            wait_on_handles(self.stdin, self.event.0, timeout)
        }
    }

    #[derive(Clone)]
    pub(super) struct EventWaker(Arc<EventHandle>);

    impl EventWaker {
        pub(super) fn wake(&self) {
            // SAFETY: `self.0.0` is a valid event handle for the lifetime of
            // this `Arc` (closed only when the last clone drops). `SetEvent`
            // on an already-signaled auto-reset event is a documented
            // no-op — exactly the coalescing behavior this primitive wants.
            unsafe {
                SetEvent(self.0.0);
            }
        }
    }

    /// The testable core of `wait`: one `WaitForMultipleObjects` call over
    /// 1-2 handles, with no console/thread state — tests substitute a
    /// second event for the console handle (an anonymous pipe is not a
    /// waitable Win32 object, so it cannot stand in here).
    fn wait_on_handles(
        input: Option<HANDLE>,
        event: HANDLE,
        timeout: Option<Duration>,
    ) -> io::Result<WaitOutcome> {
        let timeout_ms = to_wait_ms(timeout);
        let (handles, count): ([HANDLE; 2], u32) = match input {
            Some(h) => ([h, event], 2),
            None => ([event, std::ptr::null_mut()], 1),
        };
        // SAFETY: `handles[..count]` are valid, open, waitable handles for
        // the duration of this call.
        let r = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, timeout_ms) };
        match r {
            WAIT_TIMEOUT => Ok(WaitOutcome::TimedOut),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => {
                let index = r - WAIT_OBJECT_0;
                if input.is_some() && index == 0 {
                    Ok(WaitOutcome::Input)
                } else {
                    Ok(WaitOutcome::Woken)
                }
            }
        }
    }

    fn to_wait_ms(timeout: Option<Duration>) -> u32 {
        match timeout {
            None => INFINITE,
            // Clamp one below `INFINITE` (`u32::MAX`) so a huge duration
            // can never accidentally collide with the "wait forever"
            // sentinel.
            Some(d) => u32::try_from(d.as_millis()).unwrap_or(u32::MAX - 1),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn create_event() -> HANDLE {
            // SAFETY: same invariants as `event_wait_pair`'s `CreateEventW`
            // call; test-owned handles are closed explicitly below.
            let h = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            assert!(
                !h.is_null() && h != INVALID_HANDLE_VALUE,
                "CreateEventW failed"
            );
            h
        }

        fn close(h: HANDLE) {
            // SAFETY: `h` was created by `create_event` above and is closed
            // exactly once, by the caller, at the end of each test.
            unsafe {
                CloseHandle(h);
            }
        }

        #[test]
        fn wake_sets_event_returns_woken() {
            let event = create_event();
            unsafe { SetEvent(event) };
            let outcome = wait_on_handles(None, event, Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(outcome, WaitOutcome::Woken);
            close(event);
        }

        #[test]
        fn timeout_returns_timed_out() {
            let event = create_event();
            let outcome =
                wait_on_handles(None, event, Some(Duration::from_millis(10))).expect("wait");
            assert_eq!(outcome, WaitOutcome::TimedOut);
            close(event);
        }

        #[test]
        fn input_signal_wins_even_if_event_also_signaled() {
            let fake_input = create_event();
            let event = create_event();
            unsafe {
                SetEvent(fake_input);
                SetEvent(event);
            }
            let outcome = wait_on_handles(Some(fake_input), event, Some(Duration::from_secs(1)))
                .expect("wait");
            assert_eq!(outcome, WaitOutcome::Input);
            close(fake_input);
            close(event);
        }

        #[test]
        fn auto_reset_coalesces() {
            let event = create_event();
            unsafe {
                SetEvent(event);
                // A second SetEvent on an already-signaled auto-reset event
                // is a documented no-op.
                SetEvent(event);
            }
            let first = wait_on_handles(None, event, Some(Duration::from_secs(1))).expect("wait");
            assert_eq!(first, WaitOutcome::Woken);
            let second =
                wait_on_handles(None, event, Some(Duration::from_millis(10))).expect("wait");
            assert_eq!(
                second,
                WaitOutcome::TimedOut,
                "auto-reset must consume the signal on the winning wait"
            );
            close(event);
        }
    }
}

// ---------------------------------------------------------------------------
// Fallback: no known wait primitive — degrade to plain sleeping.
// ---------------------------------------------------------------------------

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::io;
    use std::time::Duration;

    use super::WaitOutcome;

    pub(super) struct EventWait;

    #[derive(Clone)]
    pub(super) struct EventWaker;

    pub(super) fn event_wait_pair() -> io::Result<(EventWait, EventWaker)> {
        Ok((EventWait, EventWaker))
    }

    impl EventWait {
        pub(super) fn wait(&mut self, timeout: Option<Duration>) -> io::Result<WaitOutcome> {
            std::thread::sleep(timeout.unwrap_or(Duration::from_millis(100)));
            Ok(WaitOutcome::TimedOut)
        }
    }

    impl EventWaker {
        pub(super) fn wake(&self) {}
    }
}
