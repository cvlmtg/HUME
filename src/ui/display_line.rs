use ropey::RopeSlice;

/// A single line as presented on screen.
///
/// This is the unit the renderer iterates over. Currently every `DisplayLine`
/// maps 1:1 to a buffer line. The abstraction exists so future virtual lines
/// can be inserted without changing the renderer's core loop.
///
/// ## Lifetime
///
/// The `'buf` lifetime ties `content` to the `Buffer` that was passed to
/// the display-line iterator. The `DisplayLine` cannot outlive that borrow.
#[derive(Debug)]
pub(crate) struct DisplayLine<'buf> {
    /// The displayable text of this line.
    ///
    /// For buffer lines this is the rope slice for that line, with the
    /// trailing `\n` stripped. The newline is implicit in the row advance —
    /// the renderer never draws it.
    pub content: RopeSlice<'buf>,

    /// The 1-based line number shown in the gutter.
    ///
    /// `None` for virtual lines (diagnostics, ghost text) which don't
    /// correspond to a numbered buffer line.
    pub line_number: Option<usize>,

    /// The char offset in the buffer where this display line's content starts.
    ///
    /// Used to map a screen column back to a buffer position. `None` for
    /// virtual lines that have no direct buffer correspondence.
    pub char_offset: Option<usize>,
}
