/// A cell coordinate on the terminal grid.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Position {
    pub x: u16,
    pub y: u16,
}

impl Position {
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

/// A rectangular region of the terminal grid, in cells.
///
/// Every accessor and combinator here saturates rather than wrapping or
/// panicking. Pane geometry is derived by arithmetic on `u16`s from whatever
/// size the terminal happens to be, including sizes too small for the panes
/// asked of them, so "degenerate input produces an empty rect" has to be the
/// defined behaviour rather than a caller obligation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Leftmost column in the rect.
    pub const fn left(self) -> u16 {
        self.x
    }

    /// One column past the rightmost — the exclusive end, and so also the
    /// `right_edge` bound a text write clips against.
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// Topmost row in the rect.
    pub const fn top(self) -> u16 {
        self.y
    }

    /// One row past the bottom — the exclusive end.
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Cell count. Widened to `u32` because a full `u16` × `u16` rect
    /// overflows `u16`, and the answer is usually compared against zero or
    /// summed rather than stored back into a coordinate.
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Whether `pos` falls inside the rect. Half-open on both axes, matching
    /// [`Rect::right`]/[`Rect::bottom`].
    pub const fn contains(self, pos: Position) -> bool {
        pos.x >= self.x && pos.x < self.right() && pos.y >= self.y && pos.y < self.bottom()
    }

    /// Shrink by `dx` columns on each side and `dy` rows on each side.
    ///
    /// The inside of a bordered box, and the one place that arithmetic is
    /// written: done by hand it is four saturating operations that have to
    /// agree with each other, and a box whose border is thicker than the box
    /// is exactly where hand-written versions produce a rect with a negative
    /// width that wrapped to enormous.
    pub const fn inset(self, dx: u16, dy: u16) -> Rect {
        Rect {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
            width: self.width.saturating_sub(dx.saturating_mul(2)),
            height: self.height.saturating_sub(dy.saturating_mul(2)),
        }
    }

    /// A `width` × `height` rect centred inside `self`, clamped to fit.
    ///
    /// Biased up and left when the leftover space is odd, which is what the
    /// integer division falls out to and what centred chrome has always done.
    pub fn centered(self, width: u16, height: u16) -> Rect {
        let width = width.min(self.width);
        let height = height.min(self.height);
        Rect {
            x: self.x + (self.width - width) / 2,
            y: self.y + (self.height - height) / 2,
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests;
