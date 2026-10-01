//! `:sort`: permute whole lines, keyed by the selected text on each line.
//!
//! Unlike Helix's `:sort` (which permutes *text between selection slots* and
//! leaves line boundaries untouched) this permutes the lines themselves, keyed
//! by whatever text a selection covers on them, closer to `sort -k`. Also
//! rejects Kakoune's `|sort` (pipes each selection through the shell), since
//! that makes N one-line selections an N-way no-op: there's nothing for a
//! per-line shell invocation to reorder against.

use hume_editing::edit::Edited;
use hume_editing::edit::{Landing, Landings, edit};
use hume_editing::lines::{line_content_range, line_start};
use hume_editing::selection::EditView;
use hume_editing::state::EditState;
use hume_rope::line::ContentLine;
use hume_rope::lines::line_token_content;
use unicode_normalization::UnicodeNormalization;

/// Flags accepted by `:sort`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SortOpts {
    pub reverse: bool,
    pub insensitive: bool,
}

/// Why [`sort_lines`] declined to produce an edit.
///
/// A distinct type rather than `Edited::unchanged`: the caller (`:sort`'s
/// typed-command handler) reports *why* nothing happened ("nothing to
/// sort" vs. "already sorted"), which an unchanged edit alone can't tell
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortRefusal {
    /// No selection spans two or more line-adjacent lines.
    NoAdjacentLines,
    /// Every contiguous group of lines is already in order.
    AlreadySorted,
}

/// One buffer line touched by a selection, keyed by the selected text on it.
struct SortEntry {
    line: ContentLine,
    key: String,
}

/// Sort each maximal run of line-adjacent entries touched by a selection,
/// keyed by the selected text on that line. Groups sort independently: text
/// never moves between groups.
pub fn sort_lines(state: EditState, opts: SortOpts) -> Result<Edited, SortRefusal> {
    let view = state.view();
    let text = state.text();
    let entries = collect_entries(&view);
    let groups = group_adjacent(&entries);

    let mut any_group = false;
    // Each moved line, and the content that lands on it.
    let mut moves: Vec<(ContentLine, String)> = Vec::new();
    // Old line -> new line, only for entries that move.
    let mut line_map = rustc_hash::FxHashMap::<ContentLine, ContentLine>::default();

    for group in &groups {
        if group.len() < 2 {
            continue;
        }
        any_group = true;

        let order = sort_order(&entries, group, opts);
        let inv = invert(&order);
        for (local, &new_local) in inv.iter().enumerate() {
            if new_local != local {
                line_map.insert(entries[group[local]].line, entries[group[new_local]].line);
            }
        }

        // Each moved slot's content is replaced by the content of the line
        // that lands there; every line keeps its own '\n'.
        for (slot, &local) in order.iter().enumerate() {
            if slot == local {
                continue;
            }
            let target = entries[group[slot]].line;
            let source = entries[group[local]].line;
            let content = line_token_content(text.rope().line(source.index()));
            moves.push((target, content));
        }
    }

    if !any_group {
        return Err(SortRefusal::NoAdjacentLines);
    }
    if moves.is_empty() {
        return Err(SortRefusal::AlreadySorted);
    }

    let primary = view.primary().index();
    Ok(edit(&state, |b| {
        for (target, content) in &moves {
            match line_content_range(text, *target) {
                Some(range) => b.replace(range, content),
                None => b.insert(line_start(text, *target), content),
            };
        }
        // A selection on one moved line follows the line to its new place. A
        // selection over several lines keeps each end's line and column, over
        // whatever content moved beneath it.
        let landings = view
            .iter()
            .map(|sel| {
                let lines = sel.lines();
                let (first, last) = match line_map.get(&lines.start) {
                    Some(&moved) if lines.start == lines.end => (moved, moved),
                    _ => (lines.start, lines.end),
                };
                Landing::at_lines(sel, first, last)
            })
            .collect();
        Landings::new(landings, primary)
    }))
}

/// Walk every selection and build one [`SortEntry`] per distinct line it
/// touches, keyed by the selected text on that line (excluding the trailing
/// `\n`). A line touched by two selections gets a compound key and never
/// discards one.
///
/// Entries come out sorted ascending and deduplicated: selections are
/// visited in document order (ascending, non-overlapping), and each one
/// walks its own lines in order.
fn collect_entries(view: &EditView<'_>) -> Vec<SortEntry> {
    let text = view.text();
    let mut entries: Vec<SortEntry> = Vec::new();
    for sel in view.iter() {
        for span in sel.line_spans() {
            let fragment = span
                .content
                .map(|range| text.slice(range.chars()).to_string())
                .unwrap_or_default();
            match entries.last_mut() {
                Some(last) if last.line == span.line => last.key.push_str(&fragment),
                _ => entries.push(SortEntry {
                    line: span.line,
                    key: fragment,
                }),
            }
        }
    }
    entries
}

/// Split `entries` (ascending, unique lines) into maximal runs of consecutive
/// line numbers. Each inner `Vec` holds indices into `entries`.
fn group_adjacent(entries: &[SortEntry]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut prev_line: Option<ContentLine> = None;
    for (idx, entry) in entries.iter().enumerate() {
        match (groups.last_mut(), prev_line) {
            (Some(g), Some(prev)) if prev.advance(1) == entry.line => g.push(idx),
            _ => groups.push(vec![idx]),
        }
        prev_line = Some(entry.line);
    }
    groups
}

/// A group's keys, classified once so every comparison in the sort reuses the
/// same parse, not re-parsed per comparison.
enum Keys {
    Int(Vec<i64>),
    /// `f64`, guaranteed finite: `"nan"`/`"inf"` parse but aren't order-total,
    /// so they fall through to `Text` instead.
    Float(Vec<f64>),
    Text(Vec<String>),
}

fn classify_keys(entries: &[SortEntry], group: &[usize], insensitive: bool) -> Keys {
    let raw: Vec<&str> = group.iter().map(|&i| entries[i].key.as_str()).collect();
    if let Some(ints) = raw
        .iter()
        .map(|s| s.trim().parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()
    {
        return Keys::Int(ints);
    }
    if let Some(floats) = raw
        .iter()
        .map(|s| s.trim().parse::<f64>().ok().filter(|f| f.is_finite()))
        .collect::<Option<Vec<_>>>()
    {
        return Keys::Float(floats);
    }
    let texts = raw
        .iter()
        .map(|s| {
            if insensitive {
                s.to_lowercase().nfc().collect()
            } else {
                s.nfc().collect()
            }
        })
        .collect();
    Keys::Text(texts)
}

/// The permutation for one group: `order[slot]` is the group-local index of
/// the entry that ends up at `slot`. Stable: equal keys keep document order,
/// including under `-r` (the comparator is flipped, not the result vector, so
/// ties are never reversed).
fn sort_order(entries: &[SortEntry], group: &[usize], opts: SortOpts) -> Vec<usize> {
    let keys = classify_keys(entries, group, opts.insensitive);
    let mut order: Vec<usize> = (0..group.len()).collect();
    order.sort_by(|&a, &b| {
        let ord = match &keys {
            Keys::Int(v) => v[a].cmp(&v[b]),
            Keys::Float(v) => v[a].total_cmp(&v[b]),
            Keys::Text(v) => v[a].cmp(&v[b]),
        };
        if opts.reverse { ord.reverse() } else { ord }
    });
    order
}

/// Invert a permutation: `inv[i]` is the slot that group-local entry `i` ends
/// up in (the slot `j` such that `order[j] == i`).
fn invert(order: &[usize]) -> Vec<usize> {
    let mut inv = vec![0; order.len()];
    for (slot, &local) in order.iter().enumerate() {
        inv[local] = slot;
    }
    inv
}
