//! Test-only scaffolding shared across this crate's per-module test suites:
//! shorthand for building a `Theme` from a literal list of scope styles, and
//! for building a `Grapheme::byte_range` from plain integers.

use std::collections::HashMap;

use hume_grid::Rgb;
use hume_rope::column::ByteCol;
use hume_rope::offset::ExclusiveRange;

use crate::theme::Theme;
use crate::types::ResolvedStyle;

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
