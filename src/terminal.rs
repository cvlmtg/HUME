use std::io::{self, stdout, Stdout};

use crossterm::{
    execute,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
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

    let kitty_enabled = crate::os::probe_kitty_support().unwrap_or(false);
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
/// All three operations are attempted even if an earlier one fails — the goal
/// is to leave the shell as usable as possible. The first error encountered is
/// returned; subsequent errors are silently discarded.
pub(crate) fn restore() -> io::Result<()> {
    let mut first_err: Option<io::Error> = None;
    let mut try_op = |r: io::Result<()>| {
        if first_err.is_none() {
            first_err = r.err();
        }
    };

    // Pop kitty keyboard protocol first. Harmless on legacy terminals — no
    // flags were pushed, so the sequence is silently ignored.
    try_op(execute!(stdout(), PopKeyboardEnhancementFlags));
    // Disable raw mode before leaving the alternate screen so the shell stays
    // usable even if LeaveAlternateScreen fails.
    try_op(disable_raw_mode());
    try_op(execute!(stdout(), LeaveAlternateScreen));

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
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
