use std::sync::Arc;

use crate::registry::GrammarBundle;

/// One parsed layer of a buffer's syntax tree: the root grammar, or one
/// embedded-language injection (a fenced code block, a combined
/// `markdown.inline` layer, etc.).
pub struct SyntaxLayer {
    pub tree: tree_sitter::Tree,
    /// The layer's language bundle — `Arc`'d from the `LanguageRegistry`,
    /// not owned per-buffer. Carries the whole bundle, not just its
    /// highlighter: any per-language query a layer may later need
    /// (`locals.scm`) comes for free, and it's what makes an injected
    /// layer's `textobjects` query reachable at all — a highlighter-only
    /// field left it unreachable.
    pub bundle: Arc<GrammarBundle>,
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
    if layer.ranges.is_empty() {
        return true;
    }
    // `ranges` is sorted by start and non-overlapping, so end bytes ascend
    // too — the only candidate for intersection is the last range starting
    // before `line_end`. Binary search keeps this O(log n) per line even for
    // combined layers with one range per paragraph (markdown.inline).
    let idx = layer.ranges.partition_point(|r| r.start_byte < line_end);
    idx > 0 && line_start < layer.ranges[idx - 1].end_byte
}

#[cfg(test)]
mod tests;
