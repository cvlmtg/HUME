use std::io::{self, stdout, Stdout};

use crossterm::{
    execute,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, supports_keyboard_enhancement},
};
use ratatui::{backend::CrosstermBackend, Terminal};

/// A ratatui `Terminal` backed by crossterm on stdout.
///
/// Aliased here so every other module can name the type without repeating the
/// backend parameter.
pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Switch the terminal into raw mode + alternate screen and create a ratatui
/// `Terminal`. Also probes for kitty keyboard protocol support and enables it
/// if available.
///
/// Returns `(Term, kitty_enabled)`. When `kitty_enabled` is `true`, the editor
/// should activate Ctrl+motion extend shortcuts and filter `KeyEventKind::Release`
/// events from the event loop.
///
/// Call [`restore`] (or let the panic hook do it) before the process exits so
/// the user's shell is left in a usable state.
pub(crate) fn init() -> io::Result<(Term, bool)> {
    enable_raw_mode()?;
    let mut out = stdout();

    // Check well-known env vars first: zero-cost and avoids the CSI probe's
    // read timeout on terminals that are known to support kitty keyboard protocol
    // (e.g. WezTerm sometimes times out on the probe even though it supports it).
    // Fall back to the CSI probe only for unrecognised terminals.
    let kitty_enabled = is_known_kitty_terminal()
        || supports_keyboard_enhancement().unwrap_or(false);
    if kitty_enabled {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        )?;
    }

    execute!(out, EnterAlternateScreen)?;
    let term = Terminal::new(CrosstermBackend::new(out))?;
    Ok((term, kitty_enabled))
}

/// Undo everything [`init`] did: pop the kitty keyboard flags (harmless no-op
/// on legacy terminals), leave the alternate screen, and disable raw mode.
///
/// Safe to call multiple times — all operations are idempotent on every major
/// platform. The unconditional `PopKeyboardEnhancementFlags` sends `\x1b[<u`,
/// which legacy terminals ignore.
pub(crate) fn restore() -> io::Result<()> {
    // Pop kitty keyboard protocol before anything else. The sequence is
    // harmless on legacy terminals (no state was pushed, the bytes are ignored).
    execute!(stdout(), PopKeyboardEnhancementFlags)?;
    // Disable raw mode before leaving the alternate screen: if LeaveAlternateScreen
    // fails we at least restore the input mode so the shell stays usable.
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Detect kitty keyboard protocol support via environment variables.
///
/// Checked before the CSI `\x1B[?u` probe to avoid its read timeout on
/// terminals that are known supporters. Each terminal sets a documented env var:
///
/// | Terminal | Variable |
/// |----------|----------|
/// | WezTerm  | `WEZTERM_EXECUTABLE` or `TERM_PROGRAM=WezTerm` |
/// | kitty    | `KITTY_WINDOW_ID` or `TERM=xterm-kitty` |
/// | ghostty  | `GHOSTTY_RESOURCES_DIR` or `TERM_PROGRAM=ghostty` |
/// | foot     | `FOOT_SERVER_SOCKET` or `TERM=foot` |
fn is_known_kitty_terminal() -> bool {
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();
    std::env::var("WEZTERM_EXECUTABLE").is_ok()
        || std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok()
        || std::env::var("FOOT_SERVER_SOCKET").is_ok()
        || matches!(term_program.as_str(), "WezTerm" | "ghostty")
        || matches!(term.as_str(), "xterm-kitty" | "foot" | "xterm-ghostty")
}

/// Install a panic hook that restores the terminal before printing the panic
/// message.
///
/// Without this, a panic leaves the terminal in raw mode / alternate screen
/// and the user sees nothing (or a garbled shell). Call once at the top of
/// `main` before any other setup.
pub(crate) fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort — ignore errors; we're already panicking.
        let _ = restore();
        original(info);
    }));
}
