//! The frame's cell grid: what HUME draws into, and the diff that turns two
//! consecutive frames into the cells worth repainting.
//!
//! Pure data and one text-measuring dependency — no terminal, no I/O, no
//! other HUME crate but `hume-rope`. The half that talks to a terminal
//! (owning a front/back pair of grids, turning the diff into escape
//! sequences) lives in `hume-platform`, which is where the platform
//! dependency belongs; keeping the grid itself free of it is what lets every
//! invariant below be tested without a terminal. `hume-rope` is the one
//! exception: [`Canvas`] is the frame's single text writer (see its own
//! doc), and a writer that measures text needs the crate that defines what
//! "measures" means here — see the width model below.
//!
//! ## One width model
//!
//! A [`Cell`] stores the number of columns it advances the cursor, measured
//! by whoever wrote the text and never recomputed downstream. HUME measures
//! text with `hume_rope::width`; a diff or an emitter that measured a second
//! time would be a second model, free to disagree with the first about the
//! same glyph — and disagreement there means a stale cell on screen, not a
//! compile error. [`Grid`] takes the advance as an argument for exactly this
//! reason: this crate never measures text itself, it only records what
//! [`Canvas`] — the one writer built directly on `hume_rope::width` — told it.
//!
//! ## Heads and continuations
//!
//! A double-width glyph occupies two cells: a *head* holding the text, and a
//! *continuation* holding no text, advancing no columns, and carrying the
//! head's style. [`Grid`]'s write primitives guarantee the two always come in
//! well-formed pairs — no continuation without its head, no head without its
//! continuations — which is what lets `diff` compare cells one at a time
//! with `==` and still never emit half a glyph. See `Grid::set_glyph`
//! (`pub(crate)` — reached from outside this crate only through [`Canvas`]).

pub mod box_glyphs;
mod canvas;
mod cell;
mod color;
mod diff;
mod geometry;
mod grid;
mod style;

pub use canvas::{Canvas, clamp_rect_to_grid};
pub use cell::Cell;
pub use color::Rgb;
pub use diff::{DiffRuns, RowRun};
pub use geometry::{Position, Rect};
pub use grid::Grid;
pub use style::{Modifiers, ResolvedStyle, UnderlineStyle};
