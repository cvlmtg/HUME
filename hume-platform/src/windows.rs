use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::FromRawHandle;
use std::time::Instant;

use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Console::{
            ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
            GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
        },
        Threading::WaitForSingleObject,
    },
};

/// Probe for kitty keyboard protocol support on Windows.
///
/// Temporarily enables `ENABLE_VIRTUAL_TERMINAL_INPUT` on stdin (so terminal
/// replies arrive as raw VT bytes via `ReadFile` instead of translated
/// `KEY_EVENT` records) and `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on stdout
/// (so the probe escape sequences are interpreted), then builds a
/// [`WinChannel`] over the console handles and delegates the query/response
/// loop to [`super::run_probe`]. Original console modes are restored on every
/// exit path.
///
/// Under ConPTY the raw VT bytes pass straight through to the hosting terminal
/// (WezTerm, Windows Terminal, …) which interprets the kitty/XTVERSION/DA1
/// queries natively.
///
/// Must be called after `enable_raw_mode()`.
pub(super) fn probe_kitty_support() -> io::Result<bool> {
    // SAFETY: all Win32 calls are on valid handles obtained from GetStdHandle.
    // All allocations are on the stack. Modes are restored on every exit path.
    unsafe {
        let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let stdin_handle = GetStdHandle(STD_INPUT_HANDLE);

        if stdout_handle == INVALID_HANDLE_VALUE || stdin_handle == INVALID_HANDLE_VALUE {
            return Ok(false);
        }

        // Save original console modes so we can restore them unconditionally.
        let mut orig_out_mode = 0u32;
        let mut orig_in_mode = 0u32;
        if GetConsoleMode(stdout_handle, &mut orig_out_mode) == 0
            || GetConsoleMode(stdin_handle, &mut orig_in_mode) == 0
        {
            return Ok(false);
        }

        // Enable VT processing on stdout so the terminal interprets the probe.
        // (crossterm enables this when entering the alt screen, but we probe
        // before that, so we set it explicitly here.) If either SetConsoleMode
        // fails, VT sequences won't work, so bail out rather than sending bytes
        // into the void.
        if SetConsoleMode(
            stdout_handle,
            orig_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        ) == 0
        {
            return Ok(false);
        }

        // Enable VT input on stdin so the terminal's response arrives as raw
        // bytes via ReadFile rather than as translated KEY_EVENT records.
        if SetConsoleMode(stdin_handle, orig_in_mode | ENABLE_VIRTUAL_TERMINAL_INPUT) == 0 {
            // Restore stdout mode before returning.
            SetConsoleMode(stdout_handle, orig_out_mode);
            return Ok(false);
        }

        let mut ch = WinChannel::new(stdout_handle, stdin_handle);
        let result = super::run_probe(&mut ch);

        // Restore original modes unconditionally regardless of probe outcome.
        SetConsoleMode(stdin_handle, orig_in_mode);
        SetConsoleMode(stdout_handle, orig_out_mode);

        result
    }
}

/// [`super::ProbeChannel`] backed by Windows console handles obtained from
/// `GetStdHandle`, using `WaitForSingleObject` to wait for stdin readiness
/// with a deadline.
///
/// `ManuallyDrop<File>` wraps the borrowed handles so the `File` destructor
/// does not close them — `GetStdHandle` returns pseudo-handles owned by the
/// process, not by us.
struct WinChannel {
    stdout: ManuallyDrop<File>,
    stdin: ManuallyDrop<File>,
    stdin_handle: windows_sys::Win32::Foundation::HANDLE,
}

impl WinChannel {
    /// # Safety
    /// Both handles must be valid Win32 console handles obtained from
    /// `GetStdHandle` (caller-owned; we must not close them).
    unsafe fn new(
        stdout_handle: windows_sys::Win32::Foundation::HANDLE,
        stdin_handle: windows_sys::Win32::Foundation::HANDLE,
    ) -> Self {
        // SAFETY: caller guarantees both handles are valid Win32 console
        // handles from GetStdHandle. ManuallyDrop prevents File::drop from
        // closing them.
        Self {
            stdout: ManuallyDrop::new(unsafe { File::from_raw_handle(stdout_handle as _) }),
            stdin: ManuallyDrop::new(unsafe { File::from_raw_handle(stdin_handle as _) }),
            stdin_handle,
        }
    }
}

impl super::ProbeChannel for WinChannel {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stdout.write_all(buf)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stdin.read(buf)
    }

    fn wait_until(&mut self, deadline: Instant) -> io::Result<bool> {
        let remaining_ms = deadline
            .checked_duration_since(Instant::now())
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        if remaining_ms == 0 {
            return Ok(false);
        }
        // SAFETY: stdin_handle is a valid Win32 HANDLE obtained from
        // GetStdHandle (invariant of `WinChannel::new`).
        let r = unsafe { WaitForSingleObject(self.stdin_handle, remaining_ms) };
        // WAIT_OBJECT_0 = signaled (input ready). WAIT_TIMEOUT / WAIT_FAILED
        // both collapse to "not ready"; the shared loop treats the former as a
        // deadline expiry and the latter semantics match the previous impl.
        Ok(r == WAIT_OBJECT_0)
    }
}
