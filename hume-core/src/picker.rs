//! Picker vocabulary shared by the scripting builtins that decode it and the
//! widget that paints it.

/// Which end of an over-long picker row is dropped: `picker!`'s and
/// `live-picker!`'s `#:truncate` symbol, decoded once at the builtin
/// boundary and carried as-is into the panel's paint-time clip. A path's
/// distinguishing part (the basename) sits at the end, so cutting the head
/// is the default; a row whose distinguishing part sits at the front (a grep
/// match's file path, say, before the line preview) wants `'tail` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TruncateEnd {
    #[default]
    Head,
    Tail,
}
