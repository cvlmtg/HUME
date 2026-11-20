//! Tree-sitter queries over a buffer's syntax tree.

use std::sync::Arc;

use hume_core::BufferId;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;

/// What a query captured: each match as the entries of its nodes, each
/// entry naming one of `names`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captures {
    /// The query's capture names, each once.
    pub names: Vec<Arc<str>>,
    pub matches: Vec<Vec<CaptureEntry>>,
}

/// One node a query captured, its range half-open in chars of the buffer's
/// text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureEntry {
    /// Index into [`Captures::names`].
    pub name: u32,
    pub start_char: usize,
    pub end_char: usize,
}

/// Queries over a buffer's syntax tree, accessed through
/// [`EditorHost::syntax`](super::EditorHost::syntax). Backs
/// `(syntax-captures pane language query-source #:ranges ranges)`.
pub trait SyntaxHost {
    /// The matches of `query`, compiled against `language`'s grammar, over
    /// each layer of `bid`'s tree parsed with that grammar, injected layers
    /// included, each as the entries of its captures, sorted as
    /// `hume_treesitter::captures::CaptureQuery::collect` sorts them: within
    /// a match by start then end descending, enclosing node first, and the
    /// matches by their entries, each distinct match once and none empty. A
    /// node several matches capture is in each. The tree is brought up to
    /// date with the text first; empty when the buffer has no tree for its
    /// current text. `Err` naming the language when it has no grammar, and
    /// with the position and kind of the error when `query` does not compile.
    ///
    /// A match counts when the node its pattern is rooted at intersects the
    /// lines of one of `ranges`, with every capture of the match, inside those
    /// lines or not. `ranges` come in any order and may overlap, each ending
    /// at most one past the last content line.
    fn syntax_captures(
        &mut self,
        bid: BufferId,
        language: &str,
        query: &str,
        ranges: &[ExclusiveRange<ContentLine>],
    ) -> Result<Captures, String>;
}
