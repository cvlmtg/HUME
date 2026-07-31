//! `:sort` — permute whole rows, keyed by the selected text on each row.
//!
//! Unlike Helix's `:sort` (which permutes *text between selection slots* and
//! leaves row boundaries untouched) this permutes the rows themselves, keyed
//! by whatever text a selection covers on them — closer to `sort -k`. See
//! `docs/ROADMAP.md` for the full design rationale.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::lines::line_end_exclusive;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

/// Flags accepted by `:sort`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SortOpts {
    pub reverse: bool,
    pub insensitive: bool,
}

/// Why [`sort_rows`] declined to produce an edit.
///
/// A distinct type (not an identity `ChangeSet`) is load-bearing: `Buffer::apply_edit`
/// records an undo revision unconditionally, so an identity edit would still push a
/// revision and mark a clean buffer dirty. Returning `Err` lets the caller skip
/// applying anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortRefusal {
    /// No selection spans two or more line-adjacent rows.
    NoAdjacentRows,
    /// Every contiguous group of rows is already in order.
    AlreadySorted,
}

/// One buffer row touched by a selection, keyed by the selected text on it.
struct Row {
    line: usize,
    key: String,
}

/// Sort each maximal run of line-adjacent rows touched by a selection, keyed by
/// the selected text on that row. Groups sort independently — text never moves
/// between groups.
pub fn sort_rows(
    buf: Text,
    sels: SelectionSet,
    opts: SortOpts,
) -> Result<(Text, SelectionSet, ChangeSet), SortRefusal> {
    let rows = collect_rows(&buf, &sels);
    let groups = group_adjacent(&rows);

    let mut b = ChangeSetBuilder::new(buf.len_chars());
    let mut any_group = false;
    let mut any_edit = false;
    // Old line -> new line, populated only for rows that actually move.
    let mut line_map = rustc_hash::FxHashMap::<usize, usize>::default();

    for group in &groups {
        if group.len() < 2 {
            continue;
        }
        any_group = true;

        let order = sort_order(&rows, group, opts);
        let inv = invert(&order);
        for (local, &new_local) in inv.iter().enumerate() {
            if new_local != local {
                line_map.insert(rows[group[local]].line, rows[group[new_local]].line);
            }
        }

        let Some((lo, hi)) = trimmed_window(&order) else {
            continue; // fully identity — nothing to write for this group
        };
        any_edit = true;

        let edit_start = buf.line_to_char(rows[group[lo]].line);
        let edit_end = line_end_exclusive(&buf, rows[group[hi]].line);
        b.retain(edit_start - b.old_pos());
        b.delete(edit_end - edit_start);
        for &local in &order[lo..=hi] {
            let line = rows[group[local]].line;
            let start = buf.line_to_char(line);
            let end = line_end_exclusive(&buf, line);
            b.insert(&buf.slice(start..end).to_string());
        }
    }

    if !any_group {
        return Err(SortRefusal::NoAdjacentRows);
    }
    if !any_edit {
        return Err(SortRefusal::AlreadySorted);
    }

    b.retain_rest();
    let cs = b.finish();
    let new_buf = cs
        .apply(&buf)
        .expect("sort produced an invalid changeset — this is a bug");

    let new_sels = remap_selections(&buf, &new_buf, &sels, &line_map);
    new_sels.debug_assert_valid(&new_buf);
    Ok((new_buf, new_sels, cs))
}

/// Walk every selection and build one [`Row`] per distinct line it touches,
/// keyed by the selected text on that line (excluding the trailing `\n`). A
/// line touched by two selections gets a compound key — never discards one.
///
/// Rows come out sorted ascending and deduplicated by construction: selections
/// are visited via `iter_sorted()` (ascending, non-overlapping), and each one
/// walks its own lines in order.
fn collect_rows(buf: &Text, sels: &SelectionSet) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for sel in sels.iter_sorted() {
        let start_line = buf.char_to_line(sel.start());
        let end_line = buf.char_to_line(sel.end_inclusive(buf));
        for line in start_line..=end_line {
            let line_start = buf.line_to_char(line);
            // This line's own trailing '\n'. Every line reachable from a
            // selection is a real content line (< len_lines() - 1) — the
            // structural final line can never host a selection, since `head`
            // is always `< len_chars()` and the buffer's last char is the
            // trailing '\n' belonging to the second-to-last line.
            let nl = line_end_exclusive(buf, line) - 1;
            let fragment = if nl > line_start {
                // On the selection's own start/end line, clamp to the part of
                // the line actually selected; on lines in between (a
                // multi-line span), the whole line's content qualifies.
                let seg_start = sel.start().max(line_start);
                let seg_end_incl = sel.end_inclusive(buf).min(nl - 1);
                if seg_start <= seg_end_incl {
                    buf.slice(seg_start..seg_end_incl + 1).to_string()
                } else {
                    String::new()
                }
            } else {
                String::new() // blank line: no content to key on
            };
            match rows.last_mut() {
                Some(last) if last.line == line => last.key.push_str(&fragment),
                _ => rows.push(Row {
                    line,
                    key: fragment,
                }),
            }
        }
    }
    rows
}

/// Split `rows` (ascending, unique lines) into maximal runs of consecutive
/// line numbers. Each inner `Vec` holds indices into `rows`.
fn group_adjacent(rows: &[Row]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut prev_line: Option<usize> = None;
    for (idx, row) in rows.iter().enumerate() {
        match (groups.last_mut(), prev_line) {
            (Some(g), Some(prev)) if prev + 1 == row.line => g.push(idx),
            _ => groups.push(vec![idx]),
        }
        prev_line = Some(row.line);
    }
    groups
}

/// A group's keys, classified once so every comparison in the sort reuses the
/// same parse — not re-parsed per comparison.
enum Keys {
    Int(Vec<i64>),
    /// `f64`, guaranteed finite — `"nan"`/`"inf"` parse but aren't order-total,
    /// so they fall through to `Text` instead.
    Float(Vec<f64>),
    Text(Vec<String>),
}

fn classify_keys(rows: &[Row], group: &[usize], insensitive: bool) -> Keys {
    let raw: Vec<&str> = group.iter().map(|&i| rows[i].key.as_str()).collect();
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
                s.to_lowercase()
            } else {
                s.to_string()
            }
        })
        .collect();
    Keys::Text(texts)
}

/// The permutation for one group: `order[slot]` is the group-local index of
/// the row that ends up at `slot`. Stable — equal keys keep document order,
/// including under `-r` (the comparator is flipped, not the result vector, so
/// ties are never reversed).
fn sort_order(rows: &[Row], group: &[usize], opts: SortOpts) -> Vec<usize> {
    let keys = classify_keys(rows, group, opts.insensitive);
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

/// Invert a permutation: `inv[i]` is the slot that group-local row `i` ends up
/// in (the slot `j` such that `order[j] == i`).
fn invert(order: &[usize]) -> Vec<usize> {
    let mut inv = vec![0; order.len()];
    for (slot, &local) in order.iter().enumerate() {
        inv[local] = slot;
    }
    inv
}

/// The smallest `[lo, hi]` window covering every slot that actually moved
/// (`order[slot] != slot`), or `None` if the group is already in order.
///
/// Because `order` is a bijection on `0..n` and everything outside `[lo, hi]`
/// is a fixed point by construction, `order[lo..=hi]` is necessarily a
/// permutation of `lo..=hi` itself — the window never needs to reach outside
/// itself for a value.
fn trimmed_window(order: &[usize]) -> Option<(usize, usize)> {
    let moved = |(slot, &local): (usize, &usize)| local != slot;
    let lo = order.iter().enumerate().position(moved)?;
    let hi = order.iter().enumerate().rposition(moved)?;
    Some((lo, hi))
}

/// Selections follow their row: a selection confined to a single moved row is
/// shifted by the same column offset onto the row's new home. A selection
/// spanning multiple rows keeps its char range unchanged — the group's total
/// length is invariant under a row permutation (rows move verbatim), so the
/// range still points at valid text, just reordered underneath it.
fn remap_selections(
    old_buf: &Text,
    new_buf: &Text,
    sels: &SelectionSet,
    line_map: &rustc_hash::FxHashMap<usize, usize>,
) -> SelectionSet {
    let mut new_sels = Vec::with_capacity(sels.len());
    for sel in sels.iter_sorted() {
        let start_line = old_buf.char_to_line(sel.start());
        let end_line = old_buf.char_to_line(sel.end_inclusive(old_buf));
        let moved = if start_line == end_line {
            line_map.get(&start_line).map(|&new_line| {
                let old_line_start = old_buf.line_to_char(start_line);
                let anchor_col = sel.anchor() - old_line_start;
                let head_col = sel.head() - old_line_start;
                let new_line_start = new_buf.line_to_char(new_line);
                Selection::new(new_line_start + anchor_col, new_line_start + head_col)
            })
        } else {
            None
        };
        new_sels.push(moved.unwrap_or(*sel));
    }
    SelectionSet::from_vec(new_sels, sels.primary_index())
}
