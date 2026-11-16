//! A text drawn outside a pane, such as a popup's: each line laid out and
//! styled as a pane lays out and styles a scope header row, then handed
//! over as styled runs for the caller to wrap and draw.

use std::borrow::Cow;

use hume_grid::Rgb;
use hume_rope::line::ContentLine;
use hume_rope::lines::content_line_count;
use hume_rope::offset::ExclusiveRange;
use hume_rope::width::CHROME_TAB_WIDTH;
use ropey::Rope;

use crate::display_lines::DisplayLineInputs;
use crate::display_lines::line_store::FormatKey;
use crate::folds::HiddenLines;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::pipeline::{ContentWidth, PaneSize};
use crate::providers::{ProviderSet, SyntaxSource};
use crate::render::resolve_arena_text;
use crate::reveal::RevealedLines;
use crate::scope_headers::HeaderLayout;
use crate::style::{LineStyle, StyleScratch, line_base, style_display_line};
use crate::theme::Theme;
use crate::types::{CellContent, ResolvedStyle};

/// One line of a text as [`styled_lines`] draws it.
#[derive(Clone, Debug, PartialEq)]
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
    syntax: Option<&dyn SyntaxSource>,
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
        } = LineStyle::enter(line, rope, syntax, providers, theme, &mut scratch);
        let pos = map.first_content_display_line(line);
        let rendered = map.render_display_line(pos);
        scratch
            .styles
            .resize(rendered.graphemes.len(), ResolvedStyle::default());
        style_display_line(
            rendered.display_line,
            rendered.graphemes,
            chars,
            None,
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
            let text = match g.content {
                CellContent::Grapheme => {
                    Cow::Borrowed(&rendered.line_text[g.byte_range.as_byte_range()])
                }
                CellContent::Whitespace { start, len }
                | CellContent::Placeholder { start, len }
                | CellContent::Virtual { start, len }
                | CellContent::Replacement { start, len } => {
                    Cow::Borrowed(resolve_arena_text(rendered.virtual_texts, start, len))
                }
                CellContent::TabFill => Cow::Owned(" ".repeat(usize::from(g.width))),
                // The second cell of a wide cluster, and the end-of-line
                // sentinel, draw nothing of their own.
                CellContent::WidthContinuation | CellContent::Empty => continue,
            };
            push_run(&mut runs, &text, style);
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
