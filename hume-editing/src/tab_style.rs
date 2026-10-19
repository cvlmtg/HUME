//! How indentation whitespace is written: what the Tab key inserts in Insert
//! mode, and how a leading-whitespace width is rendered.

use hume_rope::column::BufferLineCol;
use hume_rope::width::indent_stop;

/// `Hard` inserts a literal `\t` character; `Soft` inserts enough spaces to
/// reach the next tab stop (governed by `indent-width`). This is the single knob;
/// there is no separate "shiftwidth" or "softtabstop": `indent-width` is the
/// only width, used for both rendering and Tab-key spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabStyle {
    /// Tab key inserts one `\t` character per press.
    #[default]
    Hard,
    /// Tab key inserts spaces up to the next tab stop.
    Soft,
}

impl TabStyle {
    /// `width` columns of leading whitespace: tabs then spaces for `Hard`
    /// (a tab is one indent stop wide), spaces for `Soft`.
    pub fn render_indent(self, width: BufferLineCol, tab_width: u8) -> String {
        let width = width.get() as usize;
        match self {
            Self::Soft => " ".repeat(width),
            Self::Hard => {
                let tw = indent_stop(1, tab_width) as usize;
                std::iter::repeat_n('\t', width / tw)
                    .chain(std::iter::repeat_n(' ', width % tw))
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests;
