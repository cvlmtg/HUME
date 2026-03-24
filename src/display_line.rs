use ropey::RopeSlice;

/// Where a display line's content comes from.
///
/// Every line the renderer draws must have a source. Currently the only source
/// is a real buffer line, but the enum exists so that virtual lines
/// (diagnostics, ghost text, soft-wrap continuations) can be added later
/// without changing the renderer's iteration contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayLineSource {
    /// A real line from the buffer. `line_idx` is the 0-based line index.
    BufferLine { line_idx: usize },
    // Future variants (not yet implemented):
    // VirtualDiagnostic { after_line: usize }
    // SoftWrapContinuation { line_idx: usize, segment: usize }
}

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
    /// Where this line comes from (buffer line, virtual line, etc.).
    // Not yet consumed by the renderer; will be used when virtual lines (diagnostics, ghost text)
    // are introduced and need to be distinguished from real buffer lines.
    #[allow(dead_code)]
    pub source: DisplayLineSource,

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
