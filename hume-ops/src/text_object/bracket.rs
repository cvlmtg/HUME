//! Inner/around bracket-pair text objects: `()`, `[]`, `{}`, `<>`.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, InclusiveRange};

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::pair::find_bracket_pair;

/// Shrink a `(open, close)` delimiter pair to its inner range, or `None` if
/// the pair is empty (no inner content in the inclusive selection model).
/// Shared with quote.rs's `inner_quote`.
pub(super) fn inner_of_pair(
    pair: InclusiveRange<CharOffset>,
) -> Option<InclusiveRange<CharOffset>> {
    if pair.start.index() + 1 > pair.end.index() - 1 {
        return None;
    }
    Some(InclusiveRange::new(
        CharOffset::new(pair.start.index() + 1),
        CharOffset::new(pair.end.index() - 1),
    ))
}

fn inner_bracket(
    text: &BufferText,
    pos: CharOffset,
    open: char,
    close: char,
) -> Option<InclusiveRange<CharOffset>> {
    let pair = find_bracket_pair(text, pos, open, close)?;
    inner_of_pair(pair)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $open:literal, $close:literal) => {
        pub fn $inner_name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(text, sels, mode, |b, pos| {
                inner_bracket(b, pos, $open, $close)
            })
        }
        pub fn $around_name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(text, sels, mode, |b, pos| {
                find_bracket_pair(b, pos, $open, $close)
            })
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, '<', '>');
