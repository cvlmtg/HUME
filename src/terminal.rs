use std::io::{self, stdout, Stdout};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

/// A ratatui `Terminal` backed by crossterm on stdout.
///
/// Aliased here so every other module can name the type without repeating the
/// backend parameter.
pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Switch the terminal into raw mode + alternate screen and create a ratatui
/// `Terminal`.
///
/// Call [`restore`] (or let the panic hook do it) before the process exits so
/// the user's shell is left in a usable state.
pub(crate) fn init() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

/// Undo everything [`init`] did: leave the alternate screen and disable raw
/// mode.
///
/// Safe to call multiple times — both operations are idempotent on every major
/// platform.
pub(crate) fn restore() -> io::Result<()> {
    // Disable raw mode first: if LeaveAlternateScreen fails we at least
    // restore the input mode so the shell stays usable.
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
