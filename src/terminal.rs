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

    // Probe and activate kitty keyboard protocol while stdin is in raw mode
    // (required for the response to be readable). Errors treated as "not supported"
    // so misconfigured terminals fall back gracefully.
    let kitty_enabled = supports_keyboard_enhancement().unwrap_or(false);
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
