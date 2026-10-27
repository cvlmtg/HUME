//! Decoration vocabulary shared by the scripting builtins that decode it,
//! the stores that keep it, and the editor that renders it.

/// Which side of its anchor an inlay hint or a virtual line sits on: an
/// inlay hint's `'side`, a virtual line's `'anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
}
