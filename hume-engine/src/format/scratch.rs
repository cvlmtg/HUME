//! The per-line formatting buffers ([`LineFormat`], [`VirtualFormat`]) and
//! the scan bound ([`FormatBound`]) that
//! [`format_buffer_line`](super::format_buffer_line) reads and writes.

use std::ops::Range;

use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::lines::LineText;
use hume_rope::offset::ExclusiveRange;

use crate::types::{DisplayLine, Grapheme, ScopeId};

/// One buffer line's formatted display lines.
///
/// Every index inside is line-local: `DisplayLine::graphemes` indexes
/// `graphemes`, `Grapheme::byte_range` indexes `line_text` from 0, and
/// `CellContent`'s arena `(start, len)` pairs index `virtual_texts`, so one
/// can be handed between passes without rebasing anything.
///
/// Owned by a [`crate::display_lines::line_store::PaneLineStore`], which
/// reuses its allocations across frames.
pub struct LineFormat {
    /// `DisplayLine`s produced for this buffer line.
    pub display_lines: Vec<DisplayLine>,
    /// `Grapheme`s for this line; display lines index into this.
    pub graphemes: Vec<Grapheme>,
    /// Pre-materialised text for this line, read back as
    /// `RenderDisplayLine::line_text`.
    pub line_text: LineText,
    /// Arena backing this line's `CellContent::Virtual` (inline inserts) and
    /// `Whitespace` (indicator glyphs) text ranges, none of which can be
    /// `&'static str` (LSP hints, Steel-configured icons). `TabFill` needs no
    /// arena entry: its text is always a single space.
    pub virtual_texts: String,
    /// How much of the line the buffers above cover, or `None` when nothing
    /// has been formatted into them yet (only the virtual display lines and
    /// block shape are known).
    pub extent: Option<FormatBound>,
    /// The horizontal clip this format was cut to, if any: `DisplayLineMap`'s
    /// `h_window` when it ran. A windowed format drops leading graphemes, so
    /// it answers a different query from an unclipped one. Kept here rather
    /// than in `ResolvedFormatKey` so a window change keeps the entry's
    /// window-independent fields (block shape, virtual display lines).
    pub h_window: Option<Range<DisplayLineCol>>,
}

/// How large each buffer may stay across a frame boundary. Past this,
/// [`LineFormat::reset_and_shrink`] reclaims it down to this size.
///
/// Ceilings, not starting sizes. They sit far above an ordinary source line
/// to catch only the pathological case (a minified-JS line megabytes wide)
/// that would otherwise pin that capacity for the pane's whole life.
const DISPLAY_LINES_CEILING: usize = 256;
const GRAPHEMES_CEILING: usize = 8192;
const LINE_TEXT_CEILING: usize = 8192;
const VIRTUAL_TEXTS_CEILING: usize = 4096;

impl LineFormat {
    /// Empty, with nothing allocated yet.
    ///
    /// One exists per buffer line a pass walks, not per line it formats, and
    /// under `WrapMode::None` most never fill, so nothing is reserved up front.
    pub fn new() -> Self {
        Self {
            display_lines: Vec::new(),
            graphemes: Vec::new(),
            line_text: LineText::new(),
            virtual_texts: String::new(),
            extent: None,
            h_window: None,
        }
    }

    /// Empty every buffer, retaining allocated capacity, ready to be
    /// formatted into again.
    pub fn reset(&mut self) {
        self.display_lines.clear();
        self.graphemes.clear();
        self.line_text.clear();
        self.virtual_texts.clear();
        self.extent = None;
        self.h_window = None;
    }

    /// [`Self::reset`] plus reclaiming any buffer grown past its ceiling, for
    /// the frame boundary, where the next user may be a different line.
    /// `reset` alone serves a reformat of the same line, where shrinking
    /// would only force a re-grow.
    ///
    /// Resetting first matters: [`shrink_to`](Vec::shrink_to) never drops
    /// capacity below the current length.
    pub fn reset_and_shrink(&mut self) {
        self.reset();
        self.shrink_oversized();
    }

    /// Shrink any buffer that has grown past its ceiling back down to it.
    fn shrink_oversized(&mut self) {
        // `Vec` and `String` share no trait carrying `capacity`/`shrink_to`.
        macro_rules! shrink {
            ($buf:expr, $ceiling:expr) => {
                if $buf.capacity() > $ceiling {
                    $buf.shrink_to($ceiling);
                }
            };
        }
        shrink!(self.display_lines, DISPLAY_LINES_CEILING);
        shrink!(self.graphemes, GRAPHEMES_CEILING);
        shrink!(self.line_text, LINE_TEXT_CEILING);
        shrink!(self.virtual_texts, VIRTUAL_TEXTS_CEILING);
    }

    /// Whether this format already answers a query bounded by `bound`, cut to
    /// the same `h_window` the querying map is using.
    pub fn covers(&self, bound: FormatBound, h_window: Option<&Range<DisplayLineCol>>) -> bool {
        self.extent.is_some_and(|e| e.covers(bound)) && self.h_window.as_ref() == h_window
    }
}

impl Default for LineFormat {
    fn default() -> Self {
        Self::new()
    }
}

/// One buffer line's laid-out virtual display lines: every display line
/// its provider virtual lines occupy, `Before`-anchored ones first.
///
/// Separate from `LineFormat` because a `Before` virtual display line
/// renders ahead of content display lines that may already be formatted and
/// cached; laying it out over them would force a reformat.
pub struct VirtualFormat {
    /// The laid-out display lines; each indexes `graphemes`.
    pub display_lines: Vec<DisplayLine>,
    /// How many of `display_lines` are `Before`-anchored: the order
    /// [`hume_core::Side`] imposes puts them first, so the `i`th `After` one
    /// is at index `before + i`.
    pub before: usize,
    /// Each display line's own background scope, parallel to `display_lines`:
    /// every display line a virtual line wraps into carries that virtual
    /// line's `base_scope`.
    pub base_scopes: Vec<Option<ScopeId>>,
    /// Graphemes for `display_lines`.
    pub graphemes: Vec<Grapheme>,
    /// Arena backing these display lines' `CellContent::Virtual` text ranges
    /// (the providers' `VirtualLine::text`).
    pub texts: String,
}

impl VirtualFormat {
    /// Empty, with nothing allocated yet: every line entry holds one, with or
    /// without virtual display lines.
    pub fn new() -> Self {
        Self {
            display_lines: Vec::new(),
            before: 0,
            base_scopes: Vec::new(),
            graphemes: Vec::new(),
            texts: String::new(),
        }
    }

    /// How many display lines are anchored `After` the line.
    pub fn after(&self) -> usize {
        self.display_lines.len() - self.before
    }

    /// Reset to empty, then reclaim a buffer grown past its ceiling, for the
    /// frame boundary.
    ///
    /// A multi-line deleted hunk lands in one `VirtualFormat`, so the ceilings
    /// are the content-line ones: such a hunk is not freed and re-grown every
    /// frame.
    pub fn clear_and_shrink(&mut self) {
        self.display_lines.clear();
        self.before = 0;
        self.base_scopes.clear();
        self.graphemes.clear();
        self.texts.clear();
        if self.graphemes.capacity() > GRAPHEMES_CEILING {
            self.graphemes.shrink_to(GRAPHEMES_CEILING);
        }
        if self.texts.capacity() > LINE_TEXT_CEILING {
            self.texts.shrink_to(LINE_TEXT_CEILING);
        }
    }
}

impl Default for VirtualFormat {
    fn default() -> Self {
        Self::new()
    }
}

/// How far into a line [`format_buffer_line`](super::format_buffer_line) needs to scan.
///
/// A query for one position in a line (where a char offset sits,
/// `ToByte`; which char a display column lands on, `ToDisplayCol`) can stop
/// once the scan passes that point instead of walking a long unwrapped line
/// to the end.
///
/// The stop is an optimization only. A bounded scan emits a strict prefix of
/// what `Full` emits (`clipped` suppresses only the end-of-line tail), and
/// every consumer is prefix-stable: `DisplayLineMap::locate` never reads past
/// its target, `char_at`/`Cell` takes the first cell containing the column,
/// and `char_at`/`NearestContent` and `NearestInsertionPoint` take the first
/// column-nearest eligible cell. So `Full` can stand in for any bound.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FormatBound {
    /// Scan the whole line. Required whenever the display line *count*
    /// matters (any wrapping mode) or the caller reads the line's tail.
    Full,
    /// Stop after the grapheme containing this line-relative byte offset.
    ToByte(ByteCol),
    /// Stop after the first grapheme whose own start display column is past
    /// this one.
    ToDisplayCol(DisplayLineCol),
}

impl FormatBound {
    /// Whether a scan already run to `self` also answers a request for
    /// `other`. A `false` costs a reformat, never a stale read.
    ///
    /// Cross-kind pairs never cover each other: a byte bound implies no
    /// useful column bound (a 4-byte char is one column) and vice versa (a
    /// tab is one byte and up to 255 columns).
    pub fn covers(self, other: Self) -> bool {
        match (self, other) {
            (Self::Full, _) => true,
            (Self::ToByte(a), Self::ToByte(b)) => a >= b,
            (Self::ToDisplayCol(a), Self::ToDisplayCol(b)) => a >= b,
            _ => false,
        }
    }

    /// Whether a just-emitted grapheme spanning `bytes` and starting at
    /// display column `start_display_col` carries the scan past this bound.
    ///
    /// `ToDisplayCol` tests the grapheme's *own* start display column, not
    /// the running column after it: a wide cell (tab expanse, CJK glyph) can
    /// straddle the target, and stopping on the running column would drop
    /// the cell to its right, which may be strictly nearer the target than
    /// the straddling one, changing what `NearestContent` answers.
    pub(super) fn reached(
        self,
        bytes: ExclusiveRange<ByteCol>,
        start_display_col: DisplayLineCol,
    ) -> bool {
        match self {
            Self::Full => false,
            Self::ToByte(b) => bytes.contains(b),
            Self::ToDisplayCol(t) => start_display_col > t,
        }
    }
}
