//! The two per-line/per-frame formatting buffers ([`LineFormat`],
//! [`VirtualLineScratch`]) and the scan-bound type ([`FormatBound`]) that
//! [`format_buffer_line`](super::format_buffer_line) reads and writes —
//! moved out of `format.rs`'s per-role split. Pure container/capacity types
//! plus the bound enum; `format_buffer_line` itself stays in the parent.

use std::ops::Range;

use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::offset::ExclusiveRange;

use crate::types::{DisplayLine, Grapheme};

/// One buffer line's formatted display lines.
///
/// Every index inside is line-local — `DisplayLine::graphemes` indexes
/// `graphemes`, `Grapheme::byte_range` indexes `line_texts` from 0, and
/// `CellContent`'s arena `(start, len)` pairs index `virtual_texts`. That is
/// what lets one of these be held alongside others, or handed between the
/// passes that walk a line, without rebasing anything.
///
/// Lives in a [`crate::display_lines::line_store::PaneLineStore`], which owns them for as
/// long as the lines they describe are being walked and reuses their
/// allocations afterwards — so a line formatted on one frame costs the
/// allocator nothing on the next.
pub struct LineFormat {
    /// `DisplayLine`s produced for this buffer line.
    pub display_lines: Vec<DisplayLine>,
    /// `Grapheme`s for this line; display lines index into this.
    pub graphemes: Vec<Grapheme>,
    /// Pre-materialised text for this line. Written by
    /// [`format_buffer_line`](super::format_buffer_line); read by `display_lines::DisplayLineMap`'s render accessors as
    /// `RenderDisplayLine::line_text`.
    pub line_texts: String,
    /// Arena backing this line's `CellContent::Virtual` (inline inserts) and
    /// `Whitespace` (indicator glyphs) text ranges, none of which can be
    /// `&'static str` (LSP hints, Steel-configured icons). `TabFill` needs no
    /// arena entry — its text is always a single space.
    pub virtual_texts: String,
    /// How much of the line the buffers above actually cover, or `None` when
    /// nothing has been formatted into them yet — the state a line sits in
    /// while only its virtual display lines and block shape are known.
    ///
    /// A bounded scan stops early, so a later query wanting more has to
    /// reformat; see [`FormatBound::covers`].
    pub extent: Option<FormatBound>,
    /// The horizontal clip this format was cut to, if any — `DisplayLineMap`'s own
    /// `h_window` at the moment this ran. Not a formatting input in the sense
    /// `wrap_mode`/`tab_width`/etc. are (those live on `crate::display_lines::line_store::FormatKey`
    /// and invalidate the whole entry on change): a windowed format *drops*
    /// leading graphemes rather than truncating, so it answers a different
    /// question from an unclipped one over the same line. Recording it here
    /// instead lets the entry's window-independent fields (block shape,
    /// virtual display lines) survive a window change; only [`LineFormat::covers`]
    /// needs to tell the two formats apart.
    pub h_window: Option<Range<DisplayLineCol>>,
}

/// How large each buffer may stay across a frame boundary — past this,
/// [`LineFormat::reset_and_shrink`] reclaims it down to exactly this size.
///
/// Ceilings only, not starting sizes: a `LineFormat` begins empty and grows to
/// whatever its line actually needs. These sit far above an ordinary source
/// line (which vary by an order of magnitude among themselves) because they
/// only need to catch the genuinely pathological case — a minified-JS file's
/// single line, megabytes wide — that would otherwise pin that much capacity
/// for the pane's whole life, reversing the free list's own memory bound,
/// since retained allocations are exactly what the free list keeps to avoid
/// reallocating.
const DISPLAY_LINES_CEILING: usize = 256;
const GRAPHEMES_CEILING: usize = 8192;
const LINE_TEXTS_CEILING: usize = 8192;
const VIRTUAL_TEXTS_CEILING: usize = 4096;

impl LineFormat {
    /// Empty, with nothing allocated yet.
    ///
    /// One of these exists per buffer line a pass *walks*, not per line it
    /// formats — and under `WrapMode::None` block shape is known without
    /// formatting, so most of them never fill. Reserving up front would charge
    /// every walked line for buffers only a rendered one uses; the free list
    /// (see [`crate::display_lines::line_store::PaneLineStore`]) is what makes growing
    /// on demand free after the first frame anyway.
    pub fn new() -> Self {
        Self {
            display_lines: Vec::new(),
            graphemes: Vec::new(),
            line_texts: String::new(),
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
        self.line_texts.clear();
        self.virtual_texts.clear();
        self.extent = None;
        self.h_window = None;
    }

    /// [`Self::reset`] plus reclaiming any buffer grown past its ceiling —
    /// the frame-boundary counterpart to `reset`, and the exact shape
    /// [`VirtualLineScratch::clear_and_shrink`] takes for the same reason.
    ///
    /// `reset` alone runs when the same line is about to be reformatted,
    /// where shrinking would only force an immediate re-grow. This one runs
    /// from `PaneLineStore::rewind`, where the buffer's next user may be a
    /// different line or no line at all — the point where an outsized
    /// allocation is worth paying to give back.
    ///
    /// Resetting first is what lets the shrink take effect at all:
    /// [`shrink_to`](Vec::shrink_to) never drops capacity below the length
    /// still in the buffer.
    pub fn reset_and_shrink(&mut self) {
        self.reset();
        self.shrink_oversized();
    }

    /// Shrink any buffer that has grown past its ceiling back down to it.
    ///
    /// Buffers below their ceiling are untouched: keeping their capacity
    /// across reuse is the free list's whole point.
    fn shrink_oversized(&mut self) {
        // A macro rather than a helper fn: `Vec` and `String` share no trait
        // carrying `capacity`/`shrink_to`. Four one-liners naming their own
        // ceiling, so a field paired with the wrong constant reads as wrong
        // on the line it happens.
        macro_rules! shrink {
            ($buf:expr, $ceiling:expr) => {
                if $buf.capacity() > $ceiling {
                    $buf.shrink_to($ceiling);
                }
            };
        }
        shrink!(self.display_lines, DISPLAY_LINES_CEILING);
        shrink!(self.graphemes, GRAPHEMES_CEILING);
        shrink!(self.line_texts, LINE_TEXTS_CEILING);
        shrink!(self.virtual_texts, VIRTUAL_TEXTS_CEILING);
    }

    /// Whether this format already answers a query bounded by `bound`, cut to
    /// the same `h_window` the querying map is using. A windowed format never
    /// answers for an unwindowed query or vice versa — see the field doc.
    pub fn covers(&self, bound: FormatBound, h_window: Option<&Range<DisplayLineCol>>) -> bool {
        self.extent.is_some_and(|e| e.covers(bound)) && self.h_window.as_ref() == h_window
    }
}

impl Default for LineFormat {
    fn default() -> Self {
        Self::new()
    }
}

/// Scratch for laying out one virtual (non-buffer) display line.
///
/// A dedicated buffer, not a reuse of `LineFormat`'s content-line fields:
/// a `Before` virtual display line renders ahead of its line's content
/// display lines, and those may already be formatted and cached
/// (`display_lines::DisplayLineMap::block` runs the formatter in wrapping mode to count
/// wrap display lines) — clobbering the shared buffers to lay out the
/// virtual display line would destroy that cached format and force a
/// redundant reformat of the content display lines that follow.
pub struct VirtualLineScratch {
    /// The one display line laid out here. `None` before the first use.
    pub display_line: Option<DisplayLine>,
    /// Graphemes for `display_line`.
    pub graphemes: Vec<Grapheme>,
    /// Arena backing this display line's `CellContent::Virtual` text ranges
    /// — entirely the provider's `VirtualLine::text`, unlike
    /// `LineFormat::virtual_texts` which backs a content line's inline
    /// decorations.
    pub texts: String,
}

/// Ceilings for [`VirtualLineScratch`], in the same sense as
/// [`GRAPHEMES_CEILING`] and friends: a size a scratch may keep between
/// frames, not one it starts at.
///
/// Lower than the content-line ceilings because a virtual display line's
/// text is a display string a provider *built* (an inlay hint, a blame line, a
/// diagnostic), not a line read off disk — the megabytes-wide minified-JS
/// case that sets the content-line ceilings has no counterpart here.
const VIRTUAL_LINE_GRAPHEMES_CEILING: usize = 2048;
const VIRTUAL_LINE_TEXTS_CEILING: usize = 2048;

impl VirtualLineScratch {
    /// Empty, with nothing allocated yet.
    ///
    /// One of these exists per pane whether or not that pane has any virtual
    /// display lines at all, so it grows on first use rather than charging
    /// every pane up front — the same reasoning as [`LineFormat::new`].
    pub fn new() -> Self {
        Self {
            display_line: None,
            graphemes: Vec::new(),
            texts: String::new(),
        }
    }

    /// Reset to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.display_line = None;
        self.graphemes.clear();
        self.texts.clear();
    }

    /// [`Self::clear`] plus reclaiming a buffer grown past its ceiling.
    ///
    /// Split from `clear` on the same line `LineFormat` draws between
    /// [`LineFormat::reset`] and [`LineFormat::reset_and_shrink`]: `clear`
    /// runs before laying out each virtual display line and is followed
    /// immediately by filling it again, where shrinking would only force a
    /// re-grow. This one runs at the frame boundary, when the next user may
    /// be a different display line or no display line at all — the point
    /// where an outsized allocation is worth paying to give back.
    pub fn clear_and_shrink(&mut self) {
        self.clear();
        if self.graphemes.capacity() > VIRTUAL_LINE_GRAPHEMES_CEILING {
            self.graphemes.shrink_to(VIRTUAL_LINE_GRAPHEMES_CEILING);
        }
        if self.texts.capacity() > VIRTUAL_LINE_TEXTS_CEILING {
            self.texts.shrink_to(VIRTUAL_LINE_TEXTS_CEILING);
        }
    }
}

impl Default for VirtualLineScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// How far into a line [`format_buffer_line`](super::format_buffer_line) needs to scan.
///
/// A query that only wants one position out of a line — where a char offset
/// sits (`ToByte`), or which char a display column lands on (`ToDisplayCol`)
/// — has its answer as soon as the scan passes that point, so it can stop there
/// instead of walking an arbitrarily long unwrapped line to the end.
///
/// **The stop is a pure optimization, never a correctness mechanism.** A
/// bounded scan emits a strict *prefix* of what `Full` emits: it only
/// truncates, no emitted cell differs, and `clipped` suppresses only the
/// end-of-line tail. Every consumer is prefix-stable — `display_lines::DisplayLineMap::locate`
/// resolves by binary search and never reads past its target,
/// `char_at`/`Cell` takes the first cell containing the column, and
/// `char_at`/`NearestContent` takes the first column-nearest cell. So
/// scanning further than asked can never change an answer, which is what lets
/// `Full` stand in for any bound.
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
    /// `other`. Conservative by design: a `false` costs a reformat, never a
    /// stale read.
    ///
    /// Cross-kind pairs never cover each other — a byte bound implies no
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
    /// the cell to its right — which may be strictly nearer the target than
    /// the straddling one, changing what `NearestContent` answers.
    pub(super) fn reached(
        self,
        bytes: &ExclusiveRange<ByteCol>,
        start_display_col: DisplayLineCol,
    ) -> bool {
        match self {
            Self::Full => false,
            Self::ToByte(b) => bytes.contains(b),
            Self::ToDisplayCol(t) => start_display_col > t,
        }
    }
}
