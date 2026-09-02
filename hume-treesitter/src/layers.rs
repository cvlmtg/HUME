use std::sync::{Arc, Mutex};

use crate::registry::GrammarBundle;
use crate::textobjects::{ObjectSpans, SpanSelector};

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
    /// Memo for [`crate::textobjects::ObjectSpans::for_selector`]: the last
    /// selector collected from *these* layers, and its result.
    ///
    /// It lives here, rather than on `Syntax` keyed by a generation, because
    /// that placement is what makes invalidation total instead of careful.
    /// `Syntax::install` replaces this whole struct and `clear_layers` drops
    /// it, so both lose the memo for free; `Syntax::bake` is the only code
    /// that mutates layers in place, and it clears this explicitly. A
    /// `tree_gen` key could not do the same job: `bake` advances `tree_gen`
    /// to a generation and a later `install` for that *same* generation
    /// replaces the layers under an unchanged value.
    ///
    /// `Mutex` for the reason `Syntax::span_scratch` is one — the collection
    /// entry point takes `&SyntaxLayers`, so the editor's dispatch path needs
    /// no `&mut` and no signature change to reach it.
    ///
    /// Single-entry: the access pattern is the same structural command
    /// pressed again (key repeat, a macro or `.`-repeat step), not an
    /// alternating working set.
    pub(crate) textobject_memo: Mutex<Option<(SpanSelector, Arc<ObjectSpans>)>>,
}

impl SyntaxLayers {
    /// Wrap freshly parsed layers, memo empty. The only constructor —
    /// `textobject_memo` is private so that nothing outside this module can
    /// hand out a set of layers carrying someone else's cached spans.
    pub fn new(layers: Vec<SyntaxLayer>) -> Self {
        Self {
            layers,
            textobject_memo: Mutex::new(None),
        }
    }

    /// The root grammar's parse tree, if any layers are installed.
    pub fn root_tree(&self) -> Option<&tree_sitter::Tree> {
        self.layers.first().map(|l| &l.tree)
    }

    /// Drop the text-object memo. Called from `Syntax::bake`, the only path
    /// that edits these layers without replacing them — see the field's doc.
    pub(crate) fn clear_textobject_memo(&mut self) {
        *self
            .textobject_memo
            .lock()
            .expect("textobject memo lock poisoned") = None;
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
