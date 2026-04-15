use std::io::{self, stdout, Stdout, Write};

use crossterm::{
    execute,
    cursor::SetCursorStyle,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use engine::types::EditorMode;
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
/// Mouse tracking is enabled selectively:
/// - `mouse_enabled` enables normal tracking (button press/release + scroll,
///   `\x1b[?1000h`) plus SGR extended coordinates (`\x1b[?1006h`). With only
///   these modes, drag events are NOT sent to the application, so the terminal
///   handles drag-select natively.
/// - `mouse_select` additionally enables button-event tracking (`\x1b[?1002h`),
///   which sends drag events so the editor can create editor selections on drag.
///
/// Returns `(Term, kitty_enabled)`. When `kitty_enabled` is `true`, the editor
/// should activate Ctrl+motion extend shortcuts and filter `KeyEventKind::Release`
/// events from the event loop.
///
/// Call [`restore`] (or let the panic hook do it) before the process exits so
/// the user's shell is left in a usable state.
pub(crate) fn init(mouse_enabled: bool, mouse_select: bool) -> io::Result<(Term, bool)> {
    enable_raw_mode()?;
    let mut out = stdout();

    let kitty_enabled = crate::os::probe_kitty_support().unwrap_or(false);
    if kitty_enabled {
        // REPORT_ALTERNATE_KEYS is required so that Ctrl+shifted-chars
        // (e.g. Ctrl+}) arrive with the correct keycode instead of the base
        // key plus SHIFT. See docs/learning/command-keymap-dispatch.md.
        //
        // Known limitation: WezTerm 20240203-110809-5046fc22 does not fully
        // support REPORT_ALTERNATE_KEYS — Ctrl+shifted-char one-shot extend
        // may not work on that version.
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
            )
        )?;
    }

    if mouse_enabled {
        // Normal tracking (1000): button press/release and scroll wheel.
        // SGR extended coordinates (1006): removes the 223-column limit of the
        // legacy X10 encoding; required for wide terminals.
        // We deliberately do NOT enable button-event tracking (1002, which
        // also reports drag motion) unless `mouse_select` is true. Without
        // 1002, drag events never reach the application, so the terminal
        // handles drag-select natively.
        out.write_all(b"\x1b[?1000h\x1b[?1006h")?;
        if mouse_select {
            out.write_all(b"\x1b[?1002h")?;
        }
        out.flush()?;
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
    // Disable all mouse tracking modes. The `l` (low) sequences are harmless
    // no-ops if the corresponding mode was never enabled.
    try_op(stdout().write_all(b"\x1b[?1002l\x1b[?1000l\x1b[?1006l"));
    try_op(stdout().flush());
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

/// Emit an OSC 12 sequence to set the terminal cursor colour for `mode`.
///
/// Command/Search mode positions the cursor in the statusline, which has a
/// white background — a default (white) cursor would be invisible. We set it
/// to black so it contrasts. All other modes reset to the terminal default.
///
/// OSC 12 (`\x1b]12;COLOR\x07`) is supported by the overwhelming majority of
/// modern terminal emulators. The reset form (`\x1b]112;\x07`) restores the
/// user's configured cursor colour.
pub(crate) fn set_cursor_color_for_mode(mode: EditorMode) -> io::Result<()> {
    let seq: &[u8] = match mode {
        EditorMode::Command | EditorMode::Search => b"\x1b]12;black\x07",
        _ => b"\x1b]112;\x07",
    };
    stdout().write_all(seq)
}

/// Emit a crossterm `SetCursorStyle` escape for the cursor shape appropriate
/// to `mode`.
///
/// Bar modes (Insert, Command, Search, Select) get `SteadyBar`; all others
/// get `SteadyBlock`. Relies on [`crate::cursor::shape`] for the mode-to-style
/// mapping.
pub(crate) fn set_cursor_shape(mode: EditorMode) -> io::Result<()> {
    execute!(stdout(), crate::cursor::shape(mode))
}

/// Emit the `DefaultUserShape` escape, restoring whatever cursor shape the
/// user's terminal is configured to display.
///
/// Call this before returning to the shell so the user's preferred cursor is
/// restored.
pub(crate) fn reset_cursor_shape() -> io::Result<()> {
    execute!(stdout(), SetCursorStyle::DefaultUserShape)
}
