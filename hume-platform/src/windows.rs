//! Windows console control events: which ones end HUME, and the handler
//! that acts on them. The Windows counterpart of `unix.rs`'s terminator.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::core::BOOL;

use crate::terminal::SharedTerm;

/// How long Windows lets a process handle `CTRL_CLOSE_EVENT` before it ends
/// the process: the documented default of `SPI_GETHUNGAPPTIMEOUT`.
const CLOSE_EVENT_TIMEOUT: Duration = Duration::from_millis(5000);

// The quit runs inside the close event's handler, so it must be over before
// Windows ends the process.
const _: () = assert!(crate::QUIT_GRACE.as_millis() < CLOSE_EVENT_TIMEOUT.as_millis());

/// What the handler needs to quit: the terminal to restore, and the
/// editor's quit request, shared by every handler thread.
struct Terminator {
    term: SharedTerm,
    request_quit: Mutex<Box<dyn Fn(i32) + Send>>,
}

static TERMINATOR: OnceLock<Terminator> = OnceLock::new();

/// Claimed by the first event that ends HUME. Every later one parks.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Whether console event `ctrl_type` ends HUME. Inside the inline-output
/// frame the console sends Ctrl-c and Ctrl-Break to the child running there
/// as well, and HUME drops them, as `unix::terminating_signal` drops SIGINT
/// and SIGQUIT. Closing the console, logging off and shutting down always
/// end it.
fn ends_process(ctrl_type: u32, inline_output: bool) -> bool {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => !inline_output,
        _ => true,
    }
}

/// Runs on a thread Windows creates for each event. An event that ends HUME
/// never returns: on `CTRL_CLOSE_EVENT` Windows ends the process as soon as
/// the handler returns, so returning would cut the quit short. A later event
/// parks for the same reason, as `restore_for_exit`'s losing caller does.
extern "system" fn console_handler(ctrl_type: u32) -> BOOL {
    let Some(terminator) = TERMINATOR.get() else {
        return FALSE;
    };
    if !ends_process(ctrl_type, terminator.term.in_inline_output()) {
        return TRUE;
    }
    if QUITTING.swap(true, Ordering::AcqRel) {
        loop {
            std::thread::park();
        }
    }
    let code = crate::WINDOWS_SIGNAL_EXIT_CODE;
    {
        let request_quit = terminator
            .request_quit
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (*request_quit)(code);
    }
    // The main loop had a full grace window and didn't exit the process
    // itself, so force it down.
    std::thread::sleep(crate::QUIT_GRACE);
    crate::force_exit(&terminator.term, code)
}

/// Installs [`console_handler`] for the life of the process.
pub(super) fn install_terminator(
    term: SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> io::Result<()> {
    TERMINATOR
        .set(Terminator {
            term,
            request_quit: Mutex::new(Box::new(request_quit)),
        })
        .map_err(|_| io::Error::other("the console terminator is already installed"))?;
    // SAFETY: `console_handler` is a valid handler routine for the life of
    // the process, and adding one has no other precondition.
    if unsafe { SetConsoleCtrlHandler(Some(console_handler), TRUE) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Console::{
        CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };

    use super::*;

    const INTERRUPTS: [u32; 2] = [CTRL_C_EVENT, CTRL_BREAK_EVENT];
    const TERMINATIONS: [u32; 3] = [CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT];

    #[test]
    fn every_event_ends_hume_outside_the_inline_output_frame() {
        for event in INTERRUPTS.into_iter().chain(TERMINATIONS) {
            assert!(ends_process(event, false), "event {event}");
        }
    }

    #[test]
    fn interrupts_are_dropped_inside_the_inline_output_frame() {
        for event in INTERRUPTS {
            assert!(!ends_process(event, true), "event {event}");
        }
    }

    #[test]
    fn closing_the_console_ends_hume_inside_the_inline_output_frame() {
        for event in TERMINATIONS {
            assert!(ends_process(event, true), "event {event}");
        }
    }
}
