//! Test-only scaffolding shared across this crate's per-module test suites:
//! shorthand for building a `Theme` from a literal list of scope styles,
//! for building a `Grapheme::byte_range` from plain integers, and for
//! assembling a `RenderDisplayLine` by hand for a direct `compose_display_line` call.

use std::collections::HashMap;

use hume_grid::Rgb;
use hume_rope::column::ByteCol;
use hume_rope::offset::ExclusiveRange;

use crate::display_lines::RenderDisplayLine;
use crate::theme::Theme;
use crate::types::{DisplayLine, Grapheme, ResolvedStyle};

pub(crate) fn fg(color: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(color),
        ..Default::default()
    }
}

pub(crate) fn bg(color: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        bg: Some(color),
        ..Default::default()
    }
}

pub(crate) fn theme_with<const N: usize>(entries: [(&'static str, ResolvedStyle); N]) -> Theme {
    Theme::new(HashMap::from(entries), ResolvedStyle::default())
}

pub(crate) fn byte_range(start: usize, end: usize) -> ExclusiveRange<ByteCol> {
    ExclusiveRange::new(ByteCol::new(start), ByteCol::new(end))
}

/// Build a `RenderDisplayLine` for a direct `compose_display_line` call —
/// every test site has no provider-owned background, so `base_scope` is
/// always `None` here rather than a parameter.
pub(crate) fn render_display_line<'m>(
    display_line: &'m DisplayLine,
    graphemes: &'m [Grapheme],
    line_text: &'m str,
    virtual_texts: &'m str,
) -> RenderDisplayLine<'m> {
    RenderDisplayLine {
        display_line,
        graphemes,
        line_text,
        virtual_texts,
        base_scope: None,
    }
}
