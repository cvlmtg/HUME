use std::cmp::Reverse;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use crate::registry::{GrammarBundle, LanguageId};
use crate::textobjects::{ObjectSpans, SpanSelector};

/// One parsed layer of a buffer's syntax tree: the root grammar, or one
/// embedded-language injection (a fenced code block, a combined
/// `markdown.inline` layer, etc.).
pub struct SyntaxLayer {
    pub tree: tree_sitter::Tree,
    /// The layer's language bundle, `Arc`'d from the `LanguageRegistry`,
    /// not owned per-buffer. Carries the whole bundle, not just its
    /// highlighter: any per-language query a layer may later need
    /// (`locals.scm`) comes for free, and it's what makes an injected
    /// layer's `textobjects` query reachable at all; a highlighter-only
    /// field left it unreachable.
    pub bundle: Arc<GrammarBundle>,
    /// Absolute byte ranges this layer's tree was parsed over, sorted by
    /// `start_byte`. Empty means "the whole buffer", true only for the root
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
    /// `Syntax::install` replaces this whole struct, so it loses the memo for
    /// free; `Syntax::bake` is the only code
    /// that mutates layers in place, and it clears this explicitly. A
    /// `tree_gen` key could not do the same job: `bake` advances `tree_gen`
    /// to a generation and a later `install` for that *same* generation
    /// replaces the layers under an unchanged value.
    ///
    /// `Mutex` for the reason `Syntax::span_scratch` is one: the collection
    /// entry point takes `&SyntaxLayers`, so the editor's dispatch path needs
    /// no `&mut` and no signature change to reach it.
    ///
    /// Single-entry: the access pattern is the same structural command
    /// pressed again (key repeat, a macro or `.`-repeat step), not an
    /// alternating working set.
    pub(crate) textobject_memo: Mutex<Option<(SpanSelector, Arc<ObjectSpans>)>>,
}

impl SyntaxLayers {
    /// Wrap freshly parsed layers, memo empty. The only constructor:
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

    /// The languages of the layers holding all of `bytes` inside one of
    /// their included ranges, innermost first. A span that crosses a gap
    /// between two of a layer's ranges is not in that layer: part of it is
    /// another language's text. Of two layers at one depth, the later one
    /// comes first, as it does in highlighting.
    pub fn languages_containing(&self, bytes: Range<usize>) -> Vec<LanguageId> {
        let mut containing: Vec<(usize, &SyntaxLayer)> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer_contains(layer, &bytes))
            .collect();
        containing.sort_by_key(|&(index, layer)| Reverse((layer.depth, index)));
        containing
            .into_iter()
            .map(|(_, layer)| layer.bundle.language)
            .collect()
    }

    /// Drop the text-object memo. Called from `Syntax::bake`, the only path
    /// that edits these layers without replacing them (see the field's doc).
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
    layer.ranges.is_empty()
        || last_range_starting_before(layer, line_end).is_some_and(|r| line_start < r.end_byte)
}

/// True if one of `layer`'s included ranges holds all of `bytes`. The root
/// layer (empty `ranges`) holds every span.
fn layer_contains(layer: &SyntaxLayer, bytes: &Range<usize>) -> bool {
    layer.ranges.is_empty()
        || last_range_starting_before(layer, bytes.start + 1)
            .is_some_and(|r| bytes.end <= r.end_byte)
}

/// The last of `layer`'s ranges starting before byte `end`. `ranges` is
/// sorted by start and non-overlapping, so end bytes ascend too, and it is
/// the only range that can reach a byte before `end`. Binary search keeps
/// this O(log n) for combined layers with one range per paragraph
/// (markdown.inline).
fn last_range_starting_before(layer: &SyntaxLayer, end: usize) -> Option<&tree_sitter::Range> {
    let idx = layer.ranges.partition_point(|r| r.start_byte < end);
    idx.checked_sub(1).and_then(|i| layer.ranges.get(i))
}

#[cfg(test)]
mod tests;
