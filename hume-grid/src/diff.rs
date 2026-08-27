use crate::cell::Cell;
use crate::grid::Grid;

/// A horizontal run of cells to repaint: `cells` belong at `x..` on row `y`.
///
/// A run may extend past the last *changed* cell of a wide glyph — it carries
/// the head, and the head's own width covers the rest — so an emitter must
/// advance the cursor by each cell's [`Cell::advance`], not by the length of
/// this slice.
pub struct RowRun<'a> {
    pub y: u16,
    pub x: u16,
    pub cells: &'a [Cell],
}

/// Runs of cells that differ between two grids — see [`Grid::diff_runs`].
///
/// Plain double-buffer diffing: compare cell by cell, emit what changed.
/// There is no damage tracking above this (every frame is composed from
/// scratch), so this is the only thing standing between a repaint of one
/// character and a repaint of the whole screen.
///
/// ## Gap merging
///
/// Two changed cells separated by a few unchanged ones are emitted as a
/// single run, re-printing the cells between them. Repositioning the cursor
/// costs a ~6–9 byte CUP sequence plus a parse; re-printing an unchanged cell
/// usually costs one to four bytes and no style change at all, since it
/// shares its neighbours' style. Merging under a small threshold is therefore
/// cheaper in bytes *and* in sequences, and it keeps a styled run contiguous
/// so the emitter's SGR state survives across it. The threshold is the
/// caller's (`max_gap`); `hume-platform` owns the value.
///
/// Re-printing an unchanged cell is safe because it is idempotent: the cell
/// is emitted with its own style, so the terminal ends up with exactly what
/// it already had.
pub struct DiffRuns<'a> {
    next: &'a Grid,
    prev: &'a Grid,
    max_gap: u16,
    /// Row being scanned, and the column to resume at within it.
    y: u16,
    x: u16,
}

impl<'a> DiffRuns<'a> {
    pub(crate) fn new(next: &'a Grid, prev: &'a Grid, max_gap: u16) -> DiffRuns<'a> {
        debug_assert_eq!(
            next.size(),
            prev.size(),
            "diffing grids of different sizes — the caller must resize both together"
        );
        DiffRuns {
            next,
            prev,
            max_gap,
            y: 0,
            x: 0,
        }
    }
}

impl<'a> Iterator for DiffRuns<'a> {
    type Item = RowRun<'a>;

    fn next(&mut self) -> Option<RowRun<'a>> {
        let (width, height) = self.next.size();
        while self.y < height {
            let next_row = self.next.row(self.y);
            let prev_row = self.prev.row(self.y);
            let changed = |i: u16| next_row[i as usize] != prev_row[i as usize];

            let Some(start) = (self.x..width).find(|&i| changed(i)) else {
                self.y += 1;
                self.x = 0;
                continue;
            };

            // Absorb the next change while no more than `max_gap` unchanged
            // cells separate it from the one before — see this type's doc.
            let mut end = start + 1;
            while let Some(k) = {
                let limit = end
                    .saturating_add(self.max_gap)
                    .saturating_add(1)
                    .min(width);
                (end..limit).find(|&i| changed(i))
            } {
                end = k + 1;
            }

            self.x = end;
            debug_assert!(
                !next_row[start as usize].is_continuation(),
                "run at ({start}, {}) starts on a continuation — a changed \
                 continuation must always be preceded by its changed head",
                self.y
            );
            return Some(RowRun {
                y: self.y,
                x: start,
                cells: &next_row[start as usize..end as usize],
            });
        }
        None
    }
}

#[cfg(test)]
mod tests;
