//! Popup vocabulary shared by the scripting builtin that decodes it and the
//! editor that lays the popup out.

/// Where a popup renders: same widget, same model, two placements.
/// `show-popup!`'s `#:anchor` symbol selects between them, decoded once at
/// the builtin boundary (`hume-editor`'s `PopupLayer::layout` reads it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupLayout {
    /// Floating, anchored near the focused pane's cursor (`#:anchor
    /// 'cursor`, the default), painted by `hume-ui`'s `PopupOverlay`.
    Cursor,
    /// Docked as a full-width chrome band directly above the statusline,
    /// reserving pane space like the drawer (`#:anchor 'bottom`), painted
    /// by `hume-ui`'s `PopupBandWidget`. Used for hover content too tall for
    /// the cursor layout; keeps popup semantics (plain scroll, no selection,
    /// close-on-any-other-key) rather than becoming a pick-list.
    Docked,
}
