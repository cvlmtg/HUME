use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;

use hume_engine::render::{fill_rect_bg, write_text_run};

use crate::editor::Editor;
use crate::ui::theme::EditorColors;
use crate::ui::width::text_width;

mod elements;
use elements::{
    CwdElement, DiagnosticsElement, DirtyIndicatorElement, FileNameElement, KittyProtocolElement,
    LanguageElement, LineEndingElement, MacroRecordingElement, MiniBufElement, ModeElement,
    PositionElement, ReadOnlyElement, SearchMatchesElement, SelectionsElement, SeparatorElement,
    StatuslineElement,
};

/// Hardcoded left section for Command/Search modes.
const MINIBUF_LEFT: &[StatusElement] = &[StatusElement::MiniBuf];

// ── Configuration ─────────────────────────────────────────────────────────────

/// A named element that can appear in a statusline section.
///
/// Elements are the building blocks of the statusline. The mode indicator,
/// separators, and data fields are all first-class element variants —
/// there is no special chrome. You control the layout by choosing which
/// elements appear in each section and in what order.
///
/// The Steel scripting layer constructs [`StatusLineConfig`] values at
/// runtime; this enum is the wire format for those configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusElement {
    /// The mode indicator: `"NOR"`, `"INS"`, `"EXT"`, `"CMD"`, `"SRC"`, or `"SEL"`.
    ///
    /// Rendered in the row style, like every other element — the whole row
    /// tints with the mode, not just this label. Contains no padding — the
    /// renderer's edge padding and inter-element spacing handle surrounding
    /// whitespace.
    Mode,
    /// A thin vertical bar `│`, styled from `ui.statusline.separator` (falls
    /// back to the row style when a theme omits that scope).
    ///
    /// Place this explicitly between elements that need a visual divider.
    Separator,
    /// The file's basename, or `"*scratch*"` for unnamed buffers.
    FileName,
    /// Full path to the focused file, with the home prefix collapsed to `~`.
    ///
    /// Shows the path as the user typed it (symlinks **not** resolved). When the
    /// terminal row is too narrow the path is progressively shortened: leading
    /// directory components are abbreviated to their first grapheme cluster
    /// left-to-right; the filename is truncated with `…` only as a last resort.
    ///
    /// Intended for the `left` section where it has the most available space.
    /// For scratch and synthetic buffers (no path) this falls back to the
    /// buffer's display name — its label, or `*scratch*`.
    FilePath,
    /// Current working directory, with the home prefix replaced by `~`.
    ///
    /// Read from `editor.state.cwd`, which is cached at startup and updated by `:cd`.
    /// Renders empty when the working directory path is unavailable.
    Cwd,
    /// Cursor position as `"line:col"` (both 1-based, col = grapheme index).
    Position,
    /// Selection count as `"N sels"`, or the empty string when only one
    /// selection is active (so it occupies no space in single-cursor mode).
    Selections,
    /// Kitty keyboard protocol indicator: `"ᓚᘏᗢ"` when active, empty otherwise.
    ///
    /// Useful for diagnosing whether the protocol was successfully negotiated.
    KittyProtocol,
    /// Dirty indicator: `"[+]"` when the buffer has unsaved changes, empty otherwise.
    DirtyIndicator,
    /// Line-ending indicator: `"LF"` or `"CRLF"` reflecting the buffer's write-back
    /// encoding. Always shown — LF is the common case but worth making explicit.
    LineEnding,
    /// Search match count: `"[3/42]"` when a search regex is active, empty otherwise.
    ///
    /// The current index is 1-based — the match whose range contains the primary
    /// cursor head. Shows `0` when the cursor is between matches (e.g. the live
    /// search has no hit yet).
    SearchMatches,
    /// The mini-buffer input field: prompt character followed by typed text.
    ///
    /// Rendered only when `editor.state.minibuf` is `Some`; empty otherwise.
    MiniBuf,
    /// Macro recording indicator: `"[recording @q]"` while a macro is being
    /// recorded, empty otherwise.
    MacroRecording,
    /// The focused buffer's language identifier, rendered as `"[rust]"`, or
    /// empty when no language is detected (scratch buffers, unknown filetypes).
    Language,
    /// Read-only indicator: `"[RO]"` when the buffer is read-only, empty otherwise.
    ReadOnly,
    /// Diagnostic counts for the focused buffer: `"✘ 3 ⚠ 12"`, empty when
    /// both counts are zero. Reads the diagnostics store directly in
    /// Rust — the statusline renders every frame, so this never goes
    /// through Steel's `(diagnostic-counts …)` builtin (that one is for
    /// plugins, not the render path).
    Diagnostics,
}

impl fmt::Display for StatusElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StatusElement::Mode => "Mode",
            StatusElement::Separator => "Separator",
            StatusElement::FileName => "FileName",
            StatusElement::FilePath => "FilePath",
            StatusElement::Cwd => "Cwd",
            StatusElement::Position => "Position",
            StatusElement::Selections => "Selections",
            StatusElement::KittyProtocol => "KittyProtocol",
            StatusElement::DirtyIndicator => "DirtyIndicator",
            StatusElement::LineEnding => "LineEnding",
            StatusElement::SearchMatches => "SearchMatches",
            StatusElement::MiniBuf => "MiniBuf",
            StatusElement::MacroRecording => "MacroRecording",
            StatusElement::Language => "Language",
            StatusElement::ReadOnly => "ReadOnly",
            StatusElement::Diagnostics => "Diagnostics",
        })
    }
}

impl FromStr for StatusElement {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Mode" => Ok(StatusElement::Mode),
            "Separator" => Ok(StatusElement::Separator),
            "FileName" => Ok(StatusElement::FileName),
            "FilePath" => Ok(StatusElement::FilePath),
            "Cwd" => Ok(StatusElement::Cwd),
            "Position" => Ok(StatusElement::Position),
            "Selections" => Ok(StatusElement::Selections),
            "KittyProtocol" => Ok(StatusElement::KittyProtocol),
            "DirtyIndicator" => Ok(StatusElement::DirtyIndicator),
            "LineEnding" => Ok(StatusElement::LineEnding),
            "SearchMatches" => Ok(StatusElement::SearchMatches),
            "MiniBuf" => Ok(StatusElement::MiniBuf),
            "MacroRecording" => Ok(StatusElement::MacroRecording),
            "Language" => Ok(StatusElement::Language),
            "ReadOnly" => Ok(StatusElement::ReadOnly),
            "Diagnostics" => Ok(StatusElement::Diagnostics),
            _ => Err(format!(
                "unknown element '{s}'; valid names: Cwd Diagnostics DirtyIndicator FilePath \
                 FileName KittyProtocol Language LineEnding MacroRecording MiniBuf Mode Position \
                 ReadOnly SearchMatches Selections Separator"
            )),
        }
    }
}

/// Describes the content layout of the statusline's three sections.
///
/// Each section is a sequence of [`StatusElement`]s rendered in order. Adjacent
/// elements within a section are joined with a boundary-aware spacing rule so
/// that spacing feels natural without any element needing to hard-code its
/// neighbours.
///
/// The default config reproduces the built-in statusline layout exactly, so
/// the editor looks identical with no configuration:
///
/// ```text
/// 42:7 notes.txt              │ NOR
/// ```
#[derive(Debug, Clone)]
pub struct StatusLineConfig {
    /// Elements rendered left-aligned at the start of the statusline row.
    pub left: Vec<StatusElement>,
    /// Elements centered between the left and right sections. Empty by default.
    pub center: Vec<StatusElement>,
    /// Elements rendered right-aligned at the end of the statusline row.
    pub right: Vec<StatusElement>,
}

/// Parse one `configure-statusline!` section (`left`/`center`/`right`) from
/// its wire-format element names, labeling a parse failure with which
/// section it came from. Shared by the production host and the test mock —
/// `mock_host.rs` is `#[path]`-included into external integration-test
/// crates, so this must be `pub` like `StatusElement`/`StatusLineConfig`
/// themselves.
pub fn parse_statusline_section(
    list: Vec<String>,
    section: &str,
) -> Result<Vec<StatusElement>, String> {
    list.iter()
        .map(|s| {
            s.parse::<StatusElement>()
                .map_err(|e| format!("configure-statusline! {section}: {e}"))
        })
        .collect()
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        Self {
            left: vec![
                StatusElement::Position,
                StatusElement::FilePath,
                StatusElement::Language,
                StatusElement::ReadOnly,
                StatusElement::DirtyIndicator,
            ],
            center: vec![],
            right: vec![
                StatusElement::MacroRecording,
                StatusElement::SearchMatches,
                StatusElement::Diagnostics,
                StatusElement::KittyProtocol,
                StatusElement::Separator,
                StatusElement::Mode,
            ],
        }
    }
}

// ── Edge padding ─────────────────────────────────────────────────────────────

/// Prepend a 1-space padding span to the left section.
///
/// This gives every left section a consistent left margin without requiring
/// individual elements to be edge-aware.
fn pad_left(
    mut spans: Vec<(Cow<'static, str>, Style)>,
    colors: &EditorColors,
) -> Vec<(Cow<'static, str>, Style)> {
    if !spans.is_empty() {
        spans.insert(0, (Cow::Borrowed(" "), colors.statusline));
    }
    spans
}

/// Append a 1-space padding span to the right section.
///
/// Mirrors [`pad_left`] for the trailing edge; supplies the right-margin
/// offset used by the placement arithmetic.
fn pad_right(
    mut spans: Vec<(Cow<'static, str>, Style)>,
    colors: &EditorColors,
) -> Vec<(Cow<'static, str>, Style)> {
    if !spans.is_empty() {
        spans.push((Cow::Borrowed(" "), colors.statusline));
    }
    spans
}

// ── Drawing helpers ───────────────────────────────────────────────────────────

/// Total display-column width of a rendered section.
fn section_width(spans: &[(Cow<'static, str>, Style)]) -> u16 {
    spans
        .iter()
        .map(|(t, _)| text_width(t.as_ref()) as u16)
        .sum()
}

/// Draw a section's spans left-to-right starting at `x`.
fn draw_section(
    screen_buf: &mut ScreenBuf,
    spans: &[(Cow<'static, str>, Style)],
    mut x: u16,
    y: u16,
    right_edge: u16,
) {
    for (text, style) in spans {
        // Advance by what the write consumed rather than re-measuring the
        // span: `write_text_run` returns the column it stopped at, and it
        // measures by the same rule `section_width` laid the spans out with,
        // so the two cannot drift apart.
        x = write_text_run(screen_buf, x, y, text.as_ref(), *style, right_edge);
    }
}

// ── Engine integration ────────────────────────────────────────────────────────

/// Short-lived statusline provider that borrows `&Editor` directly.
///
/// Created each frame in `Editor::run()` and passed to `EngineView::render()`.
/// No snapshot, no Arc, no Mutex — the provider reads editor state on demand
/// during the render call.
pub(crate) struct HumeStatusline<'a> {
    pub(crate) editor: &'a Editor,
}

impl hume_engine::providers::StatuslineProvider for HumeStatusline<'_> {
    fn render(
        &self,
        area: ratatui::layout::Rect,
        theme: &hume_engine::theme::Theme,
        buf: &mut ratatui::buffer::Buffer,
    ) {
        let editor = self.editor;
        let mode = editor
            .state
            .settings
            .statusline_mode_colors
            .then(|| editor.state.mode());
        let colors = EditorColors::from_theme(theme, mode);
        let y = area.y;

        // An open confirm overlay (disk-change reload, …) owns the whole
        // row unconditionally — it's the intercept chain's top entry (see
        // `handle_key`), so it must also be the top-priority render, ahead
        // of even the minibuffer.
        if let Some(confirm) = editor.state.config.confirm.as_ref() {
            fill_row_colors(buf, &colors, area, y);
            write_text_run(
                buf,
                area.x + 1,
                y,
                &confirm.render_line(),
                colors.statusline,
                area.x + area.width,
            );
            return;
        }

        if editor.state.minibuf.is_none() {
            // A fresh status_msg (set this frame) takes priority. Falling back
            // to the log summary keeps unseen-message context visible between
            // keypresses, without adding a separate status row.
            //
            // Evaluate summary_text lazily to avoid allocating a String every
            // frame when there is nothing to show.
            let summary;
            let display_msg: Option<&str> = if let Some(ref msg) = editor.state.status_msg {
                Some(msg.as_str())
            } else {
                summary = editor.state.message_log.summary_text();
                summary.as_deref()
            };

            if let Some(msg) = display_msg {
                fill_row_colors(buf, &colors, area, y);
                write_text_run(
                    buf,
                    area.x + 1,
                    y,
                    msg,
                    colors.statusline,
                    area.x + area.width,
                );
                return;
            }
        }

        render_statusline(buf, editor, &colors, area, y);
    }
}

fn fill_row_colors(buf: &mut ScreenBuf, colors: &EditorColors, area: Rect, y: u16) {
    fill_rect_bg(buf, Rect::new(area.x, y, area.width, 1), colors.statusline);
}

fn render_statusline(
    screen_buf: &mut ScreenBuf,
    editor: &Editor,
    colors: &EditorColors,
    area: Rect,
    y: u16,
) {
    let config = &editor.state.settings.statusline;

    let (left_elems, center_elems, right_elems): (
        &[StatusElement],
        &[StatusElement],
        &[StatusElement],
    ) = if editor.state.minibuf.is_some() {
        (MINIBUF_LEFT, &[], &config.right)
    } else {
        (&config.left, &config.center, &config.right)
    };

    fill_row_colors(screen_buf, colors, area, y);

    // ── FilePath two-pass sizing ──────────────────────────────────────────────
    // The FilePath element is flexible: it shrinks when the row is narrow.
    // Measure pass: render with FilePath = "" to find the total fixed width.
    // Final pass: shorten the path to the remaining budget, then render for real.
    let filepath_full = elements::file_path::statusline_display_path(editor);
    let filepath_display: String = if filepath_full.is_empty() {
        String::new()
    } else {
        // Measure pass — FilePath contributes no width.
        let m_left = pad_left(render_section(left_elems, editor, colors, ""), colors);
        let m_center = render_section(center_elems, editor, colors, "");
        let m_right = pad_right(render_section(right_elems, editor, colors, ""), colors);
        let fixed_w = section_width(&m_left) as usize
            + section_width(&m_center) as usize
            + section_width(&m_right) as usize;
        // Subtract 1 for the inter-element space that render_section inserts
        // before a non-empty FilePath span (conservatively safe for all cases).
        let budget = (area.width as usize).saturating_sub(fixed_w + 1);
        elements::file_path::shorten_path_to_width(&filepath_full, budget)
    };

    let left_spans = pad_left(
        render_section(left_elems, editor, colors, &filepath_display),
        colors,
    );
    let center_spans = render_section(center_elems, editor, colors, &filepath_display);
    let right_spans = pad_right(
        render_section(right_elems, editor, colors, &filepath_display),
        colors,
    );

    let left_w = section_width(&left_spans);
    let center_w = section_width(&center_spans);
    let right_w = section_width(&right_spans);

    let left_x = area.x;
    let left_end = left_x + left_w;
    let right_x = area.right().saturating_sub(right_w);
    let right_fits = right_x >= left_end;
    let right_fence = if right_fits { right_x } else { area.right() };
    let gap = right_fence.saturating_sub(left_end);
    let center_x = (left_end + gap / 2).saturating_sub(center_w / 2);
    let center_fits = !center_spans.is_empty()
        && center_w <= gap
        && center_x >= left_end
        && center_x + center_w <= right_fence;

    // Each section is bounded by whatever sits to its right: the left
    // section stops where the right one starts (or at the row's end when the
    // right section didn't fit), and the right section at the row's end. Only
    // `FilePath` shortens itself to fit, so without these an over-long
    // element would write across its neighbour and off the row.
    draw_section(screen_buf, &left_spans, left_x, y, right_fence);
    if right_fits {
        draw_section(screen_buf, &right_spans, right_x, y, area.right());
    }
    if center_fits {
        draw_section(screen_buf, &center_spans, center_x, y, right_fence);
    }
}

pub(crate) fn render_element(
    seg: StatusElement,
    editor: &Editor,
    colors: &EditorColors,
    // Pre-computed text for the FilePath element. "" = render as empty (measure
    // pass); any other string = use verbatim (already shortened by caller).
    filepath_text: &str,
) -> (Cow<'static, str>, Style) {
    match seg {
        StatusElement::Mode => ModeElement::render(editor, colors),
        StatusElement::Separator => SeparatorElement::render(editor, colors),
        StatusElement::FileName => FileNameElement::render(editor, colors),
        StatusElement::FilePath => elements::file_path::render(filepath_text, colors),
        StatusElement::Position => PositionElement::render(editor, colors),
        StatusElement::KittyProtocol => KittyProtocolElement::render(editor, colors),
        StatusElement::Selections => SelectionsElement::render(editor, colors),
        StatusElement::DirtyIndicator => DirtyIndicatorElement::render(editor, colors),
        StatusElement::LineEnding => LineEndingElement::render(editor, colors),
        StatusElement::Cwd => CwdElement::render(editor, colors),
        StatusElement::SearchMatches => SearchMatchesElement::render(editor, colors),
        StatusElement::MiniBuf => MiniBufElement::render(editor, colors),
        StatusElement::MacroRecording => MacroRecordingElement::render(editor, colors),
        StatusElement::Language => LanguageElement::render(editor, colors),
        StatusElement::ReadOnly => ReadOnlyElement::render(editor, colors),
        StatusElement::Diagnostics => DiagnosticsElement::render(editor, colors),
    }
}

fn render_section(
    elements: &[StatusElement],
    editor: &Editor,
    colors: &EditorColors,
    filepath_text: &str,
) -> Vec<(Cow<'static, str>, Style)> {
    let mut spans: Vec<(Cow<'static, str>, Style)> = Vec::with_capacity(elements.len() * 2);

    for &seg in elements {
        let (text, style) = render_element(seg, editor, colors, filepath_text);
        if text.is_empty() {
            continue;
        }

        if let Some((prev_text, _)) = spans.last() {
            let a_ends_space = prev_text.ends_with(' ');
            let b_starts_space = text.starts_with(' ');

            if !a_ends_space && !b_starts_space {
                spans.push((Cow::Borrowed(" "), colors.statusline));
                spans.push((text, style));
            } else if a_ends_space && b_starts_space {
                let trimmed = match text {
                    Cow::Borrowed(s) => Cow::Borrowed(s.strip_prefix(' ').unwrap_or(s)),
                    Cow::Owned(mut s) => {
                        s.drain(..1);
                        Cow::Owned(s)
                    }
                };
                spans.push((trimmed, style));
            } else {
                spans.push((text, style));
            }
        } else {
            spans.push((text, style));
        }
    }

    spans
}

#[cfg(test)]
mod tests;
