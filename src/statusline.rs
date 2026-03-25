use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::Buffer;
use crate::editor::Mode;
use crate::renderer::RenderCtx;

// ── Public entry point ────────────────────────────────────────────────────────

/// Render the bottom row of the terminal: command line, status message, or
/// status bar — whichever has the highest priority.
///
/// Priority: command mini-buffer > transient status message > normal status bar.
pub(crate) fn render_bottom_row(
    screen_buf: &mut ScreenBuf,
    ctx: &RenderCtx<'_>,
    area: Rect,
    y: u16,
    cursor_line: usize,
    cursor_head: usize,
    buf: &Buffer,
) {
    if let Some((prompt, input)) = ctx.minibuf {
        render_command_line(screen_buf, ctx, area, y, prompt, input);
    } else if let Some(msg) = ctx.status_msg {
        render_status_message(screen_buf, ctx, area, y, msg);
    } else {
        render_status_bar(screen_buf, ctx, cursor_line, cursor_head, buf, area, y);
    }
}

// ── Renderers ─────────────────────────────────────────────────────────────────

/// Render the command-line mini-buffer on the bottom row.
///
/// Fully replaces the status bar — no mode pill, no segments. The prompt
/// character (e.g. `:`) makes the mode self-evident. The terminal cursor
/// is positioned after the input by the caller.
fn render_command_line(
    screen_buf: &mut ScreenBuf,
    ctx: &RenderCtx<'_>,
    area: Rect,
    y: u16,
    prompt: char,
    input: &str,
) {
    let colors = ctx.colors;

    // The command line fully replaces the status bar row — no segment layout,
    // no mode pill. The prompt character makes the mode self-evident.
    let blank: String = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, colors.status_bar);

    let cmd_str = format!("{prompt}{input}");
    screen_buf.set_string(area.x + 1, y, &cmd_str, colors.status_bar);
}

/// Render a transient status message on the bottom row.
///
/// Uses the inverted status bar style so the message stands out. The message
/// is cleared on the next keypress.
fn render_status_message(
    screen_buf: &mut ScreenBuf,
    ctx: &RenderCtx<'_>,
    area: Rect,
    y: u16,
    msg: &str,
) {
    let blank: String = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, ctx.colors.status_bar);
    screen_buf.set_string(area.x + 1, y, msg, ctx.colors.status_bar);
}

/// Render the one-row status bar at the bottom of the area.
///
/// Layout (all with inverted style):
/// - Left  : ` NOR ` (mode label padded with spaces, mode color) + `│` + one space + filename
/// - Right : `line:col` (both 1-based) + one space
///
/// `INS` is rendered in cyan, `EXT` in yellow, to make mode transitions visually obvious.
fn render_status_bar(
    screen_buf: &mut ScreenBuf,
    ctx: &RenderCtx<'_>,
    cursor_line: usize,
    cursor_head: usize,
    buf: &Buffer,
    area: Rect,
    y: u16,
) {
    let colors = ctx.colors;

    // Fill the entire row with inverted spaces first.
    let blank: String = " ".repeat(area.width as usize);
    screen_buf.set_string(area.x, y, &blank, colors.status_bar);

    // Mode label — " NOR " (5 chars: leading space + 3-char label + trailing space),
    // rendered with the mode color flush against the left edge (column 0).
    let (mode_label, mode_style) = match (ctx.mode, ctx.extend) {
        (Mode::Normal, true)  => (" EXT ", colors.status_extend),
        (Mode::Normal, false) => (" NOR ", colors.status_normal),
        (Mode::Insert, _)     => (" INS ", colors.status_insert),
        // Command mode is handled by render_command_line before this is reached.
        (Mode::Command, _)    => (" NOR ", colors.status_normal),
    };
    screen_buf.set_string(area.x, y, mode_label, mode_style);

    // Separator "│" immediately after the mode pill (column 5), then one space
    // of padding from the background fill, then filename at column 7.
    screen_buf.set_string(area.x + 5, y, "│", colors.status_bar);

    let filename = ctx.file_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("[scratch]");
    screen_buf.set_string(area.x + 7, y, filename, colors.status_bar);

    // Right: "line:col" (1-based column = grapheme count from line start + 1).
    let col_0 = grapheme_col_in_line(buf, cursor_line, cursor_head);
    let pos_str = format!("{}:{}", cursor_line + 1, col_0 + 1);
    // Place with 1 space of right margin.
    let pos_x = area.right().saturating_sub(pos_str.len() as u16 + 1);
    screen_buf.set_string(pos_x, y, &pos_str, colors.status_bar);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Count grapheme clusters from the start of `line_idx` to `char_pos`.
///
/// Returns the 0-based grapheme offset of the cursor within its line — the
/// same unit used by left/right cursor movement. This is intentionally a
/// logical position (grapheme index), not a display column: if the line
/// contains wide characters, the visual column may differ, but the reported
/// number matches how many times the user pressed → to get there.
pub(crate) fn grapheme_col_in_line(buf: &Buffer, line_idx: usize, char_pos: usize) -> usize {
    let line_start = buf.line_to_char(line_idx);
    // char_pos should be >= line_start, but saturating_sub guards against
    // any edge cases in empty buffers.
    let slice = buf.slice(line_start..char_pos.max(line_start));
    slice.to_string().graphemes(true).count()
}
