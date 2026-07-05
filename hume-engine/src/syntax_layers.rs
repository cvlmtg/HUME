use std::sync::Arc;

use crate::builtins::tree_sitter_hl::TreeSitterHighlighter;

/// One parsed layer of a buffer's syntax tree: the root grammar, or one
/// embedded-language injection (a fenced code block, a combined
/// `markdown.inline` layer, etc.).
pub struct SyntaxLayer {
    pub tree: tree_sitter::Tree,
    /// Shared per-language highlighter — `Arc`'d from the language's
    /// `GrammarBundle`, not owned per-buffer.
    pub highlighter: Arc<TreeSitterHighlighter>,
    /// Absolute byte ranges this layer's tree was parsed over, sorted by
    /// `start_byte`. Empty means "the whole buffer" — true only for the root
    /// layer (index 0).
    pub ranges: Vec<tree_sitter::Range>,
    /// Nesting depth: 0 for the root layer, 1+ for each level of injection.
    pub depth: u8,
}

/// A buffer's full syntax tree: the root grammar's tree plus every embedded
/// injection layer, installed atomically per parse.
///
/// `layers[0]` is always the root layer (`ranges` empty, `depth` 0).
#[derive(Default)]
pub struct SyntaxLayers {
    pub layers: Vec<SyntaxLayer>,
}

impl SyntaxLayers {
    /// The root grammar's parse tree, if any layers are installed.
    pub fn root_tree(&self) -> Option<&tree_sitter::Tree> {
        self.layers.first().map(|l| &l.tree)
    }
}

/// True if `layer` was parsed over the byte range `[line_start, line_end)`.
/// The root layer (empty `ranges`) always covers every line.
pub(crate) fn layer_covers_line(layer: &SyntaxLayer, line_start: usize, line_end: usize) -> bool {
    layer.ranges.is_empty()
        || layer
            .ranges
            .iter()
            .any(|r| r.start_byte < line_end && line_start < r.end_byte)
}
