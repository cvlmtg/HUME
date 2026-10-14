//! Tree-sitter indentation: the width a new line gets from the language's
//! `indents.scm` and the buffer's syntax tree. `docs/TS-INDENT.md` states
//! the model this implements.

mod query;
mod walk;

pub use query::IndentQuery;

use hume_editing::indent::{BreakIndent, ElectricTrigger};
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_rope::cluster::ClusterStart;
use hume_rope::column::BufferLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{ExclusiveRange, InclusiveRange};

use crate::layers::{SyntaxLayer, SyntaxLayers, range_holding};
use walk::{ElectricInput, Levels, Outcome, Target};

/// The indent of the line a break at `split` starts, from the innermost
/// syntax layer holding `split` whose grammar has an indents query, merged
/// outward into each enclosing layer holding it of the same language. An
/// injected layer's widths start from its base: the indent of its first
/// line.
///
/// The width is relative to a baseline: the first of up to 4 non-blank
/// lines from `split`'s line upward that the same layer models with the
/// same alignment. The new line gets that line's width moved by the
/// difference in levels, so it follows the file's own indentation. With
/// no such baseline, the model's absolute width is used. `after_token`
/// tells whether the cluster before `split` is a token of that layer's
/// tree, its text being its node kind.
///
/// `None` when no layer holding `split` has an indents query, or `split`
/// is inside an opaque node (a multi-line string) begun on an earlier line:
/// the caller keeps its own indent.
///
/// # Panics
/// If `layers` reach past `text`'s end: they were not parsed from `text`.
pub fn indent_for_new_line(
    layers: &SyntaxLayers,
    text: &BufferText,
    split: ClusterStart,
    tab_width: u8,
) -> Option<BreakIndent> {
    let target = Target::NewLine(text.char_to_byte(split.offset()));
    let model = model(layers, text, target, tab_width)?;
    let split_line = text.char_to_line(split.offset());
    let baselines = InclusiveRange::new(ContentLine::new(0), split_line)
        .iter()
        .rev();
    Some(BreakIndent {
        width: hybrid(layers, text, &model, baselines, tab_width)?,
        after_token: after_token(&layers.layers[model.layer], text, split),
    })
}

/// The width the model gives the existing line `line` when it starts with
/// a complete outdent token, the cursor at `cursor` and `trigger` the input
/// landing there: a token the layer's query captures as an outdent that
/// the input cannot extend. After a typed character, a token reaching the
/// cursor is complete only when it does not end in a word character, as
/// the buffer's `word_chars` classify it, and is not a proper prefix of
/// another of the grammar's tokens, so `else` waits while `elsewhere` could
/// still follow and `.` while `..` could. A line break completes every
/// token ending at or before the cursor and none reaching past it, since
/// the break carries that token's rest to the next line. The width is the
/// hybrid one [`indent_for_new_line`] computes, with baselines from the
/// line above `line` upward. The caller applies it only when it is
/// smaller than the line's current indent.
///
/// `None` for a blank line, a line whose first token is not a complete
/// outdent, or a line no layer's indents query answers for.
///
/// # Panics
/// If `layers` reach past `text`'s end: they were not parsed from `text`.
pub fn electric_indent(
    layers: &SyntaxLayers,
    text: &BufferText,
    line: ContentLine,
    cursor: ClusterStart,
    trigger: ElectricTrigger,
    word_chars: WordChars<'_>,
    tab_width: u8,
) -> Option<BufferLineCol> {
    if text.lines().is_blank(line) {
        return None;
    }
    let at = text.char_to_byte(text.lines().indent_end(line).offset());
    let indenting = indent_layer(layers, at)?;
    let input = ElectricInput {
        cursor: text.char_to_byte(cursor.offset()),
        trigger,
        word_chars,
    };
    if !walk::starts_with_outdent(indenting.query, indenting.layer, text, at, input, tab_width) {
        return None;
    }
    let model = indenting.model(text, Target::Line(at), tab_width)?;
    let baselines = InclusiveRange::new(ContentLine::new(0), line)
        .iter()
        .rev()
        .skip(1);
    hybrid(layers, text, &model, baselines, tab_width)
}

/// The most non-blank baselines `hybrid` tries.
const BASELINE_ATTEMPTS: usize = 4;

/// The width `model` gives its target, relative to the first of
/// `baselines` (nearest first) that the same layer models with the same
/// alignment; the absolute width when none of the first
/// [`BASELINE_ATTEMPTS`] non-blank ones does. `None` when the target is
/// opaque.
fn hybrid(
    layers: &SyntaxLayers,
    text: &BufferText,
    model: &Model,
    baselines: impl Iterator<Item = ContentLine>,
    tab_width: u8,
) -> Option<BufferLineCol> {
    let levels = model.outcome.levels()?;
    let width = baselines
        .filter(|&line| !text.lines().is_blank(line))
        .take(BASELINE_ATTEMPTS)
        .find_map(|line| {
            let baseline = line_model(layers, text, line, tab_width)?;
            let baseline_levels = baseline.outcome.levels()?;
            (baseline.layer == model.layer && baseline_levels.align == levels.align).then(|| {
                let (_, width) = text.columns().indent(line, tab_width);
                shifted(width, levels.net - baseline_levels.net, tab_width)
            })
        })
        .unwrap_or_else(|| absolute(levels, model.base, tab_width));
    Some(width)
}

/// Whether the cluster before `split` is a token of its own in `layer`'s
/// tree: a leaf covering that cluster and nothing more, whose kind is the
/// cluster's text. False at the buffer start.
fn after_token(layer: &SyntaxLayer, text: &BufferText, split: ClusterStart) -> bool {
    let Some(before) = text.clusters().prev(split.into()) else {
        return false;
    };
    let bytes = text.char_to_byte(before.offset())..text.char_to_byte(split.offset());
    layer
        .tree
        .root_node()
        .descendant_for_byte_range(bytes.start, bytes.end)
        .is_some_and(|node| {
            node.child_count() == 0
                && node.byte_range() == bytes
                && text.slice(ExclusiveRange::new(before.offset(), split.offset())) == node.kind()
        })
}

/// What the model says about a target, and where it said it.
struct Model {
    /// The index of the layer the answer comes from.
    layer: usize,
    /// The width the layer's column 0 sits at.
    base: BufferLineCol,
    outcome: Outcome,
}

/// The model's answer for `target`, from the layer [`indent_layer`] picks
/// for the target's byte.
fn model(layers: &SyntaxLayers, text: &BufferText, target: Target, tab_width: u8) -> Option<Model> {
    indent_layer(layers, target.byte())?.model(text, target, tab_width)
}

/// The layer whose tree indents a byte, with its index among the layers
/// and its indents query.
#[derive(Clone, Copy)]
struct IndentLayer<'l> {
    index: usize,
    layer: &'l SyntaxLayer,
    query: &'l IndentQuery,
}

impl IndentLayer<'_> {
    /// This layer's answer for `target`, whose byte it indents.
    fn model(self, text: &BufferText, target: Target, tab_width: u8) -> Option<Model> {
        Some(Model {
            layer: self.index,
            base: base(self.layer, text, target.byte(), tab_width),
            outcome: walk::compute(self.query, self.layer, text, target, tab_width)?,
        })
    }
}

/// The layer whose tree indents byte `at`: the innermost layer holding
/// `at` whose grammar has an indents query, merged outward while the next
/// layer holding it at a lower depth (an enclosing layer, never a sibling
/// at the same depth) is of the same language and has an indents query.
fn indent_layer(layers: &SyntaxLayers, at: usize) -> Option<IndentLayer<'_>> {
    let mut containing = layers.layers_containing(at..at);
    let (mut index, mut layer, mut query) = containing
        .find_map(|(index, layer)| Some((index, layer, layer.bundle.indents.as_ref()?)))?;
    for (outer_index, outer) in containing {
        if outer.depth >= layer.depth {
            continue;
        }
        let Some(outer_query) = outer
            .bundle
            .indents
            .as_ref()
            .filter(|_| outer.bundle.language == layer.bundle.language)
        else {
            break;
        };
        (index, layer, query) = (outer_index, outer, outer_query);
    }
    Some(IndentLayer {
        index,
        layer,
        query,
    })
}

/// The width column 0 of `layer`'s tree sits at around byte `at`: 0 for
/// the root layer. For an injected layer, the width of the first line its
/// range holding `at` covers from that line's first non-blank character:
/// the range's own first line when the range starts at or before that
/// character, else the line after it while still inside the range. 0 when
/// there is no such line.
fn base(layer: &SyntaxLayer, text: &BufferText, at: usize, tab_width: u8) -> BufferLineCol {
    let Some(range) = range_holding(layer, &(at..at)) else {
        return BufferLineCol::new(0);
    };
    let start = text.byte_to_char(range.start_byte);
    let first = text.char_to_line(start);
    let base_line = if text.lines().indent_end(first).offset() >= start {
        Some(first)
    } else {
        let next = first.advance(1);
        (next <= text.last_content_line()
            && text.char_to_byte(text.lines().start(next).offset()) < range.end_byte)
            .then_some(next)
    };
    base_line.map_or(BufferLineCol::new(0), |line| {
        text.columns().indent(line, tab_width).1
    })
}

/// The model's answer for the existing line `line`, measured from its
/// first non-blank character. `None` for a blank line.
fn line_model(
    layers: &SyntaxLayers,
    text: &BufferText,
    line: ContentLine,
    tab_width: u8,
) -> Option<Model> {
    if text.lines().is_blank(line) {
        return None;
    }
    let indent_end = text.lines().indent_end(line);
    let target = Target::Line(text.char_to_byte(indent_end.offset()));
    model(layers, text, target, tab_width)
}

/// `width` moved by `net` levels, stopping at 0.
fn shifted(width: BufferLineCol, net: i32, tab_width: u8) -> BufferLineCol {
    let step = hume_rope::width::indent_stop(net.unsigned_abs(), tab_width);
    if net < 0 {
        width.retreat_saturating(step)
    } else {
        width.advance_saturating(step)
    }
}

/// The width `levels` give on their own: from the alignment column, or
/// from the layer's `base`. An alignment column already counts from the
/// buffer line's start, so the base is never added to it.
fn absolute(levels: &Levels, base: BufferLineCol, tab_width: u8) -> BufferLineCol {
    shifted(levels.align.unwrap_or(base), levels.net, tab_width)
}

/// The absolute width of the line a break at `split` starts.
#[cfg(test)]
fn absolute_for_new_line(
    layers: &SyntaxLayers,
    text: &BufferText,
    split: ClusterStart,
    tab_width: u8,
) -> Option<BufferLineCol> {
    let target = Target::NewLine(text.char_to_byte(split.offset()));
    let model = model(layers, text, target, tab_width)?;
    Some(absolute(model.outcome.levels()?, model.base, tab_width))
}

/// The absolute width the model gives the existing line `line`.
#[cfg(test)]
fn absolute_for_line(
    layers: &SyntaxLayers,
    text: &BufferText,
    line: ContentLine,
    tab_width: u8,
) -> Option<BufferLineCol> {
    let model = line_model(layers, text, line, tab_width)?;
    Some(absolute(model.outcome.levels()?, model.base, tab_width))
}

/// Whether `line`'s first non-blank character is inside an opaque node
/// begun on an earlier line.
#[cfg(test)]
fn is_opaque_interior(
    layers: &SyntaxLayers,
    text: &BufferText,
    line: ContentLine,
    tab_width: u8,
) -> bool {
    line_model(layers, text, line, tab_width)
        .is_some_and(|model| matches!(model.outcome, Outcome::Opaque))
}

#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod tests;
