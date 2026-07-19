//! Terminal lifecycle management.
//!
//! Entry point is [`init`]: enables raw mode, enters the alternate screen, and
//! returns a ratatui [`Term`] ready to render. [`restore`] reverses all of
//! that and must be called before the process exits.
//!
//! [`probe_kitty`] is a separate step, called *before* [`init`]. It runs on
//! the normal screen (raw mode only, no alt-screen) so the caller can finish
//! scripting initialisation — which installs kitty-only default keybinds
//! before user `bind-key!` calls run — while the shell is still visible.
//!
//! Also provides cursor shape/colour control, DEC 2026 synchronized-update
//! framing, and the inline-subprocess-output flow
//! ([`enter_inline_output`]/[`leave_inline_output`]).

use std::io::{self, BufWriter, Stdout, Write, stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{Event, KeyEventKind, KeyboardEnhancementFlags, read},
    execute,
    terminal::{
        BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

/// A ratatui `Terminal` backed by crossterm on stdout.
///
/// Aliased here so every other module can name the type without repeating the
/// backend parameter.
pub type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

// ── Shared escape-sequence helpers ───────────────────────────────────────────

fn push_kitty_flags(out: &mut impl io::Write) -> io::Result<()> {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    // Bypass crossterm's PushKeyboardEnhancementFlags Command: its Windows
    // path hardcodes is_ansi_code_supported()=false and returns Unsupported,
    // but ConPTY passes raw VT through to WezTerm/kitty/ghostty/foot which
    // interpret the kitty keyboard protocol natively. Emits the same CSI
    // crossterm's own write_ansi would (`\x1b[>{bits}u`).
    write!(out, "\x1b[>{}u", flags.bits())?;
    out.flush()
}

fn pop_kitty_flags(out: &mut impl io::Write) -> io::Result<()> {
    // Pop one level of the keyboard enhancement stack. Fixed protocol
    // sequence (no flag argument); harmless no-op when stack is empty.
    out.write_all(b"\x1b[<1u")?;
    out.flush()
}

fn enable_mouse(out: &mut Stdout, select: bool) -> io::Result<()> {
    out.write_all(b"\x1b[?1000h\x1b[?1006h")?;
    if select {
        out.write_all(b"\x1b[?1002h")?;
    }
    out.flush()
}

fn disable_mouse(out: &mut Stdout) -> io::Result<()> {
    out.write_all(b"\x1b[?1002l\x1b[?1000l\x1b[?1006l")?;
    out.flush()
}

fn enable_bracketed_paste(out: &mut impl io::Write) -> io::Result<()> {
    // Bypass crossterm's EnableBracketedPaste Command for the same reason as
    // push_kitty_flags: a raw write is a harmless no-op on terminals without
    // DEC mode 2004, with no platform-specific failure path to route around.
    out.write_all(b"\x1b[?2004h")?;
    out.flush()
}

fn disable_bracketed_paste(out: &mut impl io::Write) -> io::Result<()> {
    out.write_all(b"\x1b[?2004l")?;
    out.flush()
}

// ── Public terminal lifecycle API ─────────────────────────────────────────────

/// Probe for kitty keyboard protocol support on the normal screen.
///
/// Enables raw mode (required to read the terminal's reply to the probe
/// query), runs the probe, then disables raw mode again before returning —
/// callers get a cooked terminal back on every path, since scripting
/// initialisation (which may spawn subprocesses, e.g. grammar installs) runs
/// on the normal screen between this call and [`init`].
///
/// Probe failures (channel errors, not mere timeouts) are surfaced to the
/// user as a one-line stderr hint: kitty support degrades to "off" but the
/// editor still starts. A plain timeout reports `Ok(false)` upstream.
pub fn probe_kitty() -> io::Result<bool> {
    enable_raw_mode()?;
    let kitty_enabled = match crate::probe_kitty_support() {
        Ok(v) => v,
        Err(e) => {
            // Disable raw mode so the hint prints with normal line discipline
            // (no staircase) before we report it.
            let _ = disable_raw_mode();
            eprintln!("hume: kitty keyboard probe failed: {e}");
            return Ok(false);
        }
    };
    disable_raw_mode()?;
    Ok(kitty_enabled)
}

/// Switch the terminal into raw mode + alternate screen and create a ratatui
/// `Terminal`.
///
/// `kitty_enabled` is the result of a prior [`probe_kitty`] call. When `true`,
/// the caller should filter `KeyEventKind::Release` events from the event
/// loop and may enable Ctrl-modified key bindings that require the enhanced
/// protocol.
///
/// Mouse tracking is enabled selectively:
/// - `mouse_enabled` enables normal tracking (button press/release + scroll,
///   `\x1b[?1000h`) plus SGR extended coordinates (`\x1b[?1006h`). With only
///   these modes, drag events are NOT sent to the application, so the terminal
///   handles drag-select natively.
/// - `mouse_select` additionally enables button-event tracking (`\x1b[?1002h`),
///   which sends drag events so the editor can create editor selections on drag.
///
/// Call [`restore`] (or let the panic hook do it) before the process exits so
/// the user's shell is left in a usable state.
pub fn init(mouse_enabled: bool, mouse_select: bool, kitty_enabled: bool) -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();

    // Enter alternate screen before pushing kitty flags.  Some terminals
    // (WezTerm, kitty) maintain a per-screen keyboard stack; the push must
    // land on the alternate screen's stack so that key reads (which consult
    // the active screen) pick up the enhanced encoding.
    execute!(out, EnterAlternateScreen)?;
    enable_bracketed_paste(&mut out)?;

    if kitty_enabled {
        // REPORT_ALTERNATE_KEYS is required so that Ctrl+shifted-chars
        // (e.g. Ctrl+}) arrive with the correct keycode instead of the base
        // key plus SHIFT. See docs/learning/command-keymap-dispatch.md.
        //
        // Known limitation: WezTerm 20240203-110809-5046fc22 does not fully
        // support REPORT_ALTERNATE_KEYS — Ctrl+shifted-char one-shot extend
        // may not work on that version.
        push_kitty_flags(&mut out)?;
    }

    if mouse_enabled {
        // Normal tracking (1000): button press/release and scroll wheel.
        // SGR extended coordinates (1006): removes the 223-column limit of the
        // legacy X10 encoding; required for wide terminals.
        // We deliberately do NOT enable button-event tracking (1002, which
        // also reports drag motion) unless `mouse_select` is true. Without
        // 1002, drag events never reach the application, so the terminal
        // handles drag-select natively.
        enable_mouse(&mut out, mouse_select)?;
    }
    let term = Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(
        64 * 1024,
        out,
    )))?;
    Ok(term)
}

/// Undo everything [`init`] did: pop the kitty keyboard flags (harmless no-op
/// on legacy terminals), leave the alternate screen, and disable raw mode.
///
/// All three operations are attempted even if an earlier one fails — the goal
/// is to leave the shell as usable as possible. The first error encountered is
/// returned; subsequent errors are silently discarded.
pub fn restore() -> io::Result<()> {
    let mut first_err: Option<io::Error> = None;
    let mut try_op = |r: io::Result<()>| {
        if first_err.is_none() {
            first_err = r.err();
        }
    };

    // Close any deferred-paint envelope first: a panic mid-frame can leave the
    // terminal expecting an EndSynchronizedUpdate, and most (but not all) emit
    // the held buffer on alt-screen exit. Sending it explicitly is harmless if
    // no envelope was open.
    try_op(execute!(stdout(), EndSynchronizedUpdate));
    // Disable bracketed paste before leaving the alt screen — must not leak
    // into the shell, where a subsequent paste would dump raw `\x1b[200~`
    // markers into the prompt.
    try_op(disable_bracketed_paste(&mut stdout()));
    // Pop kitty keyboard protocol. Harmless on legacy terminals — the pop
    // is a no-op if the stack is empty.
    try_op(pop_kitty_flags(&mut stdout()));
    // Disable all mouse tracking modes. The `l` (low) sequences are harmless
    // no-ops if the corresponding mode was never enabled.
    try_op(disable_mouse(&mut stdout()));
    // Disable raw mode before leaving the alternate screen so the shell stays
    // usable even if LeaveAlternateScreen fails.
    try_op(disable_raw_mode());
    try_op(execute!(stdout(), LeaveAlternateScreen));
    // Second pop after leaving the alternate screen. Since `init()` now
    // pushes onto the alt screen's stack, the first pop (above) clears it.
    // This extra pop handles terminals with a global keyboard stack — it is
    // a harmless no-op on per-screen-buffer terminals (WezTerm, kitty).
    try_op(pop_kitty_flags(&mut stdout()));

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// RAII guard that calls [`restore`] when dropped. Ensures the terminal is
/// returned to a sane state on every exit path — clean return, `?`-propagated
/// error, panic unwinding.
///
/// Declare the guard *before* the `Terminal` value so that on unwind the
/// `Terminal`'s `BufWriter` flushes first (any buffered render bytes hit the
/// alt screen while it is still active) and [`restore`] runs last.
///
/// After an explicit [`restore`]`()?` on the happy path, call [`disarm`] to
/// suppress the drop-time call.
///
/// [`disarm`]: TerminalGuard::disarm
pub struct TerminalGuard {
    armed: bool,
}

impl TerminalGuard {
    pub fn new() -> Self {
        Self { armed: true }
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Default for TerminalGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.armed
            && let Err(e) = restore()
        {
            eprintln!("hume: terminal restore failed: {e}");
        }
    }
}

/// Emit an OSC 12 sequence to set the terminal cursor colour.
///
/// When `black` is `true`, sets the cursor to black — used when the cursor
/// sits on a light-background surface (e.g. the statusline in Command/Search
/// mode) where the default colour would be invisible.
///
/// When `black` is `false`, resets to the user's configured terminal default.
///
/// OSC 12 (`\x1b]12;COLOR\x07`) is supported by the overwhelming majority of
/// modern terminal emulators. The reset form (`\x1b]112;\x07`) restores the
/// user's configured cursor colour.
pub fn set_cursor_color(black: bool) -> io::Result<()> {
    let seq: &[u8] = if black {
        b"\x1b]12;black\x07"
    } else {
        b"\x1b]112;\x07"
    };
    stdout().write_all(seq)
}

/// Emit a crossterm `SetCursorStyle` escape for the cursor shape.
///
/// When `bar` is `true`, emits `SteadyBar` (used for Insert/Command/Search/Select).
/// When `bar` is `false`, emits `SteadyBlock` (used for Normal/Extend).
pub fn set_cursor_shape(bar: bool) -> io::Result<()> {
    let style = if bar {
        SetCursorStyle::SteadyBar
    } else {
        SetCursorStyle::SteadyBlock
    };
    execute!(stdout(), style)
}

/// Emit the `DefaultUserShape` escape, restoring whatever cursor shape the
/// user's terminal is configured to display.
///
/// Call this before returning to the shell so the user's preferred cursor is
/// restored.
pub fn reset_cursor_shape() -> io::Result<()> {
    execute!(stdout(), SetCursorStyle::DefaultUserShape)
}

/// Emit DEC Mode 2026 `\x1b[?2026h` — ask the terminal to defer display
/// updates until [`end_synchronized_update`] is called.
///
/// Call once per frame, before `term.draw(…)`. Terminals that do not
/// recognise DEC 2026 silently ignore the sequence, so this is safe to emit
/// unconditionally. The `let _ = …` pattern at the call site is intentional:
/// a write failure here must never abort the render loop.
pub fn begin_synchronized_update() -> io::Result<()> {
    execute!(stdout(), BeginSynchronizedUpdate)
}

/// Emit DEC Mode 2026 `\x1b[?2026l` — signal the terminal that the current
/// frame is complete and it may paint the accumulated output atomically.
///
/// Call after every write that contributes to the current frame (draw,
/// cursor shape, cursor colour). Pairs with [`begin_synchronized_update`].
pub fn end_synchronized_update() -> io::Result<()> {
    execute!(stdout(), EndSynchronizedUpdate)
}

/// Leave the alt-screen and raw mode so subprocess output streams to the user's
/// terminal live. Call before spawning blocking processes (git clone, formatters)
/// that would otherwise be invisible inside the TUI.
///
/// Must be paired with [`leave_inline_output`] to restore the editor. Passes
/// the current kitty and mouse state so [`leave_inline_output`] can re-apply it.
///
/// Called from `EditorHostImpl::ensure_inline_output_screen`, not eagerly at
/// dispatch — the caller only reaches this on a command's first real output,
/// so a command whose body produces none never leaves the alt-screen at all.
pub fn enter_inline_output(kitty_enabled: bool, mouse_enabled: bool) -> io::Result<()> {
    // Close any open synchronized-output envelope (harmless if none is open).
    let _ = execute!(stdout(), EndSynchronizedUpdate);
    disable_bracketed_paste(&mut stdout())?;
    if kitty_enabled {
        pop_kitty_flags(&mut stdout())?;
    }
    if mouse_enabled {
        disable_mouse(&mut stdout())?;
    }
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Re-enter raw mode and the alt-screen after [`enter_inline_output`].
///
/// Call after all subprocess output has been written and the user has had a
/// chance to read it (typically after a "press any key" prompt). Restores
/// kitty and mouse to the state that was active before `enter_inline_output`.
pub fn leave_inline_output(
    kitty_enabled: bool,
    mouse_enabled: bool,
    mouse_select: bool,
) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    enable_bracketed_paste(&mut stdout())?;
    if kitty_enabled {
        push_kitty_flags(&mut stdout())?;
    }
    if mouse_enabled {
        enable_mouse(&mut stdout(), mouse_select)?;
    }
    Ok(())
}

/// Print the `--- running NAME ---` bold banner and flush stdout.
///
/// Call just after [`enter_inline_output`] so the user sees a clear separator
/// between the TUI and subprocess output.
pub fn print_running_banner(name: &str) {
    print!("\r\n\x1b[1m--- running {name} ---\x1b[0m\r\n\r\n");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Print the `--- press any key to return ---` dim prompt and flush stdout.
///
/// Call before [`wait_for_keypress`] so the user knows how to dismiss the
/// subprocess output and return to the TUI.
pub fn print_return_prompt() {
    print!("\r\n\x1b[2m--- press any key to return to editor ---\x1b[0m\r\n");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Block until the user presses a key, ignoring resize, mouse, and key-release
/// events. Holds subprocess output on screen until the user is ready to return
/// to the TUI.
pub fn wait_for_keypress() {
    let _ = enable_raw_mode();
    loop {
        match read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    let _ = disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::{
        disable_bracketed_paste, enable_bracketed_paste, pop_kitty_flags, push_kitty_flags,
    };

    // Regression guard for the Windows/WezTerm crash. The previous impl
    // dispatched through crossterm's `PushKeyboardEnhancementFlags` Command,
    // whose Windows arm hardcodes `is_ansi_code_supported()=false` and returns
    // `Unsupported`, crashing hume on startup. On Unix the same Command wrote
    // these exact bytes via `write_ansi`, so the test only catches the
    // regression on Windows CI — which is the matrix that was broken.
    #[test]
    fn push_kitty_flags_emits_raw_csi() {
        let mut buf = Vec::new();
        push_kitty_flags(&mut buf).unwrap();
        // DISAMBIGUATE_ESCAPE_CODES(1) | REPORT_EVENT_TYPES(2)
        // | REPORT_ALTERNATE_KEYS(4) = 7. Spec:
        // https://sw.kovidgoyal.net/kitty/keyboard-protocol/#progressive-enhancement
        assert_eq!(buf, b"\x1b[>7u");
    }

    #[test]
    fn pop_kitty_flags_emits_raw_csi() {
        let mut buf = Vec::new();
        pop_kitty_flags(&mut buf).unwrap();
        // kitty pop = CSI < 1 u (one stack level, fixed — no flag arg).
        assert_eq!(buf, b"\x1b[<1u");
    }

    #[test]
    fn enable_bracketed_paste_emits_raw_csi() {
        let mut buf = Vec::new();
        enable_bracketed_paste(&mut buf).unwrap();
        assert_eq!(buf, b"\x1b[?2004h");
    }

    #[test]
    fn disable_bracketed_paste_emits_raw_csi() {
        let mut buf = Vec::new();
        disable_bracketed_paste(&mut buf).unwrap();
        assert_eq!(buf, b"\x1b[?2004l");
    }
}
