//! The rendering engine: buffer text, selections and provider output in,
//! a frame of cells out.
//!
//! [`pipeline::EngineView`] holds the pane layout, the theme and the scope
//! registry, and [`pipeline::EngineView::render`] draws one frame into a
//! `hume_grid::Grid`. Each [`pane::Pane`] owns its viewport, wrap mode and
//! per-frame display-line cache. [`display_lines`] is the one mapping from
//! buffer lines to display lines (soft wrap and virtual lines) that
//! rendering, scrolling, cursor placement and mouse hits all read, and
//! [`format`](mod@format) lays one buffer line out into graphemes.
//!
//! What the editor adds beyond text arrives through the traits in
//! [`providers`]: syntax spans, gutter columns, decorations, overlays and
//! chrome bands. The crate knows nothing of files, keymaps or scripting; it
//! depends only on `hume-core`, `hume-rope` and `hume-grid`.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod builtins;
pub mod display_lines;
pub mod format;
pub mod interval_sweep;
pub mod layout;
pub mod lock;
pub mod pane;
pub mod pipeline;
pub mod providers;
pub mod render;
pub mod scope_headers;
pub mod style;
#[cfg(any(test, feature = "test-util"))]
pub mod test_doubles;
#[cfg(test)]
mod test_support;
pub mod theme;
pub mod types;
