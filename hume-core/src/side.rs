//! Decoration vocabulary shared by the scripting builtins that decode it,
//! the stores that keep it, and the editor that renders it.

named_enum! {
    /// Which side of its anchor an inlay hint or a virtual line sits on: an
    /// inlay hint's `'side`, a virtual line's `'anchor`. `Before < After` is
    /// the order a line's virtual lines render in around its content.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Side {
        Before => "before",
        After => "after",
    }
}
