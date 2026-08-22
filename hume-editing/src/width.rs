//! Thin delegations to `hume_rope::width`'s tab-stop arithmetic, so crates
//! that don't depend on `hume-rope` directly (`hume-ops`) can still reach it.
//! Unlike this module's siblings (`grapheme.rs`, `lines.rs`), these take no
//! `&Text` — the underlying functions are pure `usize`/`u8` scalar math with
//! no rope to adapt. See `hume_rope::width` for the implementations and
//! detailed doc comments.

/// See [`hume_rope::width::tab_advance`].
pub fn tab_advance(display_col: usize, tab_width: u8) -> usize {
    hume_rope::width::tab_advance(display_col, tab_width)
}

/// See [`hume_rope::width::prev_tab_stop`].
pub fn prev_tab_stop(display_col: usize, tab_width: u8) -> usize {
    hume_rope::width::prev_tab_stop(display_col, tab_width)
}
