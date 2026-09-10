//! Inner/around quote text objects: `"`, `'`, `` ` ``.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, InclusiveRange};

use super::apply_text_object_by_mode;
use super::bracket::inner_of_pair;
use crate::MotionMode;
use crate::pair::find_quote_pair;

fn inner_quote(
    text: &BufferText,
    pos: CharOffset,
    quote: char,
) -> Option<InclusiveRange<CharOffset>> {
    let pair = find_quote_pair(text, pos, quote)?;
    inner_of_pair(pair)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub fn $inner_name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(text, sels, mode, |b, pos| inner_quote(b, pos, $quote))
        }
        pub fn $around_name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(text, sels, mode, |b, pos| find_quote_pair(b, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');
