use std::num::NonZeroU16;

use slotmap::SecondaryMap;

use crate::display_lines::line_store::FormatKey;
use crate::display_lines::{DisplayLineMap, DisplayLinePos};
use crate::pipeline::ContentWidth;
use crate::providers::ProviderSet;
use crate::types::PaintedSelections;
use hume_core::BufferId;
use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;
use ropey::Rope;

// Viewport  (per-pane scroll)

/// The scroll state of one pane's viewport. Its size is a
/// [`PaneSize`](crate::pipeline::PaneSize) read from the screen's layout.
///
/// `top` is written from outside this crate only through the scroll verbs
/// in [`crate::display_lines::scroll`] (`scroll_by`/`reveal`/`align`), so no
/// caller resolves a `DisplayLinePos` independently of them, and through
/// [`Pane::resume_scroll`], which puts back a top those verbs resolved in an
/// earlier run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewport {
    pub(crate) top: DisplayLinePos,
    /// Horizontal scroll in document columns, not terminal cells (0 when
    /// soft-wrap is on). Written outside this crate only through
    /// [`Viewport::reset_horizontal`].
    pub(crate) horizontal_offset: DisplayLineCol,
}

impl Viewport {
    /// The viewport's top display-line address as last stored, unvalidated:
    /// a write since the last [`Viewport::top_at`](crate::display_lines::scroll)
    /// (`show_buffer`, an LSP jump, `Pane::inherit_view_state`) can leave it
    /// stale. A caller holding a `DisplayLineMap` calls `top_at` instead.
    pub fn top(&self) -> DisplayLinePos {
        self.top
    }

    /// The viewport's horizontal scroll offset in document columns.
    pub fn horizontal_offset(&self) -> DisplayLineCol {
        self.horizontal_offset
    }

    /// Zero the horizontal scroll offset on a wrap-mode change, without
    /// waiting a frame for
    /// [`Viewport::reveal_horizontal`](crate::display_lines::scroll).
    pub fn reset_horizontal(&mut self) {
        self.horizontal_offset = DisplayLineCol::new(0);
    }

    /// Seed an arbitrary top, for test fixtures. Not `#[cfg(test)]` because
    /// `hume-editor`'s tests use it too. Production never calls it. An
    /// invalid address heals on the next `top_at` read.
    pub fn seed_top_for_test(&mut self, top: DisplayLinePos) {
        self.top = top;
    }

    /// [`Viewport::seed_top_for_test`]'s counterpart for `horizontal_offset`.
    pub fn seed_horizontal_offset_for_test(&mut self, horizontal_offset: DisplayLineCol) {
        self.horizontal_offset = horizontal_offset;
    }
}

/// A nonzero-height viewport's scroll-margin geometry, resolved once per
/// scroll operation and threaded through every verb in
/// [`crate::display_lines::scroll`]. Minted only by
/// [`PaneSize::geometry`](crate::pipeline::PaneSize::geometry), so `height`
/// is never 0. Cursor-follow (`reveal`) and the scroll-down bound
/// (`max_scroll_top`) read `margin`/`target` from the same value, so a
/// `Ctrl-d` to EOF and the next cursor motion agree on the viewport top.
#[derive(Copy, Clone, Debug)]
pub struct ViewGeometry {
    /// The pane's height in terminal rows. Callers also use it as a
    /// display-line or buffer-line cap; each site picks its unit.
    pub(crate) height: usize,
    /// Display lines of look-ahead kept above/below the cursor, clamped so
    /// the two margins can never meet in the middle of an odd-or-even height.
    pub(crate) margin: usize,
    /// Display line (0-indexed from the top) the far edge settles at once
    /// `margin` is reserved on both sides; always `>= margin`.
    pub(crate) target: usize,
}

impl ViewGeometry {
    /// The pane's height in terminal rows.
    pub fn height(&self) -> usize {
        self.height
    }
}

// Wrap mode

/// How the formatter handles lines that exceed the content width.
///
/// For `Soft`, `Word`, and `Indent`, `width: 0` means "wrap at the content
/// width" (pane width minus gutter). [`WrapMode::resolve`] turns it into the
/// [`ResolvedWrap`] formatting reads.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// No wrapping: horizontal scroll.
    #[default]
    None,
    /// Break at `width` columns (`0` = the content width).
    Soft { width: u16 },
    /// Break at whitespace boundaries; prefer not to split words (`0` = the content width).
    Word { width: u16 },
    /// Word wrap + indent continuation display lines to match the line's indent level (`0` = the content width).
    Indent { width: u16 },
}

/// The wrapping style used when nothing more specific applies:
/// `EditorSettings::wrap_mode`'s default, and what `:wrap` pins when neither
/// a saved pane pin nor the configured style would wrap.
pub const DEFAULT_WRAP_STYLE: WrapMode = WrapMode::Indent { width: 0 };

/// A pane's wrap-mode override for one buffer: the value type of
/// `Pane::wraps`.
///
/// Kept per (pane, buffer) so switching to another buffer resolves through
/// that buffer's own override/global chain, and switching back restores this
/// pane's pin, the same lifetime a saved `Viewport` has in `saved_scrolls`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WrapOverride {
    /// `None` means no override: the effective mode falls back to the
    /// buffer's, then the global default (resolved in `hume-editor`). Stored
    /// unresolved, since content width depends on live pane geometry.
    pub mode: Option<WrapMode>,
    /// The override to restore when `:wrap` turns wrapping back on for this
    /// buffer: `None` returns the pane to inheriting, `Some(m)` restores the
    /// pin `m`. Never `Some(WrapMode::None)`, which would make toggle-on a
    /// no-op.
    pub saved: Option<WrapMode>,
}

impl WrapMode {
    /// Replace a `width: 0` with `content_width`: the only way to get a
    /// [`ResolvedWrap`]. Other modes pass through unchanged.
    pub fn resolve(self, content_width: NonZeroU16) -> ResolvedWrap {
        let content_width = content_width.get();
        ResolvedWrap(match self {
            WrapMode::Soft { width: 0 } => WrapMode::Soft {
                width: content_width,
            },
            WrapMode::Word { width: 0 } => WrapMode::Word {
                width: content_width,
            },
            WrapMode::Indent { width: 0 } => WrapMode::Indent {
                width: content_width,
            },
            other => other,
        })
    }

    pub fn is_wrapping(&self) -> bool {
        !matches!(self, WrapMode::None)
    }
}

/// A [`WrapMode`] whose width is a concrete, non-zero column: what
/// formatting reads. Minted only by [`WrapMode::resolve`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWrap(WrapMode);

impl ResolvedWrap {
    /// Wrap column, or `None` if wrapping is off.
    pub fn wrap_width(self) -> Option<u16> {
        match self.0 {
            WrapMode::None => None,
            WrapMode::Soft { width } | WrapMode::Word { width } | WrapMode::Indent { width } => {
                Some(width)
            }
        }
    }

    pub fn is_wrapping(self) -> bool {
        self.0.is_wrapping()
    }

    /// Whether an overflowing line backtracks to its last whitespace
    /// (`Word`, `Indent`) rather than splitting at the exact wrap column.
    pub fn breaks_at_word(self) -> bool {
        matches!(self.0, WrapMode::Word { .. } | WrapMode::Indent { .. })
    }

    /// Whether continuation display lines are indented to the line's own
    /// indent (`Indent`).
    pub fn indents_continuations(self) -> bool {
        matches!(self.0, WrapMode::Indent { .. })
    }
}

// Whitespace indicators

/// When to render a whitespace indicator for a particular whitespace type.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WhitespaceRender {
    /// Never render an indicator.
    #[default]
    None,
    /// Always render an indicator.
    All,
    /// Only render for trailing whitespace (before end-of-line).
    Trailing,
}

/// Configuration for whitespace indicator rendering.
///
/// Part of `display_lines::line_store`'s format key, so a whitespace change
/// invalidates cached formats.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WhitespaceConfig {
    pub space: WhitespaceRender,
    pub tab: WhitespaceRender,
    /// Whether to render the newline indicator. A newline is always
    /// trailing, so this is on/off only.
    pub newline: bool,
    /// Character to show in place of a space when rendered.
    pub space_char: &'static str,
    /// Character to show at the start of a tab expansion.
    pub tab_char: &'static str,
    /// Character to show in place of a newline when rendered.
    pub newline_char: &'static str,
    /// Character to show in place of an invisible Unicode space (NBSP U+00A0,
    /// ideographic space U+3000) when rendered, so stray ones stand out.
    /// Gated by the `space` render mode.
    pub nbsp_char: &'static str,
}

impl Default for WhitespaceConfig {
    fn default() -> Self {
        Self {
            space: WhitespaceRender::None,
            tab: WhitespaceRender::None,
            newline: false,
            space_char: "·",
            tab_char: "→",
            newline_char: "⏎",
            nbsp_char: "⍽",
        }
    }
}

// Pane

/// A single editor pane: an independent view into a buffer.
pub struct Pane {
    /// Which buffer this pane views. Changed only through
    /// [`EngineView::show_buffer`](crate::pipeline::EngineView::show_buffer).
    pub(crate) buffer_id: BufferId,
    /// Scroll and size state. Outside this crate it is read through
    /// [`Pane::viewport`], [`Pane::viewport_mut`],
    /// [`Pane::display_lines_and_viewport`], [`Pane::scroll_top`],
    /// [`Pane::inherit_view_state`] and
    /// [`EngineView::show_buffer`](crate::pipeline::EngineView::show_buffer),
    /// which a host can ban where a view must first follow its cursor.
    pub(crate) viewport: Viewport,
    /// Where this pane was when it last viewed each buffer.
    pub saved_scrolls: SecondaryMap<BufferId, Viewport>,
    /// All active selections as the renderer paints them. `None` until the
    /// host writes the first frame's mirror.
    pub selections: Option<PaintedSelections>,
    /// Registered providers for this pane.
    pub providers: ProviderSet,
    /// This pane's wrap-mode override, per buffer it has shown. Two panes on
    /// the same buffer may wrap differently. Read and written through
    /// [`Pane::wrap`]/[`Pane::set_wrap`], or [`Pane::wrap_in`]/
    /// [`Pane::resume_wrap`] for a buffer the pane may not show.
    pub wraps: SecondaryMap<BufferId, WrapOverride>,
    /// What every walk of this pane's display lines has learned about the
    /// lines it visited; see [`crate::display_lines::line_store`].
    ///
    /// Per pane because two panes on one buffer at one width resolve the
    /// same [`FormatKey`], and a shared hit would skip the other pane's
    /// providers. Every pane's scroll
    /// step also runs before any pane renders, so one shared store would hold
    /// only the last pane's lines.
    pub line_store: crate::display_lines::line_store::PaneLineStore,
}

impl Pane {
    /// Create a new pane viewing `buffer_id`, with no wrap-mode override.
    pub fn new(buffer_id: BufferId) -> Self {
        Self {
            buffer_id,
            viewport: Viewport::default(),
            saved_scrolls: SecondaryMap::new(),
            selections: None,
            providers: ProviderSet::new(),
            wraps: SecondaryMap::new(),
            line_store: crate::display_lines::line_store::PaneLineStore::new(),
        }
    }

    /// Copy the view state a same-buffer split inherits from its source pane.
    ///
    /// The exhaustive destructure makes a new `Pane` field a compile error
    /// here until it is inherited or skipped.
    pub fn inherit_view_state(&mut self, src: &Pane) {
        let Pane {
            buffer_id: _, // pane identity, set by the split itself
            viewport,
            saved_scrolls,
            selections: _, // engine render copy, repopulated every frame
            providers: _,  // allocated per pane in build_pane
            wraps,
            line_store: _, // per-frame scratch, rewound every frame
        } = src;
        self.viewport = *viewport;
        self.saved_scrolls = saved_scrolls.clone();
        self.wraps = wraps.clone();
    }

    /// This pane's wrap-mode override for the buffer it currently views,
    /// `WrapOverride::default()` if it has never pinned or toggled one.
    pub fn wrap(&self) -> WrapOverride {
        self.wraps.get(self.buffer_id).copied().unwrap_or_default()
    }

    /// Write this pane's wrap-mode override for the buffer it currently views.
    pub fn set_wrap(&mut self, wrap: WrapOverride) {
        self.wraps.insert(self.buffer_id, wrap);
    }

    /// The buffer this pane shows.
    pub fn buffer_id(&self) -> BufferId {
        self.buffer_id
    }

    /// This pane's viewport as it stands.
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// This pane's viewport as it stands, to write.
    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    /// The display lines of `rope` in this pane, laid out by `key` at
    /// `content_width`: what every scroll, cursor and movement consumer walks
    /// instead of walking display lines itself.
    pub fn display_lines<'a>(
        &'a mut self,
        rope: &'a Rope,
        key: FormatKey,
        content_width: ContentWidth,
    ) -> DisplayLineMap<'a> {
        DisplayLineMap::new(
            rope,
            &self.providers,
            content_width,
            key,
            &mut self.line_store,
        )
    }

    /// [`Self::display_lines`] together with the viewport, for a caller that
    /// writes the viewport while it reads the map.
    pub fn display_lines_and_viewport<'a>(
        &'a mut self,
        rope: &'a Rope,
        key: FormatKey,
        content_width: ContentWidth,
    ) -> (DisplayLineMap<'a>, &'a mut Viewport) {
        let dlm = DisplayLineMap::new(
            rope,
            &self.providers,
            content_width,
            key,
            &mut self.line_store,
        );
        (dlm, &mut self.viewport)
    }

    /// Show `target`, saving the scroll of the buffer shown until now and
    /// restoring `target`'s (the top on a first visit).
    ///
    /// `top`'s line is clamped to `last_content_line`, `target`'s current
    /// last line, since the buffer may have shrunk since its scroll was
    /// saved. `top`'s slot is restored unvalidated; the next
    /// `Viewport::top_at` read heals a stale one.
    pub(crate) fn show_buffer(&mut self, target: BufferId, last_content_line: ContentLine) {
        self.saved_scrolls.insert(self.buffer_id, self.viewport);
        self.buffer_id = target;
        let saved = self.saved_scrolls.get(target).copied().unwrap_or_default();
        self.viewport = Viewport {
            top: DisplayLinePos::new(saved.top.line.min(last_content_line), saved.top.slot),
            ..saved
        };
    }

    /// Scroll back to `top` in `bid`, a top this pane had for it in an
    /// earlier run: at once for the buffer it shows, on the next
    /// [`EngineView::show_buffer`](crate::pipeline::EngineView::show_buffer)
    /// for any other. Like that restore, the slot is
    /// unvalidated and the next `Viewport::top_at` read heals a stale one.
    pub fn resume_scroll(&mut self, bid: BufferId, top: DisplayLinePos) {
        let viewport = Viewport {
            top,
            ..Viewport::default()
        };
        if bid == self.buffer_id {
            self.viewport = viewport;
        } else {
            self.saved_scrolls.insert(bid, viewport);
        }
    }

    /// This pane's top display line in `bid`: the live one for the buffer it
    /// shows, the saved one for any other, the start when it has none.
    pub fn scroll_top(&self, bid: BufferId) -> DisplayLinePos {
        if bid == self.buffer_id {
            self.viewport.top
        } else {
            self.saved_scrolls
                .get(bid)
                .map(|viewport| viewport.top)
                .unwrap_or_default()
        }
    }

    /// This pane's wrap-mode override for `bid`, `None` if it has never
    /// pinned or toggled one there.
    pub fn wrap_in(&self, bid: BufferId) -> Option<WrapOverride> {
        self.wraps.get(bid).copied()
    }

    /// Put back `wrap` as this pane's override for `bid`, one it had there in
    /// an earlier run.
    pub fn resume_wrap(&mut self, bid: BufferId, wrap: WrapOverride) {
        self.wraps.insert(bid, wrap);
    }

    /// Drop the saved scroll and wrap-mode override entries for `id` (called
    /// when the buffer is closed).
    pub fn forget_buffer(&mut self, id: BufferId) {
        self.saved_scrolls.remove(id);
        self.wraps.remove(id);
    }
}

/// Line index of the primary selection head.
///
/// Takes the two fields rather than `&Pane` so the render pass can call it
/// while holding `&mut` on the pane's line store.
pub fn primary_head_line(selections: &PaintedSelections, rope: &Rope) -> ContentLine {
    let head = selections.primary().cursor;
    debug_assert!(
        head.offset().index() < rope.len_chars(),
        "stale selection mirror: head {head:?} beyond rope len {}: \
         pane.selections is out of sync with pane.buffer_id",
        rope.len_chars()
    );
    // A cluster start lies below the text end, on a real content line.
    ContentLine::new(hume_rope::lines::char_to_ropey_line(rope, head.offset()).index())
}

#[cfg(test)]
mod tests;
