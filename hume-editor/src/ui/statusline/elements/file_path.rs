use std::borrow::Cow;

use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use hume_platform::path::{is_path_sep, shorten_home, strip_unc_prefix};

use crate::editor::Editor;
use crate::ui::theme::EditorColors;

/// The `FilePath` element does not implement [`super::StatuslineElement`]:
/// its content isn't read from `Editor` directly, but computed externally by
/// `render_statusline`'s two-pass sizing pass (measure with `""`, then
/// [`shorten_path_to_width`] against the remaining budget) and handed in as
/// `filepath_text`.
pub(in crate::ui::statusline) fn render(
    filepath_text: &str,
    colors: &EditorColors,
) -> (Cow<'static, str>, Style) {
    (Cow::Owned(filepath_text.to_owned()), colors.statusline)
}

/// Returns the display path for the `FilePath` element: `display_path` (user-typed,
/// symlinks unresolved) when set, falling back to the canonical `path`. Both are
/// stripped of the Windows `\\?\` verbatim prefix and `~`-collapsed for display.
/// Falls back to the buffer's display name (label, or `*scratch*`) when there is
/// no path at all — scratch and synthetic buffers.
pub(in crate::ui::statusline) fn statusline_display_path(editor: &Editor) -> String {
    let doc = editor.doc();
    match doc.display_path().or_else(|| doc.path()) {
        Some(p) => display_path_string(Some(p)),
        None => doc.display_name(),
    }
}

/// Normalize a buffer path for display: strip the Windows `\\?\` verbatim
/// prefix (no-op on other platforms and on paths that never carried one, e.g.
/// `display_path`'s `absolute_unresolved` output), then `~`-collapse.
/// `strip_unc_prefix` must run first — `shorten_home`'s prefix match is
/// against the clean `C:\Users\...` form, which the verbatim-prefixed string
/// wouldn't match. `None` maps to `""`.
pub(in crate::ui::statusline) fn display_path_string(path: Option<&std::path::Path>) -> String {
    match path {
        Some(p) => shorten_home(&strip_unc_prefix(p.to_owned())),
        None => String::new(),
    }
}

/// Shorten `display` (a `~`-collapsed path string) to fit within `max_cols`
/// terminal columns by abbreviating leading directory components.
///
/// Algorithm:
/// 1. If it already fits, return as-is.
/// 2. Replace leading dir components with their first grapheme cluster,
///    left-to-right, re-checking width each time.
/// 3. If still too wide after all dirs are abbreviated, truncate the filename
///    with a trailing `…`, shrinking until it fits or only `…` remains.
pub(in crate::ui::statusline) fn shorten_path_to_width(display: &str, max_cols: usize) -> String {
    shorten_path_to_width_with(display, max_cols, is_path_sep)
}

/// `is_sep` is injected so the separator handling (`/` everywhere, plus `\` on
/// Windows) is exercisable in tests regardless of the host platform. Splits
/// keep each component's trailing separator character so mixed `/`/`\` input
/// (as can appear in a Windows path) survives reassembly unchanged.
pub(in crate::ui::statusline) fn shorten_path_to_width_with(
    display: &str,
    max_cols: usize,
    is_sep: fn(char) -> bool,
) -> String {
    if UnicodeWidthStr::width(display) <= max_cols {
        return display.to_owned();
    }
    if max_cols == 0 {
        return String::new();
    }

    let parts: Vec<&str> = display.split_inclusive(is_sep).collect();
    let n = parts.len();
    if n == 0 {
        return String::new();
    }

    // Build a mutable copy of the components. The last part is the filename;
    // abbreviate the leading dirs (all but the last) one by one.
    let mut components: Vec<String> = parts.iter().map(|s| s.to_string()).collect();

    for i in 0..n.saturating_sub(1) {
        let (content, sep) = split_trailing_sep(&components[i], is_sep);
        // Skip components that are already at minimum width: empty, "~", or
        // single-grapheme entries (abbreviated in a previous run).
        let grapheme_count = content.graphemes(true).count();
        if grapheme_count <= 1 {
            continue;
        }
        // Abbreviate to the first grapheme cluster, keeping the separator.
        let first = content.graphemes(true).next().unwrap_or("").to_owned();
        components[i] = format!("{first}{sep}");

        let candidate = components.concat();
        if UnicodeWidthStr::width(candidate.as_str()) <= max_cols {
            return candidate;
        }
    }

    // All dirs abbreviated — still too wide. Truncate the filename with `…`.
    let ellipsis = "…"; // U+2026, 1 col wide
    let ellipsis_w = UnicodeWidthStr::width(ellipsis);
    let prefix = components[..n.saturating_sub(1)].concat();
    // Available columns for the filename (after prefix + ellipsis). The
    // prefix already carries its own trailing separator, if any.
    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
    let available = max_cols.saturating_sub(prefix_w + ellipsis_w);

    let filename = &components[n - 1];
    let mut truncated = String::new();
    let mut cols_used = 0usize;
    for g in filename.graphemes(true) {
        let gw = UnicodeWidthStr::width(g);
        if cols_used + gw > available {
            break;
        }
        truncated.push_str(g);
        cols_used += gw;
    }

    if truncated.is_empty() {
        // Not even one grapheme of filename fits; just show the ellipsis.
        if prefix.is_empty() {
            ellipsis.to_owned()
        } else {
            format!("{prefix}{ellipsis}")
        }
    } else {
        format!("{prefix}{truncated}{ellipsis}")
    }
}

/// Split `s` into `(content, sep)` where `sep` is the trailing separator
/// character (as injected by `is_sep`), or `(s, "")` if `s` has none.
fn split_trailing_sep(s: &str, is_sep: fn(char) -> bool) -> (&str, &str) {
    match s.chars().next_back() {
        Some(c) if is_sep(c) => {
            let split_at = s.len() - c.len_utf8();
            (&s[..split_at], &s[split_at..])
        }
        _ => (s, ""),
    }
}
