use std::io;
use std::time::{Duration, Instant};

use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode,
            ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        },
        Threading::WaitForSingleObject,
    },
};

/// Probe for kitty keyboard protocol support on Windows.
///
/// Uses the same DA1-sentinel method as the Unix probe: sends `\x1B[?u`
/// (kitty keyboard protocol flags query), `\x1B[>q` (XTVERSION — terminal
/// name/version, fallback for terminals that support kitty push but don't
/// answer the flags query), and `\x1B[c` (DA1 sentinel). A terminal supporting
/// the protocol responds with `\x1B[?<flags>u`; one that doesn't responds only
/// with the XTVERSION and DA1 replies.
///
/// We temporarily enable `ENABLE_VIRTUAL_TERMINAL_INPUT` on stdin so that the
/// terminal's response arrives as raw VT bytes via `ReadFile`, and restore the
/// original mode unconditionally before returning.
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

        let result = run_probe(stdout_handle, stdin_handle);

        // Restore original modes unconditionally regardless of probe outcome.
        SetConsoleMode(stdin_handle, orig_in_mode);
        SetConsoleMode(stdout_handle, orig_out_mode);

        result
    }
}

unsafe fn run_probe(
    stdout_handle: windows_sys::Win32::Foundation::HANDLE,
    stdin_handle: windows_sys::Win32::Foundation::HANDLE,
) -> io::Result<bool> {
    // Send three queries together: kitty flags query, XTVERSION (fallback
    // identification for terminals that support push but not query), DA1 sentinel.
    let query = b"\x1B[?u\x1B[>q\x1B[c";
    let mut written = 0u32;
    if WriteFile(
        stdout_handle,
        query.as_ptr(),
        query.len() as u32,
        &mut written,
        std::ptr::null_mut(),
    ) == 0
    {
        return Ok(false);
    }

    // Read the response with a deadline. Use a generous initial budget (500 ms)
    // to accommodate slow terminals; once data starts flowing everything arrives
    // quickly, so remaining time naturally bounds the subsequent reads.
    let mut response = Vec::with_capacity(256);
    let mut buf = [0u8; 256]; // large enough for kitty + XTVERSION + DA1
    let deadline = Instant::now() + Duration::from_millis(500);

    loop {
        let remaining_ms = deadline
            .checked_duration_since(Instant::now())
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        if remaining_ms == 0 {
            break;
        }

        if WaitForSingleObject(stdin_handle, remaining_ms) != WAIT_OBJECT_0 {
            break; // timeout or error
        }

        let mut bytes_read = 0u32;
        if ReadFile(
            stdin_handle,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut bytes_read,
            std::ptr::null_mut(),
        ) == 0
            || bytes_read == 0
        {
            break;
        }

        response.extend_from_slice(&buf[..bytes_read as usize]);

        // Keep reading until we see a complete DA1 response (ESC [ ? ... c),
        // which is the last thing the terminal sends.
        if super::has_da1_response(&response) {
            break;
        }
    }

    Ok(super::has_kitty_response(&response) || super::has_kitty_xtversion(&response))
}
