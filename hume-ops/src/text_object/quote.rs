//! Inner/around quote text objects: `"`, `'`, `` ` ``.

use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

use super::apply_text_object_by_mode;
use super::bracket::inner_of_pair;
use crate::MotionMode;
use crate::pair::find_quote_pair;

fn inner_quote(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
    let (open, close) = find_quote_pair(buf, pos, quote)?;
    inner_of_pair(open, close)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub fn $inner_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_quote(b, pos, $quote))
        }
        pub fn $around_name(
            buf: &Text,
            sels: SelectionSet,
            _count: usize,
            mode: MotionMode,
        ) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| find_quote_pair(b, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');
