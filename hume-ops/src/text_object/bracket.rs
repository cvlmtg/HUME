//! Inner/around bracket-pair text objects: `()`, `[]`, `{}`, `<>`.

use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};

use super::{apply_text_object_by_mode, inner_of_pair};
use crate::MotionMode;
use crate::pair::find_bracket_pair;

fn inner_bracket(
    text: &BufferText,
    pos: ClusterStart,
    open: char,
    close: char,
) -> Option<ClusterRange> {
    inner_of_pair(text, find_bracket_pair(text, pos, open, close)?)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $open:literal, $close:literal) => {
        pub fn $inner_name(state: EditState, _count: usize, mode: MotionMode) -> EditState {
            apply_text_object_by_mode(state, mode, |t, pos| inner_bracket(t, pos, $open, $close))
        }
        pub fn $around_name(state: EditState, _count: usize, mode: MotionMode) -> EditState {
            apply_text_object_by_mode(state, mode, |t, pos| {
                find_bracket_pair(t, pos, $open, $close)
            })
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, '<', '>');
