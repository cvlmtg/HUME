//! Test-only scaffolding shared across this crate's per-module test suites:
//! shorthand for building a `Theme` from a literal list of scope styles.

use std::collections::HashMap;

use hume_grid::Rgb;

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
