use ropey::RopeSlice;

/// A single line as presented on screen.
///
/// This is the unit the renderer iterates over. A `DisplayLine` may map 1:1
/// to a buffer line, or it may be a soft-wrap continuation or a virtual line
/// (diagnostics, ghost text). The abstraction lets the renderer's core loop
/// stay the same regardless of source.
///
/// ## Lifetime
///
/// The `'buf` lifetime ties `content` to the `Buffer` that was passed to
/// the display-line iterator. The `DisplayLine` cannot outlive that borrow.
#[derive(Debug)]
pub(crate) struct DisplayLine<'buf> {
    /// The displayable text of this line.
    ///
    /// For buffer lines this is the rope slice for that line (or a wrapped
    /// segment of it), with the trailing `\n` stripped. The newline is
    /// implicit in the row advance — the renderer never draws it.
    pub content: RopeSlice<'buf>,

    /// The 1-based line number shown in the gutter.
    ///
    /// `None` for continuation rows (soft-wrap) and virtual lines
    /// (diagnostics, ghost text) which don't start a new buffer line.
    pub line_number: Option<usize>,

    /// The char offset in the buffer where this display line's content starts.
    ///
    /// Used to map a screen column back to a buffer position. `None` for
    /// virtual lines that have no direct buffer correspondence. For
    /// soft-wrap continuations this points to the first char of the segment.
    pub char_offset: Option<usize>,

    /// `true` for soft-wrap continuation rows — the second, third, etc.
    /// display rows of a buffer line that was too long to fit in one row.
    /// The gutter is left blank for these rows (no line number, no indicator).
    pub is_continuation: bool,
}
