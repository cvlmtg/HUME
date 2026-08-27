//! Frame presentation: the terminal-facing half of [`hume_grid`].
//!
//! [`Screen`] owns the two grids a double-buffered frame needs — `front`,
//! what the terminal is showing, and `back`, what is being composed — and the
//! escape-sequence emitter that carries one to the other. Callers compose
//! into [`Screen::frame`] and then [`Screen::present`]; nothing else in HUME
//! writes cell content to the terminal.
//!
//! The emitter is deliberately a set of pure functions over a `&mut String`
//! with the terminal nowhere in sight, so the exact bytes for a frame can be
//! asserted in a unit test rather than inferred from behaviour.
//!
//! Not handled here, because they already bypass this path and should keep
//! doing so: the DEC 2026 synchronized-update bracket, cursor shape, and
//! cursor colour all write single escapes straight through
//! [`crate::terminal`] around a frame, not as part of one.

use std::io::{self, Write};

use hume_grid::{Grid, Modifiers, Position, ResolvedStyle, Rgb, RowRun, UnderlineStyle};
use termina::OneBased;
use termina::Terminal as _;
use termina::escape::csi::{Csi, Cursor, DecPrivateModeCode, Sgr, SgrAttributes, SgrModifiers};
use termina::style::{ColorSpec, RgbColor};

use crate::terminal::{SharedTerm, dec_reset, dec_set};

/// How many unchanged cells the emitter will re-print to avoid repositioning
/// the cursor.
///
/// Moving the cursor costs a ~6–9 byte CUP sequence and a sequence for the
/// terminal to parse; re-printing an unchanged cell costs its own bytes and,
/// because it shares its neighbours' style, usually no SGR change at all.
/// Four keeps the worst-case re-print at roughly the cost of the CUP it
/// replaces, and keeps a styled run contiguous so the emitter's running SGR
/// state survives across the gap.
pub const MAX_REPRINT_GAP: u16 = 4;

/// A double-buffered terminal screen.
pub struct Screen {
    term: SharedTerm,
    /// What the terminal is currently showing.
    front: Grid,
    /// What the next [`Screen::present`] will make it show.
    back: Grid,
    /// Emit every cell next time instead of diffing, because what the
    /// terminal shows is no longer known — after a resize, or after something
    /// else wrote over the screen (see [`Screen::invalidate`]).
    force_full: bool,
    /// Reused escape buffer. One frame is built here and written in a single
    /// `write_all`, so a partially-written frame can never be interleaved
    /// with another writer's output.
    out: String,
}

impl Screen {
    pub fn new(term: SharedTerm) -> io::Result<Screen> {
        let (width, height) = dimensions(&term)?;
        Ok(Screen {
            term,
            front: Grid::new(width, height),
            back: Grid::new(width, height),
            // Nothing is known about what the terminal shows at startup.
            force_full: true,
            out: String::new(),
        })
    }

    /// The terminal's current size, read fresh.
    pub fn size(&self) -> io::Result<(u16, u16)> {
        dimensions(&self.term)
    }

    /// The grid to compose this frame into, blanked and sized to
    /// `width` × `height`.
    ///
    /// The caller passes the size rather than this reading it, so one frame
    /// has exactly one size authority: the loop reads the terminal once, and
    /// viewport geometry and cell storage are derived from that same answer.
    /// A resize between that read and [`Screen::present`] costs one clipped
    /// frame — harmless, since the terminal clamps a too-large cursor move
    /// and the queued resize event wakes the loop straight into another
    /// frame at the new size.
    pub fn frame(&mut self, width: u16, height: u16) -> &mut Grid {
        if self.back.size() != (width, height) {
            self.back.resize(width, height);
            // The front grid tracks the terminal, whose content after a
            // resize is its own business — reflowed, truncated, or cleared.
            // Resize it too so the two stay diffable, and repaint in full
            // rather than trusting a guess about what survived.
            self.front.resize(width, height);
            self.force_full = true;
        }
        self.back.reset();
        &mut self.back
    }

    /// Force the next [`Screen::present`] to emit every cell.
    ///
    /// For when something outside this type has written to the screen — HUME
    /// leaves the alternate screen to run a subprocess inline — so the front
    /// grid no longer describes what is displayed.
    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    /// Write this frame to the terminal and make it the new front buffer.
    ///
    /// `cursor` is where the hardware cursor should be left visible; `None`
    /// leaves it hidden, which is what modes that paint their own cursor
    /// block want.
    pub fn present(&mut self, cursor: Option<Position>) -> io::Result<()> {
        self.out.clear();
        let prev = (!self.force_full).then_some(&self.front);
        frame(&mut self.out, &self.back, prev, cursor);

        let mut term = self.term.clone();
        term.write_all(self.out.as_bytes())?;
        term.flush()?;

        std::mem::swap(&mut self.front, &mut self.back);
        self.force_full = false;
        Ok(())
    }
}

fn dimensions(term: &SharedTerm) -> io::Result<(u16, u16)> {
    let size = term.get_dimensions()?;
    // column-name-safe: termina's own field name for a terminal width.
    Ok((size.cols, size.rows))
}

// ── Emitter ──────────────────────────────────────────────────────────────

/// Build the escape sequence for one frame.
///
/// `prev` is the frame currently on screen, or `None` to emit every cell.
fn frame(out: &mut String, next: &Grid, prev: Option<&Grid>, cursor: Option<Position>) {
    push(out, dec_reset(DecPrivateModeCode::ShowCursor));

    match prev {
        Some(prev) => emit_runs(out, next.diff_runs(prev, MAX_REPRINT_GAP)),
        // A full redraw is the same emitter over one run per row — no
        // erase-display first, since every cell is written anyway.
        None => {
            let (_, height) = next.size();
            emit_runs(
                out,
                (0..height).map(|y| RowRun {
                    y,
                    x: 0,
                    cells: next.row(y),
                }),
            );
        }
    }

    // Leave SGR clean: the next frame's emitter starts from the default
    // style, and anything else that writes between frames (an inline
    // subprocess banner) must not inherit a cell's colours.
    push(out, Csi::Sgr(Sgr::Reset));

    if let Some(pos) = cursor {
        push(out, Csi::Cursor(cursor_to(pos)));
        push(out, dec_set(DecPrivateModeCode::ShowCursor));
    }
}

/// Write each run, carrying cursor position and SGR state across runs so
/// neither is re-stated when it has not changed.
fn emit_runs<'a>(out: &mut String, runs: impl Iterator<Item = RowRun<'a>>) {
    // Both start known: the frame ends with `Sgr::Reset`, so the terminal
    // enters each frame in the default style, and the first cell written
    // always positions the cursor explicitly.
    let mut style = ResolvedStyle::default();
    let mut at: Option<Position> = None;

    for run in runs {
        let mut x = run.x;
        for cell in run.cells {
            // A continuation's column was covered when the terminal drew its
            // head, so it contributes neither bytes nor advance.
            if cell.is_continuation() {
                continue;
            }
            let here = Position::new(x, run.y);
            if at != Some(here) {
                push(out, Csi::Cursor(cursor_to(here)));
            }
            let want = cell.style();
            if want != style {
                push(out, Csi::Sgr(Sgr::Attributes(sgr_delta(&style, &want))));
                style = want;
            }
            out.push_str(cell.text());
            x += cell.advance();
            // Past the last column of a row this is off the grid, which is
            // deliberate: the terminal is then in its pending-wrap state, and
            // recording a position no later write can match forces a cursor
            // move that resolves it before anything is drawn.
            at = Some(Position::new(x, run.y));
        }
    }
}

/// The attribute changes needed to go from `from` to `to`, grouped into one
/// SGR update.
fn sgr_delta(from: &ResolvedStyle, to: &ResolvedStyle) -> SgrAttributes {
    let mut attrs = SgrAttributes::default();
    if from.fg != to.fg {
        attrs.foreground = Some(color_spec(to.fg));
    }
    if from.bg != to.bg {
        attrs.background = Some(color_spec(to.bg));
    }
    if from.underline_color != to.underline_color {
        attrs.underline_color = Some(color_spec(to.underline_color));
    }
    attrs.modifiers = modifier_delta(from, to);
    attrs
}

fn color_spec(color: Option<Rgb>) -> ColorSpec {
    match color {
        // The terminal's own default, as SGR 39/49/59 — not a colour we pick.
        None => ColorSpec::Reset,
        Some(Rgb(r, g, b)) => ColorSpec::TrueColor(RgbColor::new(r, g, b).into()),
    }
}

fn modifier_delta(from: &ResolvedStyle, to: &ResolvedStyle) -> SgrModifiers {
    let mut m = SgrModifiers::empty();

    // Intensity (bold/dim) and blink (slow/rapid) are each *one* tri-state in
    // SGR rather than two independent switches: SGR 22 clears bold and dim
    // together, SGR 25 clears both blink rates. So when either member of a
    // pair changes, clear the whole state and re-assert whatever the target
    // still wants — deriving the update from added/removed bits alone emits
    // a bare clear and silently drops the member that was meant to survive.
    let intensity = |s: &ResolvedStyle| s.modifiers & (Modifiers::BOLD | Modifiers::DIM);
    if intensity(from) != intensity(to) {
        m |= SgrModifiers::INTENSITY_NORMAL;
        if to.modifiers.contains(Modifiers::DIM) {
            m |= SgrModifiers::INTENSITY_DIM;
        }
        if to.modifiers.contains(Modifiers::BOLD) {
            m |= SgrModifiers::INTENSITY_BOLD;
        }
    }

    let blink = |s: &ResolvedStyle| s.modifiers & (Modifiers::SLOW_BLINK | Modifiers::RAPID_BLINK);
    if blink(from) != blink(to) {
        m |= SgrModifiers::BLINK_NONE;
        if to.modifiers.contains(Modifiers::SLOW_BLINK) {
            m |= SgrModifiers::BLINK_SLOW;
        }
        if to.modifiers.contains(Modifiers::RAPID_BLINK) {
            m |= SgrModifiers::BLINK_RAPID;
        }
    }

    for (bit, on, off) in [
        (
            Modifiers::ITALIC,
            SgrModifiers::ITALIC,
            SgrModifiers::NO_ITALIC,
        ),
        (
            Modifiers::REVERSED,
            SgrModifiers::REVERSE,
            SgrModifiers::NO_REVERSE,
        ),
        (
            Modifiers::HIDDEN,
            SgrModifiers::INVISIBLE,
            SgrModifiers::NO_INVISIBLE,
        ),
        (
            Modifiers::STRIKETHROUGH,
            SgrModifiers::STRIKE_THROUGH,
            SgrModifiers::NO_STRIKE_THROUGH,
        ),
    ] {
        let now = to.modifiers.contains(bit);
        if from.modifiers.contains(bit) != now {
            m |= if now { on } else { off };
        }
    }

    if from.underline != to.underline {
        m |= match to.underline {
            UnderlineStyle::None => SgrModifiers::UNDERLINE_NONE,
            UnderlineStyle::Solid => SgrModifiers::UNDERLINE_SINGLE,
            UnderlineStyle::Wavy => SgrModifiers::UNDERLINE_CURLY,
            UnderlineStyle::Dotted => SgrModifiers::UNDERLINE_DOTTED,
            UnderlineStyle::Dashed => SgrModifiers::UNDERLINE_DASHED,
        };
    }

    m
}

fn cursor_to(pos: Position) -> Cursor {
    // `OneBased::from_zero_based` panics at `u16::MAX`; clamping keeps this
    // total at no practical cost, no terminal being 65535 cells across.
    Cursor::Position {
        line: OneBased::from_zero_based(pos.y.min(u16::MAX - 1)),
        // column-name-safe: termina's own field name for a cursor column.
        col: OneBased::from_zero_based(pos.x.min(u16::MAX - 1)),
    }
}

/// Append one escape sequence. `write!` into a `String` cannot fail —
/// `fmt::Write for String` only ever returns `Ok` — so there is no error to
/// propagate, and asserting that says so more clearly than discarding it.
fn push(out: &mut String, escape: impl std::fmt::Display) {
    use std::fmt::Write as _;
    write!(out, "{escape}").expect("writing an escape to a String cannot fail");
}

#[cfg(test)]
mod tests;
