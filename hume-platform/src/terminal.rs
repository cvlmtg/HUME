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
//!
//! [`SharedTerm`] is a cheap-to-clone handle: every caller that needs to read
//! or write the terminal (the render loop, the signal handler, the inline-
//! output bracket) holds a clone. Event reads/polls never lock the shared
//! mutex — they go straight to the cloned [`EventReader`] — so a blocking
//! read on one thread can never stall a write on another.

use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ratatui_termina::TerminaBackend;
use termina::Terminal as _;
use termina::escape::csi::{
    Csi, Cursor, DecPrivateMode, DecPrivateModeCode, Keyboard, KittyKeyboardFlags, Mode,
};
use termina::escape::osc::{ColorOrQuery, DynamicColorNumber, Osc};
use termina::event::KeyEventKind;
use termina::style::{CursorStyle, RgbColor};
use termina::{Event, EventReader, PlatformHandle, PlatformTerminal, WindowSize};

/// A ratatui `Terminal` backed by a [`SharedTerm`].
///
/// Aliased here so every other module can name the type without repeating the
/// backend parameter.
pub type Term = ratatui::Terminal<TerminaBackend<SharedTerm>>;

// ── SharedTerm ────────────────────────────────────────────────────────────────

/// A cheap-to-clone handle to the process's terminal.
///
/// Wraps the platform terminal behind a mutex so it can be shared between the
/// render loop, the signal handler, and the inline-output bracket, plus a
/// cloned [`EventReader`] captured once at [`create`] time. The mutex guards
/// only short operations — writes and mode switches; blocking `poll`/`read`
/// calls go through the `EventReader` directly and never take the lock, so a
/// pending read can never stall a writer on another thread.
#[derive(Clone)]
pub struct SharedTerm {
    inner: Arc<Mutex<PlatformTerminal>>,
    reader: EventReader,
}

impl SharedTerm {
    fn lock(&self) -> MutexGuard<'_, PlatformTerminal> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A cloneable reader for blocking or non-blocking event reads.
    pub fn event_reader(&self) -> EventReader {
        self.reader.clone()
    }
}

impl io::Write for SharedTerm {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.lock().flush()
    }
}

impl termina::Terminal for SharedTerm {
    fn enter_raw_mode(&mut self) -> io::Result<()> {
        self.lock().enter_raw_mode()
    }

    fn enter_cooked_mode(&mut self) -> io::Result<()> {
        self.lock().enter_cooked_mode()
    }

    fn get_dimensions(&self) -> io::Result<WindowSize> {
        self.lock().get_dimensions()
    }

    fn event_reader(&self) -> EventReader {
        self.reader.clone()
    }

    fn poll<F: Fn(&Event) -> bool>(
        &self,
        filter: F,
        timeout: Option<Duration>,
    ) -> io::Result<bool> {
        self.reader.poll(timeout, filter)
    }

    fn read<F: Fn(&Event) -> bool>(&self, filter: F) -> io::Result<Event> {
        self.reader.read(filter)
    }

    fn set_panic_hook(&mut self, f: impl Fn(&mut PlatformHandle) + Send + Sync + 'static) {
        self.lock().set_panic_hook(f);
    }
}

/// Open the process terminal. Call once at startup; clone the result for
/// every caller that needs to read or write it.
///
/// On Windows, `PlatformTerminal::new` enables VT input/output mode
/// unconditionally (HUME runs no legacy-console fallback — see
/// `docs/ROADMAP.md`); a console that can't provide it (older than Windows
/// 10 1809, or a raw pipe such as mintty without winpty) fails here.
pub fn create() -> io::Result<SharedTerm> {
    #[cfg(windows)]
    let inner = PlatformTerminal::new().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("{e} (HUME requires Windows 10 1809+ and a VT-capable console)"),
        )
    })?;
    #[cfg(not(windows))]
    let inner = PlatformTerminal::new()?;

    let reader = inner.event_reader();
    Ok(SharedTerm {
        inner: Arc::new(Mutex::new(inner)),
        reader,
    })
}

// ── Shared escape-sequence helpers ───────────────────────────────────────────

fn dec_set(code: DecPrivateModeCode) -> Csi {
    Csi::Mode(Mode::SetDecPrivateMode(DecPrivateMode::Code(code)))
}

fn dec_reset(code: DecPrivateModeCode) -> Csi {
    Csi::Mode(Mode::ResetDecPrivateMode(DecPrivateMode::Code(code)))
}

fn kitty_flags() -> KittyKeyboardFlags {
    // REPORT_ALTERNATE_KEYS is required so that Ctrl+shifted-chars (e.g.
    // Ctrl+}) arrive with the correct keycode instead of the base key plus
    // SHIFT. See docs/learning/command-keymap-dispatch.md.
    //
    // Known limitation: WezTerm 20240203-110809-5046fc22 does not fully
    // support REPORT_ALTERNATE_KEYS — Ctrl+shifted-char one-shot extend may
    // not work on that version.
    KittyKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES
        | KittyKeyboardFlags::REPORT_EVENT_TYPES
        | KittyKeyboardFlags::REPORT_ALTERNATE_KEYS
}

fn write_kitty_push(out: &mut impl io::Write) -> io::Result<()> {
    write!(out, "{}", Csi::Keyboard(Keyboard::PushFlags(kitty_flags())))?;
    out.flush()
}

fn write_kitty_pop(out: &mut impl io::Write) -> io::Result<()> {
    // Pop one level of the keyboard enhancement stack. Fixed protocol
    // sequence (no flag argument); harmless no-op when the stack is empty.
    write!(out, "{}", Csi::Keyboard(Keyboard::PopFlags(1)))?;
    out.flush()
}

fn write_mouse_enable(out: &mut impl io::Write, select: bool) -> io::Result<()> {
    // Normal tracking (1000): button press/release and scroll wheel.
    // SGR extended coordinates (1006): removes the 223-column limit of the
    // legacy X10 encoding; required for wide terminals.
    write!(
        out,
        "{}{}",
        dec_set(DecPrivateModeCode::MouseTracking),
        dec_set(DecPrivateModeCode::SGRMouse)
    )?;
    if select {
        // Button-event tracking (1002) additionally reports drag motion.
        // Without it, drag events never reach the application, so the
        // terminal handles drag-select natively.
        write!(out, "{}", dec_set(DecPrivateModeCode::ButtonEventMouse))?;
    }
    out.flush()
}

fn write_mouse_disable(out: &mut impl io::Write) -> io::Result<()> {
    // The reset sequences are harmless no-ops if the corresponding mode was
    // never enabled.
    write!(
        out,
        "{}{}{}",
        dec_reset(DecPrivateModeCode::ButtonEventMouse),
        dec_reset(DecPrivateModeCode::MouseTracking),
        dec_reset(DecPrivateModeCode::SGRMouse)
    )?;
    out.flush()
}

fn write_paste_enable(out: &mut impl io::Write) -> io::Result<()> {
    write!(out, "{}", dec_set(DecPrivateModeCode::BracketedPaste))?;
    out.flush()
}

fn write_paste_disable(out: &mut impl io::Write) -> io::Result<()> {
    write!(out, "{}", dec_reset(DecPrivateModeCode::BracketedPaste))?;
    out.flush()
}

fn write_sync_reset(out: &mut impl io::Write) -> io::Result<()> {
    write!(out, "{}", dec_reset(DecPrivateModeCode::SynchronizedOutput))?;
    out.flush()
}

fn write_leave_alt_screen(out: &mut impl io::Write) -> io::Result<()> {
    write!(
        out,
        "{}",
        dec_reset(DecPrivateModeCode::ClearAndEnableAlternateScreen)
    )?;
    out.flush()
}

/// SSOT byte sequence that undoes every application-level mode [`init`]
/// turns on: closes any open synchronized-update envelope, disables
/// bracketed paste, pops the kitty keyboard stack, disables mouse tracking,
/// and leaves the alternate screen. Shared between [`restore`] and the panic
/// hook installed by [`init`] — the hook can only write bytes (no raw/cooked
/// mode switch), and termina restores the platform mode itself right after
/// the hook returns.
///
/// Each step is attempted independently, even if an earlier one fails — the
/// goal is to leave the shell as usable as possible. The first error
/// encountered is returned; later ones are silently discarded.
fn write_unwind_escapes(out: &mut impl io::Write) -> io::Result<()> {
    let mut first_err: Option<io::Error> = None;
    let mut record = |r: io::Result<()>| {
        if first_err.is_none() {
            first_err = r.err();
        }
    };

    record(write_sync_reset(out));
    record(write_paste_disable(out));
    record(write_kitty_pop(out));
    record(write_mouse_disable(out));
    record(write_leave_alt_screen(out));
    // Second pop. Since `init()` pushes onto the alt screen's stack, the
    // first pop (above) clears it. This extra pop handles terminals with a
    // global keyboard stack — a harmless no-op on per-screen-buffer
    // terminals (WezTerm, kitty).
    record(write_kitty_pop(out));

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
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
pub fn probe_kitty(term: &SharedTerm) -> io::Result<bool> {
    let mut term = term.clone();
    term.enter_raw_mode()?;
    let kitty_enabled = match crate::probe_kitty_support(&term) {
        Ok(v) => v,
        Err(e) => {
            // Disable raw mode so the hint prints with normal line discipline
            // (no staircase) before we report it.
            let _ = term.enter_cooked_mode();
            eprintln!("hume: kitty keyboard probe failed: {e}");
            return Ok(false);
        }
    };
    term.enter_cooked_mode()?;
    Ok(kitty_enabled)
}

/// Switch the terminal into raw mode + alternate screen and return a ratatui
/// `Term` ready to render.
///
/// `kitty_enabled` is the result of a prior [`probe_kitty`] call. When `true`,
/// the caller should filter `KeyEventKind::Release` events from the event
/// loop and may enable Ctrl-modified key bindings that require the enhanced
/// protocol.
///
/// Mouse tracking is enabled selectively:
/// - `mouse_enabled` enables normal tracking (button press/release + scroll)
///   plus SGR extended coordinates. With only these modes, drag events are
///   NOT sent to the application, so the terminal handles drag-select
///   natively.
/// - `mouse_select` additionally enables button-event tracking, which sends
///   drag events so the editor can create editor selections on drag.
///
/// Call [`restore`] before the process exits so the user's shell is left in
/// a usable state; a panic during the session also restores it via the panic
/// hook installed here. The hook is armed before any mode is entered, and a
/// failure partway through this function's own enable sequence is unwound
/// before the error is returned — `init` never returns `Err` while leaving
/// the alternate screen, mouse tracking, or bracketed paste set.
pub fn init(
    term: &SharedTerm,
    mouse_enabled: bool,
    mouse_select: bool,
    kitty_enabled: bool,
) -> io::Result<Term> {
    let mut term = term.clone();

    // Arm before entering any mode: a panic during the enable sequence below
    // still unwinds through this hook, and every escape it emits is a
    // documented no-op for a mode not yet entered.
    term.set_panic_hook(|handle| {
        let _ = write_unwind_escapes(handle);
    });

    let enter = (|| -> io::Result<()> {
        term.enter_raw_mode()?;

        // Enter alternate screen before pushing kitty flags. Some terminals
        // (WezTerm, kitty) maintain a per-screen keyboard stack; the push
        // must land on the alternate screen's stack so that key reads
        // (which consult the active screen) pick up the enhanced encoding.
        write!(
            term,
            "{}",
            dec_set(DecPrivateModeCode::ClearAndEnableAlternateScreen)
        )?;
        term.flush()?;
        write_paste_enable(&mut term)?;

        if kitty_enabled {
            write_kitty_push(&mut term)?;
        }

        if mouse_enabled {
            write_mouse_enable(&mut term, mouse_select)?;
        }

        Ok(())
    })();

    if let Err(e) = enter {
        // A partial enable must not leak: the caller propagates this error
        // and never reaches the happy-path `restore()` in `run()`.
        let _ = restore(&term);
        return Err(e);
    }

    ratatui::Terminal::new(TerminaBackend::new(term))
}

/// Undo everything [`init`] did: run [`write_unwind_escapes`] then leave raw
/// mode. Both are attempted even if the first fails — the goal is to leave
/// the shell as usable as possible. The first error encountered is returned;
/// a second is silently discarded.
pub fn restore(term: &SharedTerm) -> io::Result<()> {
    let mut term = term.clone();
    let mut first_err: Option<io::Error> = None;
    let mut record = |r: io::Result<()>| {
        if first_err.is_none() {
            first_err = r.err();
        }
    };

    record(write_unwind_escapes(&mut term));
    record(term.enter_cooked_mode());

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Emit an OSC 12 sequence to set the terminal cursor colour.
///
/// When `black` is `true`, sets the cursor to black — used when the cursor
/// sits on a light-background surface (e.g. the statusline in Command/Search
/// mode) where the default colour would be invisible.
///
/// When `black` is `false`, resets to the user's configured terminal default.
pub fn set_cursor_color(term: &SharedTerm, black: bool) -> io::Result<()> {
    let mut term = term.clone();
    if black {
        write!(
            term,
            "{}",
            Osc::ChangeDynamicColors(
                DynamicColorNumber::TextCursorColor,
                vec![ColorOrQuery::Color(RgbColor::new(0, 0, 0))],
            )
        )?;
    } else {
        write!(
            term,
            "{}",
            Osc::ResetDynamicColor(DynamicColorNumber::TextCursorColor)
        )?;
    }
    term.flush()
}

/// Emit a DECSCUSR escape for the cursor shape.
///
/// When `bar` is `true`, emits `SteadyBar` (used for Insert/Command/Search/Select).
/// When `bar` is `false`, emits `SteadyBlock` (used for Normal/Extend).
pub fn set_cursor_shape(term: &SharedTerm, bar: bool) -> io::Result<()> {
    let style = if bar {
        CursorStyle::SteadyBar
    } else {
        CursorStyle::SteadyBlock
    };
    let mut term = term.clone();
    write!(term, "{}", Csi::Cursor(Cursor::CursorStyle(style)))?;
    term.flush()
}

/// Restore whatever cursor shape the user's terminal is configured to
/// display. Call before returning to the shell so the user's preferred
/// cursor is restored.
pub fn reset_cursor_shape(term: &SharedTerm) -> io::Result<()> {
    let mut term = term.clone();
    write!(
        term,
        "{}",
        Csi::Cursor(Cursor::CursorStyle(CursorStyle::Default))
    )?;
    term.flush()
}

/// Ask the terminal to defer display updates until [`end_synchronized_update`]
/// is called.
///
/// Call once per frame, before `term.draw(…)`. Terminals that do not
/// recognise DEC 2026 silently ignore the sequence, so this is safe to emit
/// unconditionally. The `let _ = …` pattern at the call site is intentional:
/// a write failure here must never abort the render loop.
pub fn begin_synchronized_update(term: &SharedTerm) -> io::Result<()> {
    let mut term = term.clone();
    write!(term, "{}", dec_set(DecPrivateModeCode::SynchronizedOutput))?;
    term.flush()
}

/// Signal the terminal that the current frame is complete and it may paint
/// the accumulated output atomically.
///
/// Call after every write that contributes to the current frame (draw,
/// cursor shape, cursor colour). Pairs with [`begin_synchronized_update`].
pub fn end_synchronized_update(term: &SharedTerm) -> io::Result<()> {
    let mut term = term.clone();
    write!(
        term,
        "{}",
        dec_reset(DecPrivateModeCode::SynchronizedOutput)
    )?;
    term.flush()
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
pub fn enter_inline_output(
    term: &SharedTerm,
    kitty_enabled: bool,
    mouse_enabled: bool,
) -> io::Result<()> {
    let mut term = term.clone();
    // Close any open synchronized-output envelope (harmless if none is open).
    let _ = write!(
        term,
        "{}",
        dec_reset(DecPrivateModeCode::SynchronizedOutput)
    );
    let _ = term.flush();
    write_paste_disable(&mut term)?;
    if kitty_enabled {
        write_kitty_pop(&mut term)?;
    }
    if mouse_enabled {
        write_mouse_disable(&mut term)?;
    }
    term.enter_cooked_mode()?;
    write!(
        term,
        "{}",
        dec_reset(DecPrivateModeCode::ClearAndEnableAlternateScreen)
    )?;
    term.flush()
}

/// Re-enter raw mode and the alt-screen after [`enter_inline_output`].
///
/// Call after all subprocess output has been written and the user has had a
/// chance to read it (typically after a "press any key" prompt). Restores
/// kitty and mouse to the state that was active before `enter_inline_output`.
pub fn leave_inline_output(
    term: &SharedTerm,
    kitty_enabled: bool,
    mouse_enabled: bool,
    mouse_select: bool,
) -> io::Result<()> {
    let mut term = term.clone();
    term.enter_raw_mode()?;
    write!(
        term,
        "{}",
        dec_set(DecPrivateModeCode::ClearAndEnableAlternateScreen)
    )?;
    term.flush()?;
    write_paste_enable(&mut term)?;
    if kitty_enabled {
        write_kitty_push(&mut term)?;
    }
    if mouse_enabled {
        write_mouse_enable(&mut term, mouse_select)?;
    }
    Ok(())
}

/// Reapply the mouse-tracking mode to match `mouse_enabled`/`mouse_select`.
///
/// Always disables tracking first (a harmless no-op for modes not currently
/// set — see [`write_mouse_disable`]) then re-enables per the new flags, so
/// it's safe to call whenever the desired mode changes at runtime, not just
/// once at startup (unlike [`init`], which only applies the startup mode).
pub fn set_mouse_mode(term: &SharedTerm, mouse_enabled: bool, mouse_select: bool) -> io::Result<()> {
    let mut term = term.clone();
    write_mouse_disable(&mut term)?;
    if mouse_enabled {
        write_mouse_enable(&mut term, mouse_select)?;
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
pub fn wait_for_keypress(term: &SharedTerm) {
    let mut term = term.clone();
    let _ = term.enter_raw_mode();
    let reader = term.event_reader();
    loop {
        match reader.read(|_| true) {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    let _ = term.enter_cooked_mode();
}

#[cfg(test)]
mod tests;
