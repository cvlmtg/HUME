//! One row of a scrolling list (drawer, menu, picker): its text, the parts
//! of it a theme scope styles, and the scope that styles the whole row while
//! it is the selected one.

use std::sync::Arc;

use hume_core::TruncateEnd;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::types::{ResolvedStyle, ScopeId};
use hume_grid::Rect;
use hume_rope::spanned::{self, SpannedText};

/// The scopes that style one span of a list row: `scope`, or `selected` in
/// its place while the row is the selected one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanScopes {
    pub scope: ScopeId,
    pub selected: Option<ScopeId>,
}

impl SpanScopes {
    fn for_row(self, is_selected: bool) -> ScopeId {
        match self.selected {
            Some(selected) if is_selected => selected,
            _ => self.scope,
        }
    }
}

/// One list row: its text, the parts of it a theme scope styles, and the
/// scope that styles the whole row in place of the list's selected-row style
/// while it is the selected one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    content: SpannedText<SpanScopes>,
    selected_scope: Option<ScopeId>,
}

impl ListRow {
    /// A row with no styled part.
    pub fn plain(text: String) -> Self {
        Self::styled(SpannedText::plain(text))
    }

    /// A row whose spans style parts of its text.
    pub fn styled(content: SpannedText<SpanScopes>) -> Self {
        Self {
            content,
            selected_scope: None,
        }
    }

    /// This row with `scope` styling it in place of the list's selected-row
    /// style while it is the selected row. A theme that leaves `scope`
    /// unstyled selects the row in its default style.
    pub fn with_selected_scope(self, scope: Option<ScopeId>) -> Self {
        Self {
            selected_scope: scope,
            ..self
        }
    }

    pub fn text(&self) -> &str {
        self.content.text()
    }

    /// This row cut to `budget` cells at its `cut` end, by
    /// [`truncate_spanned`](hume_rope::width::truncate_spanned).
    pub fn fit(self, budget: usize, cut: TruncateEnd) -> Self {
        Self {
            content: hume_rope::width::truncate_spanned(self.content, budget, cut),
            ..self
        }
    }

    /// This row cut to `budget` cells at its `cut` end, as [`Self::fit`]
    /// cuts it, sharing `self` when it already fits.
    pub fn fit_shared(self: &Arc<Self>, budget: usize, cut: TruncateEnd) -> Arc<Self> {
        if hume_rope::width::chrome_width(self.text()) <= budget {
            Arc::clone(self)
        } else {
            Arc::new(Self::clone(self).fit(budget, cut))
        }
    }

    /// This row followed by `after`, unstyled.
    pub(crate) fn append(self, after: &str) -> Self {
        Self {
            content: self.content.append(after),
            ..self
        }
    }

    /// The whole text as consecutive runs, each with the style it adds over
    /// the row's own: a span adds the style of its scope for a selected or
    /// unselected row, and the text between spans adds nothing.
    fn runs<'a>(
        &'a self,
        theme: &'a Theme,
        is_selected: bool,
    ) -> impl Iterator<Item = (&'a str, ResolvedStyle)> {
        spanned::runs(self.content.text(), self.content.spans()).map(move |(run, scopes)| {
            let style = scopes.map_or_else(ResolvedStyle::default, |scopes| {
                theme.resolve_layer(scopes.for_row(is_selected))
            });
            (run, style)
        })
    }
}

/// Paint one list row over `area`, one cell row tall: a highlight bar
/// across it when `is_selected`, then the row's runs up to its right edge,
/// each layered over `selected_style` when `is_selected` or `base_style`
/// otherwise. The row's selected scope replaces `selected_style`. `None` is
/// a row not rendered yet, which paints only its highlight.
pub(crate) fn draw_row(
    canvas: &mut Canvas,
    area: Rect,
    row: Option<&ListRow>,
    theme: &Theme,
    is_selected: bool,
    selected_style: ResolvedStyle,
    base_style: ResolvedStyle,
) {
    let row_style = if is_selected {
        let selected_style = row
            .and_then(|row| row.selected_scope)
            .map_or(selected_style, |scope| theme.resolve(scope));
        canvas.fill_rect_bg(area, selected_style);
        selected_style
    } else {
        base_style
    };
    let runs = row
        .into_iter()
        .flat_map(|row| row.runs(theme, is_selected))
        .map(|(text, over)| (text, row_style.layer(over)));
    paint_styled_row(canvas, area.x, area.y, runs, area.right());
}

/// Paint `runs` left to right from `(x, y)`, each in its own style, stopping
/// at `right_edge`. Every row goes through it: a list's rows ([`draw_row`])
/// and a popup's lines in [`draw_menu_box`](crate::menu_box::draw_menu_box).
pub(crate) fn paint_styled_row<'a>(
    canvas: &mut Canvas,
    x: u16,
    y: u16,
    runs: impl IntoIterator<Item = (&'a str, ResolvedStyle)>,
    right_edge: u16,
) {
    let mut cx = x;
    for (run_text, run_style) in runs {
        cx = canvas.write_text_run(cx, y, run_text, run_style, right_edge);
    }
}

#[cfg(test)]
mod tests;
