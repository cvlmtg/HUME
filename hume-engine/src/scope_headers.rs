//! Scope headers: the first lines of the scopes a pane's top rows are
//! inside, painted over those rows.
//!
//! [`PaneRows`] holds a pane's rows with its headers walked once: rendering
//! draws them, [`ViewGeometry::top_margin_at`] keeps the cursor below them,
//! and the caret, a popup's anchor and a mouse hit read through it which row
//! shows what.

use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;
use ropey::Rope;

use crate::display_lines::line_store::{FormatKey, PaneLineStore};
use crate::display_lines::{DisplayLineInputs, DisplayLineMap, DisplayLinePos, h_window};
use crate::folds::HiddenLines;
use crate::pane::{ViewGeometry, WrapMode};
use crate::providers::{DecorationKinds, ProviderSet, SyntaxSource};

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

/// A pane's rows while `top` is its top: the scope header rows over its top
/// rows, then the display lines from `top` on under them. Render, the
/// caret, a popup's anchor and a mouse hit read a pane's rows from one of
/// these, minted by [`ViewGeometry::rows`] or returned by the scroll verb
/// that settled the view.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PaneRows {
    top: DisplayLinePos,
    height: usize,
    headers: HeaderLines,
}

/// The lines a pane's scope header rows draw, top row first.
#[derive(Copy, Clone, Debug, Default)]
pub struct HeaderLines {
    lines: [ContentLine; MAX_SCOPE_HEADERS],
    count: usize,
}

impl HeaderLines {
    pub fn as_slice(&self) -> &[ContentLine] {
        &self.lines[..self.count]
    }
}

impl PartialEq for HeaderLines {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for HeaderLines {}

/// What one row of a pane shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PaneRow {
    /// A scope header row, drawing this line.
    Header(ContentLine),
    /// This display line, below the header rows.
    Body(DisplayLinePos),
}

impl PaneRows {
    /// The rows of a pane laid out as `geo` whose top is `top`, an address
    /// [`Viewport::top_at`](crate::pane::Viewport::top_at) resolved.
    ///
    /// The headers are built row by row: the line under the headers taken so
    /// far is the one at the next row, and its outermost scope not yet taken
    /// becomes the next header, until that line has none left. At most
    /// `min(MAX_SCOPE_HEADERS, (height - 1) / 2)` rows, the most the top
    /// margin [`ViewGeometry::top_margin_at`] keeps the cursor below; past
    /// that cap each new header drops the outermost row, or with
    /// [`ContextSettings::outer_level`] the row after it, and the line under
    /// the headers stays at the cap's row. A header whose row would have no
    /// line under it, past the document's end, is not shown.
    pub(crate) fn walk(
        dlm: &mut DisplayLineMap<'_>,
        geo: &ViewGeometry<'_>,
        top: DisplayLinePos,
    ) -> Self {
        let mut rows = Self {
            top,
            height: geo.height,
            headers: HeaderLines::default(),
        };
        let ScopeHeaderInputs {
            source: Some(source),
            settings,
        } = geo.headers
        else {
            return rows;
        };
        let cap = MAX_SCOPE_HEADERS.min((geo.height - 1) / 2);
        if !settings.enabled || cap == 0 {
            return rows;
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
                // A closed fold can hide the header of a scope that runs past it.
                scratch.retain(|line| !dlm.hidden().is_hidden(*line));
                scratch_line = Some(pos.line);
            }
            if rows.headers.count == cap {
                break;
            }
            let Some(&header) = scratch.iter().find(|line| !rows.headers().contains(*line)) else {
                return rows;
            };
            let Some(next) = dlm.next(pos) else {
                return rows;
            };
            rows.headers.lines[rows.headers.count] = header;
            rows.headers.count += 1;
            pos = next;
        }
        // At the cap the line under the headers stays at row `cap`, so each of
        // its scopes not yet taken joins in turn, outer to inner, as the last
        // row and drops a row. With one row and `outer_level` the newcomer is
        // the row dropped.
        scratch.retain(|line| !rows.headers().contains(line));
        let dropped = usize::from(settings.outer_level);
        for &header in &scratch {
            if dropped < cap {
                rows.headers.lines.copy_within(dropped + 1..cap, dropped);
                rows.headers.lines[cap - 1] = header;
            }
        }
        rows
    }

    /// The display line at the pane's top row, under any header row.
    pub fn top(&self) -> DisplayLinePos {
        self.top
    }

    /// The line each header row draws, top row first.
    pub fn headers(&self) -> &[ContentLine] {
        self.headers.as_slice()
    }

    /// [`Self::headers`] as a value to keep past these rows.
    pub fn header_lines(&self) -> HeaderLines {
        self.headers
    }

    /// The display line drawn on the first row below the header rows.
    pub fn body_top(&self, dlm: &mut DisplayLineMap<'_>) -> DisplayLinePos {
        dlm.advance_saturating(self.top, self.headers.count as isize)
    }

    /// What pane row `row` shows.
    pub fn row_at(&self, dlm: &mut DisplayLineMap<'_>, row: u16) -> PaneRow {
        match self.headers().get(usize::from(row)) {
            Some(&line) => PaneRow::Header(line),
            None => PaneRow::Body(dlm.advance_saturating(self.top, row as isize)),
        }
    }

    /// The pane row display line `pos` is drawn on: `None` before the top,
    /// past the pane's last row, or under a header row.
    pub fn row_of(&self, dlm: &mut DisplayLineMap<'_>, pos: DisplayLinePos) -> Option<usize> {
        dlm.distance(self.top, pos, self.height - 1)
            .filter(|&row| row >= self.headers.count)
    }
}

/// What header rows are laid out with: a line store of their own, apart
/// from the pane's, whose key they do not share.
///
/// A header row lays its line out with the pane's conceals and paints the
/// pane's highlights, so a pinned heading draws as its body does. It shows
/// no inline insert, virtual line, line tint or selection.
#[derive(Default)]
pub struct HeaderLayout {
    store: PaneLineStore,
}

impl HeaderLayout {
    /// The map a header row is laid out through, whole lines, for a hit
    /// test: the pane's `inputs` with wrapping off, so a long header line is
    /// cut at the pane's edge in every wrap mode, and with no line hidden,
    /// since each row lays out one line on its own. The pane's revealed
    /// lines stay, so a revealed heading draws raw.
    pub fn display_map<'a>(
        &'a mut self,
        rope: &'a Rope,
        providers: &'a ProviderSet,
        inputs: &DisplayLineInputs,
    ) -> DisplayLineMap<'a> {
        let inputs = DisplayLineInputs {
            format: FormatKey {
                wrap_mode: WrapMode::None,
                ..inputs.format
            },
            hidden: HiddenLines::default(),
            content_width: inputs.content_width,
            revealed: inputs.revealed.clone(),
        };
        DisplayLineMap::narrowed(
            rope,
            providers,
            inputs,
            &mut self.store,
            DecorationKinds::CONCEAL,
        )
    }

    /// [`Self::display_map`] clipped to the columns the header rows draw,
    /// which start at column 0 whatever the pane's horizontal scroll.
    ///
    /// The clip is a render-only bound: a map with a window answers no
    /// bounded query, so a hit test reads [`Self::display_map`].
    pub(crate) fn render_map<'a>(
        &'a mut self,
        rope: &'a Rope,
        providers: &'a ProviderSet,
        inputs: &DisplayLineInputs,
    ) -> DisplayLineMap<'a> {
        self.display_map(rope, providers, inputs)
            .with_h_window(h_window(
                WrapMode::None,
                DisplayLineCol::new(0),
                inputs.content_width,
            ))
    }
}

#[cfg(test)]
mod tests;
