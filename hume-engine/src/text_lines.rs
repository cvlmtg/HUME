//! A text drawn outside a pane, such as a popup's: each line laid out as a
//! pane lays out a scope header row, styled as a pane styles a line it
//! draws without selections, then handed over as styled runs for the caller
//! to wrap and draw.

use hume_grid::Rgb;
use hume_rope::line::ContentLine;
use hume_rope::lines::content_line_count;
use hume_rope::offset::ExclusiveRange;
use hume_rope::width::{CHROME_TAB_WIDTH, chrome_width};
use ropey::Rope;

use crate::display_lines::DisplayLineInputs;
use crate::display_lines::line_store::FormatKey;
use crate::folds::HiddenLines;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::pipeline::{ContentWidth, PaneSize};
use crate::providers::{ProviderSet, SyntaxSource};
use crate::render::{CellDraw, cell_draw};
use crate::reveal::RevealedLines;
use crate::scope_headers::HeaderLayout;
use crate::style::{LineStyle, StyleScratch, line_base, style_unselected_line};
use crate::theme::Theme;
use crate::types::{CellContent, ResolvedStyle};

/// One line of a text as [`styled_lines`] draws it.
pub struct StyledLine {
    /// The line's cells as runs of text, none holding a `\n`.
    pub runs: Vec<(String, ResolvedStyle)>,
    /// The background of the line's tint, `None` for none.
    pub fill: Option<Rgb>,
}

/// Every content line of `rope`, unwrapped, with the conceals of `providers`
/// applied to every line, and styled over `base` by `syntax`'s highlights and
/// `providers`' highlights and line tints, as a pane styles a line it draws
/// without selections. A tab is one column wide, as in other chrome text.
pub fn styled_lines(
    rope: &Rope,
    syntax: &dyn SyntaxSource,
    providers: &ProviderSet,
    base: ResolvedStyle,
    theme: &Theme,
) -> Vec<StyledLine> {
    let mut layout = HeaderLayout::default();
    let inputs = DisplayLineInputs {
        format: FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::None,
            tab_width: CHROME_TAB_WIDTH,
            whitespace: WhitespaceConfig::default(),
        },
        hidden: HiddenLines::default(),
        // Unwrapped, a line's layout does not depend on the width.
        content_width: ContentWidth::of(PaneSize::new(1, 1), 0),
        revealed: RevealedLines::default(),
    };
    let mut map = layout.display_map(rope, providers, &inputs);
    let mut scratch = StyleScratch::new();
    ExclusiveRange::new(
        ContentLine::default(),
        content_line_count(rope).end_exclusive(),
    )
    .iter()
    .map(|line| {
        let LineStyle {
            chars,
            tint_bg: fill,
        } = LineStyle::enter(line, rope, Some(syntax), providers, theme, &mut scratch);
        let rendered = style_unselected_line(
            &mut map,
            line,
            chars,
            line_base(base, fill),
            theme,
            &mut scratch,
        );
        let cells = rendered.display_line.graphemes.clone();
        let mut runs = Vec::new();
        for (g, &style) in rendered.graphemes[cells.clone()]
            .iter()
            .zip(&scratch.styles[cells])
        {
            // A popup line has no cursor to stand past its end.
            if matches!(g.content, CellContent::Empty) {
                continue;
            }
            match cell_draw(g, rendered.line_text, rendered.virtual_texts) {
                Some(CellDraw::Glyph(text)) => push_run(&mut runs, text, style),
                Some(CellDraw::Run(text)) => {
                    push_run(&mut runs, text, style);
                    let blank = usize::from(g.width).saturating_sub(chrome_width(text));
                    push_run(&mut runs, &" ".repeat(blank), style);
                }
                None => {}
            }
        }
        StyledLine { runs, fill }
    })
    .collect()
}

/// `text` in `style` appended to `runs`, merged into the last run when it
/// has the same style. Empty text adds nothing.
pub fn push_run(runs: &mut Vec<(String, ResolvedStyle)>, text: &str, style: ResolvedStyle) {
    if text.is_empty() {
        return;
    }
    match runs.last_mut() {
        Some((last, last_style)) if *last_style == style => last.push_str(text),
        _ => runs.push((text.to_owned(), style)),
    }
}
