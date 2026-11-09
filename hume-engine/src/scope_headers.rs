//! Scope headers: the first lines of the scopes a pane's top rows are
//! inside, painted over those rows.
//!
//! [`scope_headers`] is the one walk that decides them. Rendering draws its
//! answer, [`ViewGeometry::top_margin_at`](crate::pane::ViewGeometry::top_margin_at)
//! keeps the cursor below it, and a mouse hit on a header row resolves
//! through it.

use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;
use ropey::Rope;

use crate::display_lines::line_store::{FormatKey, PaneLineStore};
use crate::display_lines::{DisplayLineMap, DisplayLinePos, h_window};
use crate::pane::WrapMode;
use crate::pipeline::ContentWidth;
use crate::providers::{ProviderSet, SyntaxSource};

/// The most header rows a pane shows.
pub const MAX_SCOPE_HEADERS: usize = 3;

/// A buffer's scope-header settings.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ContextSettings {
    /// Whether the buffer shows scope headers at all.
    pub enabled: bool,
    /// With more scopes than header rows, keep the outermost one and drop
    /// the one after it; otherwise drop the outermost.
    pub outer_level: bool,
}

/// What [`scope_headers`] reads a pane's scopes from.
#[derive(Copy, Clone)]
pub struct ScopeHeaderInputs<'a> {
    /// The buffer's syntax, `None` for a buffer without a grammar.
    pub source: Option<&'a dyn SyntaxSource>,
    pub settings: ContextSettings,
}

#[cfg(test)]
impl ScopeHeaderInputs<'static> {
    /// Inputs that yield no header.
    pub(crate) const NONE: Self = Self {
        source: None,
        settings: ContextSettings {
            enabled: false,
            outer_level: false,
        },
    };
}

/// The header lines covering rows `0..out.len()` of a pane `height` rows tall
/// whose top is `top`, top row first, written to `out`, replacing its
/// contents.
///
/// Built row by row: the line under the headers taken so far is the one at
/// row `out.len()`, and its outermost scope not yet taken becomes the next
/// header, until that line has none left. At most
/// `min(MAX_SCOPE_HEADERS, height - 1)` rows, so a row is always left for
/// text; past that cap each new header drops the outermost row, or with
/// [`ContextSettings::outer_level`] the row after it, and the line under the
/// headers stays at the cap's row. A header whose row would have no line
/// under it, past the document's end, is not shown.
pub fn scope_headers(
    dlm: &mut DisplayLineMap<'_>,
    top: DisplayLinePos,
    height: usize,
    inputs: ScopeHeaderInputs<'_>,
    out: &mut Vec<ContentLine>,
) {
    out.clear();
    let ScopeHeaderInputs {
        source: Some(source),
        settings,
    } = inputs
    else {
        return;
    };
    let cap = MAX_SCOPE_HEADERS.min(height.saturating_sub(1));
    if !settings.enabled || cap == 0 {
        return;
    }
    let mut pos = dlm.clamp(top);
    let mut taken = Vec::new();
    let mut scratch = Vec::new();
    while out.len() < cap {
        // A virtual display line's row belongs to the line it is anchored to.
        source.header_lines(pos.line, dlm.rope(), &mut scratch);
        let Some(&header) = scratch.iter().find(|line| !taken.contains(*line)) else {
            return;
        };
        let Some(next) = dlm.next(pos) else {
            return;
        };
        taken.push(header);
        out.push(header);
        pos = next;
    }
    // At the cap the line under the headers stays at row `cap`, so each of
    // its scopes not yet taken joins in turn, outer to inner, and drops a row.
    source.header_lines(pos.line, dlm.rope(), &mut scratch);
    for &header in scratch.iter().filter(|line| !taken.contains(*line)) {
        out.push(header);
        out.remove(if settings.outer_level { 1 } else { 0 });
    }
}

/// The format a header row lays its line out with: the pane's `key` with
/// wrapping off, so a long header line is cut at the pane's edge in every
/// wrap mode.
fn header_format_key(key: FormatKey) -> FormatKey {
    FormatKey {
        wrap_mode: WrapMode::None,
        ..key
    }
}

/// The map a header row is laid out through, whole lines, for a hit test.
///
/// `store` is rewound: the pane's providers (inlay hints, EOL text) shape its
/// lines but are not in `key`, so two panes on one buffer can share a key and
/// differ.
pub fn header_display_map<'a>(
    rope: &'a Rope,
    providers: &'a ProviderSet,
    content_width: ContentWidth,
    key: FormatKey,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    store.rewind();
    DisplayLineMap::new(
        rope,
        providers,
        content_width,
        header_format_key(key),
        store,
    )
}

/// [`header_display_map`] clipped to the columns the header rows draw, which
/// start at column 0 whatever the pane's horizontal scroll.
///
/// The clip is a render-only bound: a map with a window answers no bounded
/// query, so a hit test reads [`header_display_map`].
pub(crate) fn header_render_map<'a>(
    rope: &'a Rope,
    providers: &'a ProviderSet,
    content_width: ContentWidth,
    key: FormatKey,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    header_display_map(rope, providers, content_width, key, store).with_h_window(h_window(
        header_format_key(key).wrap_mode,
        DisplayLineCol::new(0),
        content_width,
    ))
}

/// The display line a header row draws: `line`'s first content display line.
pub fn header_display_line(dlm: &mut DisplayLineMap<'_>, line: ContentLine) -> DisplayLinePos {
    DisplayLinePos::new(line, dlm.block(line).before)
}

#[cfg(test)]
mod tests;
