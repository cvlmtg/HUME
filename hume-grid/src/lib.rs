//! The frame's cell grid: what HUME draws into, and the diff that turns two
//! consecutive frames into the cells worth repainting.
//!
//! Pure data — no terminal, no I/O, no other HUME crate. The half that talks
//! to a terminal (owning a front/back pair of grids, turning the diff into
//! escape sequences) lives in `hume-platform`, which is where the platform
//! dependency belongs; keeping the grid itself free of it is what lets every
//! invariant below be tested without a terminal.
//!
//! ## One width model
//!
//! A [`Cell`] stores the number of columns it advances the cursor, measured
//! by whoever wrote the text and never recomputed downstream. HUME measures
//! text with `hume_rope::width`; a diff or an emitter that measured a second
//! time would be a second model, free to disagree with the first about the
//! same glyph — and disagreement there means a stale cell on screen, not a
//! compile error. [`Grid`] takes the advance as an argument for exactly this
//! reason: this crate never measures text, it only records what it was told.
//!
//! ## Heads and continuations
//!
//! A double-width glyph occupies two cells: a *head* holding the text, and a
//! *continuation* holding no text, advancing no columns, and carrying the
//! head's style. [`Grid`]'s write primitives guarantee the two always come in
//! well-formed pairs — no continuation without its head, no head without its
//! continuations — which is what lets [`diff`] compare cells one at a time
//! with `==` and still never emit half a glyph. See [`Grid::set_glyph`].

mod cell;
mod color;
pub mod diff;
mod geometry;
mod grid;
mod style;

pub use cell::Cell;
pub use color::Rgb;
pub use diff::{DiffRuns, RowRun};
pub use geometry::{Position, Rect};
pub use grid::Grid;
pub use style::{Modifiers, ResolvedStyle, UnderlineStyle};
