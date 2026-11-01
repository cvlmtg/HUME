//! List-row vocabulary shared by the scripting builtins that decode it and
//! the editor and widgets that cut rows with it.

named_enum! {
    /// Which end of an over-long row is dropped: the `#:truncate` symbol of
    /// `picker!`, `live-picker!` and `show-drawer-list!`, and `string-fit`'s
    /// side, decoded once at the builtin boundary. Menus and tab labels cut
    /// their tail. A path's distinguishing part (the basename) sits at the
    /// end, so cutting the head is the default and the pickers'; a row whose
    /// distinguishing part sits at the front (a grep match's file path, say,
    /// before the line preview) wants `'tail`, the drawer's default.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum TruncateEnd {
        #[default]
        Head => "head",
        Tail => "tail",
    }
}
