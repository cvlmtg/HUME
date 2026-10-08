//! What the Tab key inserts in Insert mode.

/// `Hard` inserts a literal `\t` character; `Soft` inserts enough spaces to
/// reach the next tab stop (governed by `tab-width`). This is the single knob;
/// there is no separate "shiftwidth" or "softtabstop": `tab-width` is the
/// only width, used for both rendering and Tab-key spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabStyle {
    /// Tab key inserts one `\t` character per press.
    #[default]
    Hard,
    /// Tab key inserts spaces up to the next tab stop.
    Soft,
}
