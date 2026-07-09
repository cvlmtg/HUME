use std::borrow::Cow;

use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use hume_platform::path::shorten_home;

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
/// `~`-collapsed for display. Returns `""` for scratch and synthetic buffers.
pub(in crate::ui::statusline) fn statusline_display_path(editor: &Editor) -> String {
    let doc = editor.doc();
    let path = doc.display_path().or_else(|| doc.path());
    match path {
        Some(p) => shorten_home(p),
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
    if UnicodeWidthStr::width(display) <= max_cols {
        return display.to_owned();
    }
    if max_cols == 0 {
        return String::new();
    }

    // Split into components on '/'. Works on the `~`-collapsed string where
    // the separator is always '/'.
    let parts: Vec<&str> = display.split('/').collect();
    let n = parts.len();
    if n == 0 {
        return String::new();
    }

    // Build a mutable copy of the components. The last part is the filename;
    // abbreviate the leading dirs (all but the last) one by one.
    let mut components: Vec<String> = parts.iter().map(|s| s.to_string()).collect();

    for i in 0..n.saturating_sub(1) {
        // Skip components that are already at minimum width: empty, "~", or
        // single-grapheme entries (abbreviated in a previous run).
        let grapheme_count = components[i].graphemes(true).count();
        if grapheme_count <= 1 {
            continue;
        }
        // Abbreviate to the first grapheme cluster.
        let first = components[i]
            .graphemes(true)
            .next()
            .unwrap_or("")
            .to_owned();
        components[i] = first;

        let candidate = components.join("/");
        if UnicodeWidthStr::width(candidate.as_str()) <= max_cols {
            return candidate;
        }
    }

    // All dirs abbreviated — still too wide. Truncate the filename with `…`.
    let ellipsis = "…"; // U+2026, 1 col wide
    let ellipsis_w = UnicodeWidthStr::width(ellipsis);
    let prefix = components[..n.saturating_sub(1)].join("/");
    let sep = if n > 1 { "/" } else { "" };
    // Available columns for the filename (after prefix + sep + ellipsis).
    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
    let sep_w = sep.len(); // '/' is always 1 col
    let available = max_cols.saturating_sub(prefix_w + sep_w + ellipsis_w);

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
            format!("{prefix}{sep}{ellipsis}")
        }
    } else {
        format!("{prefix}{sep}{truncated}{ellipsis}")
    }
}
