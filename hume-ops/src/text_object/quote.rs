//! Inner/around quote text objects: `"`, `'`, `` ` ``.

use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};

use super::{apply_text_object_by_mode, inner_of_pair};
use crate::MotionMode;
use crate::pair::find_quote_pair;

fn inner_quote(text: &BufferText, pos: ClusterStart, quote: char) -> Option<ClusterRange> {
    inner_of_pair(text, find_quote_pair(text, pos, quote)?)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub fn $inner_name(state: EditState, _count: usize, mode: MotionMode) -> EditState {
            apply_text_object_by_mode(state, mode, |t, pos| inner_quote(t, pos, $quote))
        }
        pub fn $around_name(state: EditState, _count: usize, mode: MotionMode) -> EditState {
            apply_text_object_by_mode(state, mode, |t, pos| find_quote_pair(t, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');
