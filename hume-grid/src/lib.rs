//! The frame's cell grid: what HUME draws into, and the diff that turns two
//! consecutive frames into the cells worth repainting.
//!
//! Pure data, testable without a terminal. The terminal half (front/back
//! grids, escape sequences) lives in `hume-platform`. The only HUME
//! dependency is `hume-rope`, for [`Canvas`]'s text measurement.
//!
//! ## One width model
//!
//! A [`Cell`] stores how many columns it advances, measured once by the writer
//! with `hume_rope::width` and never recomputed. A second measurement in the
//! diff or emitter could disagree about a glyph and leave a stale cell, so
//! [`Grid`] takes the advance as an argument and only [`Canvas`] measures.
//!
//! ## Heads and continuations
//!
//! A double-width glyph is a *head* cell holding the text plus a
//! *continuation* with no text, zero advance and the head's style. [`Grid`]'s
//! write primitives (`pub(crate)`, reached from outside only through
//! [`Canvas`]) always write them as a pair, so `diff` can compare cells with
//! `==` and never emit half a glyph.

#![deny(rustdoc::broken_intra_doc_links)]

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
