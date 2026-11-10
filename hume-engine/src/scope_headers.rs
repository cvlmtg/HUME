//! Scope headers: the first lines of the scopes a pane's top rows are
//! inside, painted over those rows.
//!
//! One walk decides them, which
//! [`ViewGeometry::scope_headers`](crate::pane::ViewGeometry::scope_headers)
//! runs for a pane's view. Rendering draws its answer,
//! [`ViewGeometry::top_margin_at`](crate::pane::ViewGeometry::top_margin_at)
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
const MAX_SCOPE_HEADERS: usize = 3;

/// A buffer's scope-header settings.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ContextSettings {
    /// Whether the buffer shows scope headers at all.
    pub enabled: bool,
    /// With more scopes than header rows, keep the outermost one and drop
    /// the one after it; otherwise drop the outermost.
    pub outer_level: bool,
}

/// What the scope-header walk reads a pane's scopes from.
#[derive(Copy, Clone)]
pub struct ScopeHeaderInputs<'a> {
    /// The buffer's syntax, `None` for a buffer without a grammar.
    pub source: Option<&'a dyn SyntaxSource>,
    pub settings: ContextSettings,
}

#[cfg(test)]
impl ContextSettings {
    /// Settings that show no header.
    pub(crate) const OFF: Self = Self {
        enabled: false,
        outer_level: false,
    };
}

#[cfg(test)]
impl ScopeHeaderInputs<'static> {
    /// Inputs that yield no header.
    pub(crate) const NONE: Self = Self {
        source: None,
        settings: ContextSettings::OFF,
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
pub(crate) fn scope_headers(
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
    // The scopes of `scratch_line`, which a wrapped or virtual-line block
    // keeps under the headers for several rows.
    let mut scratch = Vec::new();
    let mut scratch_line = None;
    loop {
        // A virtual display line's row belongs to the line it is anchored to.
        if scratch_line != Some(pos.line) {
            source.header_lines(pos.line, dlm.rope(), &mut scratch);
            scratch_line = Some(pos.line);
        }
        if out.len() == cap {
            break;
        }
        let Some(&header) = scratch.iter().find(|line| !out.contains(*line)) else {
            return;
        };
        let Some(next) = dlm.next(pos) else {
            return;
        };
        out.push(header);
        pos = next;
    }
    // At the cap the line under the headers stays at row `cap`, so each of
    // its scopes not yet taken joins in turn, outer to inner, and drops a row.
    scratch.retain(|line| !out.contains(line));
    for &header in &scratch {
        out.push(header);
        out.remove(if settings.outer_level { 1 } else { 0 });
    }
}

/// What header rows are laid out with: a line store of their own, apart
/// from the pane's, whose key they do not share, and no decoration
/// providers, so a header row shows its line's text alone.
#[derive(Default)]
pub struct HeaderLayout {
    store: PaneLineStore,
    providers: ProviderSet,
}

impl HeaderLayout {
    /// The map a header row is laid out through, whole lines, for a hit
    /// test: the pane's `key` with wrapping off, so a long header line is
    /// cut at the pane's edge in every wrap mode.
    pub fn display_map<'a>(
        &'a mut self,
        rope: &'a Rope,
        content_width: ContentWidth,
        key: FormatKey,
    ) -> DisplayLineMap<'a> {
        let key = FormatKey {
            wrap_mode: WrapMode::None,
            ..key
        };
        DisplayLineMap::new(rope, &self.providers, content_width, key, &mut self.store)
    }

    /// [`Self::display_map`] clipped to the columns the header rows draw,
    /// which start at column 0 whatever the pane's horizontal scroll.
    ///
    /// The clip is a render-only bound: a map with a window answers no
    /// bounded query, so a hit test reads [`Self::display_map`].
    pub(crate) fn render_map<'a>(
        &'a mut self,
        rope: &'a Rope,
        content_width: ContentWidth,
        key: FormatKey,
    ) -> DisplayLineMap<'a> {
        self.display_map(rope, content_width, key)
            .with_h_window(h_window(
                WrapMode::None,
                DisplayLineCol::new(0),
                content_width,
            ))
    }
}

#[cfg(test)]
mod tests;
