//! Popup vocabulary shared by the scripting builtin that decodes it and the
//! editor that lays the popup out.

named_enum! {
    /// Where a popup renders: same widget, same model, two placements.
    /// `show-popup!`'s `#:anchor` symbol selects between them, decoded once
    /// at the builtin boundary (`hume-editor`'s `PopupLayer::layout` reads
    /// it).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PopupLayout {
        /// Floating, anchored near the focused pane's cursor (the default).
        Cursor => "cursor",
        /// A full-width band directly above the statusline that reserves
        /// pane space like the drawer, for content too tall for the cursor
        /// layout. It keeps popup semantics: plain scroll, no selection,
        /// closed by any other key.
        Docked => "bottom",
    }
}
