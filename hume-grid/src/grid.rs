use crate::cell::Cell;
use crate::diff::DiffRuns;
use crate::geometry::Rect;
use crate::style::ResolvedStyle;

/// A rectangular grid of [`Cell`]s — one frame's worth of terminal content.
///
/// Origin-free: a grid is `width` × `height` addressed from `(0, 0)`. Screen
/// regions are described by [`Rect`], which carries an origin, but every grid
/// in HUME covers the whole terminal, so giving the storage an offset of its
/// own would add an addressing mode nothing uses.
///
/// ## The write invariant
///
/// [`Grid::set_glyph`] and [`Grid::fill_span`] are the only ways to change a
/// cell, and both maintain: **every continuation has its head immediately
/// reachable to its left, and every head is followed by exactly as many
/// continuations as it advances columns.** Overwriting half of a
/// double-width glyph demotes the other half to a blank rather than leaving
/// it orphaned.
///
/// That invariant is what buys the diff its simplicity. Because a
/// continuation carries its head's style, a continuation differs between two
/// frames only when its head does too — so a run of changed cells can never
/// begin at a continuation, and repainting a run can never start in the
/// middle of a glyph. Callers used to blank the trailing column of a wide
/// glyph by hand at each write site; enforcing it here instead means a new
/// write site cannot forget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    width: u16,
    height: u16,
    /// Row-major, `width * height` long. Kept private so the invariant above
    /// has no bypass.
    cells: Vec<Cell>,
}

impl Grid {
    pub fn new(width: u16, height: u16) -> Grid {
        Grid {
            width,
            height,
            cells: vec![Cell::default(); width as usize * height as usize],
        }
    }

    /// The whole grid as a rect at the origin.
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Resize, discarding all content.
    ///
    /// The caller repaints from scratch after a resize rather than preserving
    /// what fits: a terminal reflows its own content on resize in ways this
    /// side cannot predict, so anything kept would be a guess about what the
    /// terminal already shows.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.cells.clear();
        self.cells
            .resize(width as usize * height as usize, Cell::default());
    }

    /// Blank every cell, keeping the allocation. Run between frames — HUME
    /// composes each frame from scratch.
    pub fn reset(&mut self) {
        self.cells.fill(Cell::default());
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        (x < self.width && y < self.height).then(|| &self.cells[self.index(x, y)])
    }

    /// One row's cells, left to right. Empty if `y` is out of bounds.
    pub fn row(&self, y: u16) -> &[Cell] {
        if y >= self.height {
            return &[];
        }
        let start = self.index(0, y);
        &self.cells[start..start + self.width as usize]
    }

    /// Runs of cells that differ from `prev` — see [`crate::diff`].
    pub fn diff_runs<'a>(&'a self, prev: &'a Grid, max_gap: u16) -> DiffRuns<'a> {
        DiffRuns::new(self, prev, max_gap)
    }

    fn index(&self, x: u16, y: u16) -> usize {
        y as usize * self.width as usize + x as usize
    }

    /// Draw `text` at `(x, y)`, occupying `advance` columns.
    ///
    /// Writes the head plus its continuations, and repairs both edges so the
    /// row keeps this type's write invariant. A glyph that would run past the
    /// right edge is written as blanks instead — never split, matching the
    /// rule text measurement follows everywhere else in HUME. Out-of-bounds
    /// coordinates are ignored.
    pub fn set_glyph(&mut self, x: u16, y: u16, text: &str, advance: u8, style: ResolvedStyle) {
        if x >= self.width || y >= self.height {
            return;
        }
        let advance = advance.max(1);
        let span = advance as u16;

        // Clipped: the glyph is dropped whole and its columns blanked, never
        // split across the edge — the rule `hume_rope::width` truncation
        // follows, so a field measured to fit and a field drawn to fit agree.
        if x.saturating_add(span) > self.width {
            let len = self.width - x;
            self.repair_edges(x, y, len);
            let row = self.index(0, y);
            self.cells[row + x as usize..row + self.width as usize].fill(Cell::blank(style));
            return;
        }

        self.repair_edges(x, y, span);
        let row = self.index(0, y);
        self.cells[row + x as usize] = Cell::glyph(text, advance, style);
        for k in 1..span {
            self.cells[row + (x + k) as usize] = Cell::continuation(style);
        }
    }

    /// Fill `x_start..x_end` on row `y` with clones of `cell`.
    ///
    /// The bulk primitive behind background fills and row clears: one slice
    /// write instead of N bounds-checked cell writes. Repairs both edges the
    /// same way [`Grid::set_glyph`] does, so filling over half a wide glyph
    /// cannot orphan the other half.
    pub fn fill_span(&mut self, y: u16, x_start: u16, x_end: u16, cell: Cell) {
        debug_assert!(
            cell.advance() == 1,
            "fill_span writes one column per cell; a wide glyph needs set_glyph"
        );
        if y >= self.height {
            return;
        }
        let x_start = x_start.min(self.width);
        let x_end = x_end.min(self.width);
        if x_start >= x_end {
            return;
        }
        self.repair_edges(x_start, y, x_end - x_start);
        let row = self.index(0, y);
        self.cells[row + x_start as usize..row + x_end as usize].fill(cell);
    }

    /// Blank whichever glyph a write to `x..x + len` on row `y` would leave
    /// half-overwritten, so the row still satisfies this type's write
    /// invariant afterwards.
    ///
    /// Both repairs read the row as it stands *before* the write and touch
    /// only cells outside the span, so the caller can then write over the
    /// span itself without re-checking anything. A demoted half keeps the
    /// style it was painted in rather than taking the incoming one: it is
    /// leftover background, not part of what is being drawn.
    fn repair_edges(&mut self, x: u16, y: u16, len: u16) {
        let row = self.index(0, y);

        // Left: a continuation at `x` means its head sits before the span.
        if self.cells[row + x as usize].is_continuation() {
            let mut head = x;
            while head > 0 && self.cells[row + head as usize].is_continuation() {
                head -= 1;
            }
            for cx in head..x {
                let style = self.cells[row + cx as usize].style();
                self.cells[row + cx as usize] = Cell::blank(style);
            }
        }

        // Right: a continuation just past the span is about to lose its head
        // to the write, whether that head is inside the span or was itself
        // already orphaned on the left.
        let mut cx = x.saturating_add(len);
        while cx < self.width && self.cells[row + cx as usize].is_continuation() {
            let style = self.cells[row + cx as usize].style();
            self.cells[row + cx as usize] = Cell::blank(style);
            cx += 1;
        }
    }
}

impl std::ops::Index<(u16, u16)> for Grid {
    type Output = Cell;

    fn index(&self, (x, y): (u16, u16)) -> &Cell {
        self.cell(x, y).unwrap_or_else(|| {
            panic!(
                "({x}, {y}) out of bounds for {}x{}",
                self.width, self.height
            )
        })
    }
}

#[cfg(test)]
mod tests;
