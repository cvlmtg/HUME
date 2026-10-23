use std::cmp::Reverse;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use crate::edited_bytes::EditedBytes;
use crate::injections::{InjectionSites, PriorInjection};
use crate::parse_worker::PriorLayers;
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
    /// The matches of the layer's injections query over its tree, moved
    /// with the tree by every edit since the parse. Empty at the nesting
    /// cap and for a grammar without an injections query.
    pub sites: InjectionSites,
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
    /// free; [`Self::apply_edits`] is the only code that mutates layers in
    /// place, and it clears this explicitly. A generation key could not do
    /// the same job: `bake` moves the layers to a generation and a later
    /// `install` for that *same* generation replaces them under an unchanged
    /// value.
    ///
    /// `Mutex` for the reason `Syntax::span_scratch` is one: the collection
    /// entry point takes `&SyntaxLayers`, so the editor's dispatch path needs
    /// no `&mut` and no signature change to reach it.
    ///
    /// Single-entry: the access pattern is the same structural command
    /// pressed again (key repeat, a macro or `.`-repeat step), not an
    /// alternating working set.
    pub(crate) textobject_memo: Mutex<Option<(SpanSelector, Arc<ObjectSpans>)>>,
    /// The bytes the edits applied since these layers were parsed may have
    /// changed. Placed here for the memo's reason: `Syntax::install`
    /// replaces this struct, which empties it, and [`Self::apply_edits`] is
    /// the only code that changes it.
    edited: EditedBytes,
}

impl SyntaxLayers {
    /// Wrap freshly parsed layers, memo and edited bytes empty. The only
    /// constructor: `textobject_memo` is private so that nothing outside
    /// this module can hand out a set of layers carrying someone else's
    /// cached spans.
    pub fn new(layers: Vec<SyntaxLayer>) -> Self {
        Self {
            layers,
            textobject_memo: Mutex::new(None),
            edited: EditedBytes::default(),
        }
    }

    /// Apply `edits`, in the order `Tree::edit` takes them, to every
    /// layer's tree and sites, move each injected layer's `ranges` with its
    /// tree, and record the edited bytes. Drops the text-object memo, whose
    /// spans were collected at the pre-edit positions.
    pub(crate) fn apply_edits<'e>(
        &mut self,
        edits: impl IntoIterator<Item = &'e tree_sitter::InputEdit>,
    ) {
        for edit in edits {
            for layer in &mut self.layers {
                layer.tree.edit(edit);
                layer.sites.edit(edit);
            }
            self.edited.record(edit);
        }
        for layer in &mut self.layers {
            // `ranges` is a cached copy of the tree's included ranges,
            // consulted by `layer_covers_line`. The root's are always empty
            // (whole buffer).
            if layer.depth > 0 {
                layer.ranges = layer.tree.included_ranges();
            }
        }
        self.clear_textobject_memo();
    }

    /// These layers as the prior layers of a parse of the text they are
    /// edited to.
    ///
    /// # Panics
    /// If there are no layers: `layers[0]` is the root.
    pub(crate) fn prior(&self) -> PriorLayers {
        let (root, injected) = self.layers.split_first().expect("layers[0] is the root");
        PriorLayers {
            root: root.tree.clone(),
            root_sites: root.sites.clone(),
            injected: injected
                .iter()
                .map(|layer| PriorInjection {
                    bundle: Arc::clone(&layer.bundle),
                    depth: layer.depth,
                    tree: layer.tree.clone(),
                    ranges: layer.ranges.clone(),
                    sites: layer.sites.clone(),
                })
                .collect(),
            edited: self.edited.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn edited(&self) -> &EditedBytes {
        &self.edited
    }

    /// The root grammar's parse tree, if any layers are installed.
    #[cfg(any(test, feature = "test-util"))]
    pub fn root_tree(&self) -> Option<&tree_sitter::Tree> {
        self.layers.first().map(|l| &l.tree)
    }

    /// The languages of the layers holding all of `bytes`, innermost first,
    /// in the order `layers_containing` gives.
    pub fn languages_containing(&self, bytes: Range<usize>) -> Vec<LanguageId> {
        self.layers_containing(bytes)
            .map(|(_, layer)| layer.bundle.language)
            .collect()
    }

    /// The layers holding all of `bytes` inside one of their included
    /// ranges, with their indices, innermost first. A span that crosses a
    /// gap between two of a layer's ranges is not in that layer: part of it
    /// is another language's text. Of two layers at one depth, the later
    /// one comes first, as it does in highlighting.
    pub(crate) fn layers_containing(
        &self,
        bytes: Range<usize>,
    ) -> impl Iterator<Item = (usize, &SyntaxLayer)> {
        let mut containing: Vec<(usize, &SyntaxLayer)> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer_contains(layer, &bytes))
            .collect();
        containing.sort_by_key(|&(index, layer)| Reverse((layer.depth, index)));
        containing.into_iter()
    }

    fn clear_textobject_memo(&mut self) {
        *self
            .textobject_memo
            .lock()
            .expect("textobject memo lock poisoned") = None;
    }

    /// Every layer's shape, in layer order.
    #[cfg(any(test, feature = "test-util"))]
    pub fn shapes(&self) -> Vec<LayerShape> {
        self.layers
            .iter()
            .map(|layer| LayerShape::of(&layer.tree, &layer.bundle, layer.depth, &layer.ranges))
            .collect()
    }
}

impl SyntaxLayer {
    /// The sites a fresh resolution finds in this layer's tree, and the
    /// shapes of the layers it injects, their own injections included, in
    /// the order a parse installs them.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn injections_resolved_afresh(
        &self,
        rope: &ropey::Rope,
        langs: &crate::registry::GrammarSnapshot,
    ) -> (InjectionSites, Vec<LayerShape>) {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let cx = crate::injections::InjectionContext {
            rope,
            langs,
            prior: None,
            cancel: &cancel,
        };
        let (sites, injected) = crate::injections::resolve_and_parse_injections(
            &mut tree_sitter::Parser::new(),
            &cx,
            &self.tree,
            &self.bundle,
            self.depth + 1,
            None,
            &mut Vec::new(),
        );
        let shapes = injected
            .iter()
            .map(|layer| LayerShape::of(&layer.tree, &layer.bundle, layer.depth, &layer.ranges))
            .collect();
        (sites, shapes)
    }
}

/// A layer in comparable form: its grammar, place and every node of its
/// tree. Two layers of equal shape hold the same syntax over the same text.
#[cfg(any(test, feature = "test-util"))]
#[derive(Clone)]
pub struct LayerShape {
    pub bundle: Arc<GrammarBundle>,
    pub depth: u8,
    pub ranges: Vec<tree_sitter::Range>,
    /// Every node in pre-order: its kind, whether the parser inserted it as
    /// missing, and its range.
    pub nodes: Vec<(String, bool, tree_sitter::Range)>,
}

#[cfg(any(test, feature = "test-util"))]
impl LayerShape {
    pub(crate) fn of(
        tree: &tree_sitter::Tree,
        bundle: &Arc<GrammarBundle>,
        depth: u8,
        ranges: &[tree_sitter::Range],
    ) -> Self {
        let mut nodes = Vec::new();
        let mut cursor = tree.walk();
        'walk: loop {
            let node = cursor.node();
            nodes.push((node.kind().to_owned(), node.is_missing(), node.range()));
            if cursor.goto_first_child() || cursor.goto_next_sibling() {
                continue;
            }
            while cursor.goto_parent() {
                if cursor.goto_next_sibling() {
                    continue 'walk;
                }
            }
            break;
        }
        Self {
            bundle: Arc::clone(bundle),
            depth,
            ranges: ranges.to_vec(),
            nodes,
        }
    }
}

/// Where two lists of layer shapes first differ.
#[cfg(any(test, feature = "test-util"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerMismatch {
    pub index: usize,
    pub field: LayerField,
}

/// The part of a [`LayerShape`] a [`LayerMismatch`] found different.
#[cfg(any(test, feature = "test-util"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerField {
    /// Not the same grammar bundle, by identity.
    Bundle,
    Depth,
    Ranges,
    Nodes,
    /// The layer exists in one list only.
    Absent,
}

/// The first index at which `expected` and `actual` differ, and the first
/// field, in declaration order, that differs there.
#[cfg(any(test, feature = "test-util"))]
pub fn first_layer_mismatch(
    expected: &[LayerShape],
    actual: &[LayerShape],
) -> Option<LayerMismatch> {
    let differing_field = |expected: &LayerShape, actual: &LayerShape| {
        if !Arc::ptr_eq(&expected.bundle, &actual.bundle) {
            Some(LayerField::Bundle)
        } else if expected.depth != actual.depth {
            Some(LayerField::Depth)
        } else if expected.ranges != actual.ranges {
            Some(LayerField::Ranges)
        } else if expected.nodes != actual.nodes {
            Some(LayerField::Nodes)
        } else {
            None
        }
    };
    expected
        .iter()
        .zip(actual)
        .enumerate()
        .find_map(|(index, (expected, actual))| {
            differing_field(expected, actual).map(|field| LayerMismatch { index, field })
        })
        .or_else(|| {
            (expected.len() != actual.len()).then(|| LayerMismatch {
                index: expected.len().min(actual.len()),
                field: LayerField::Absent,
            })
        })
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
    layer.ranges.is_empty() || range_holding(layer, bytes).is_some()
}

/// The one of `layer`'s included ranges holding all of `bytes`. `None` for
/// the root layer, which has no ranges, and for bytes the layer does not
/// hold.
pub(crate) fn range_holding<'l>(
    layer: &'l SyntaxLayer,
    bytes: &Range<usize>,
) -> Option<&'l tree_sitter::Range> {
    last_range_starting_before(layer, bytes.start + 1).filter(|r| bytes.end <= r.end_byte)
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
