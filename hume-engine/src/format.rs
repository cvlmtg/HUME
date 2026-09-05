use std::ops::Range;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use crate::pane::{WhitespaceConfig, WhitespaceRender, WrapMode};
use crate::providers::InlineInsert;
use crate::types::{CellContent, DisplayRow, Grapheme, RowKind, ScopeId};

// ---------------------------------------------------------------------------
// Formatted output
// ---------------------------------------------------------------------------

/// One buffer line's formatted display rows.
///
/// Every index inside is line-local — `DisplayRow::graphemes` indexes
/// `graphemes`, `Grapheme::byte_range` indexes `line_texts` from 0, and
/// `CellContent`'s arena `(start, len)` pairs index `virtual_texts`. That is
/// what lets one of these be held alongside others, or handed between the
/// passes that walk a line, without rebasing anything.
///
/// Lives in a [`crate::rows::line_store::PaneLineStore`], which owns them for as
/// long as the lines they describe are being walked and reuses their
/// allocations afterwards — so a line formatted on one frame costs the
/// allocator nothing on the next.
pub struct LineFormat {
    /// `DisplayRow`s produced for this buffer line.
    pub display_rows: Vec<DisplayRow>,
    /// `Grapheme`s for this line; rows index into this.
    pub graphemes: Vec<Grapheme>,
    /// Pre-materialised text for this line. Written by
    /// [`format_buffer_line`]; read by `rows::RowMap`'s render accessors as
    /// `RenderRow::line_text`.
    pub line_texts: String,
    /// Arena backing this line's `CellContent::Virtual` (inline inserts) and
    /// `Whitespace` (indicator glyphs) text ranges, none of which can be
    /// `&'static str` (LSP hints, Steel-configured icons). `TabFill` needs no
    /// arena entry — its text is always a single space.
    pub virtual_texts: String,
    /// How much of the line the buffers above actually cover, or `None` when
    /// nothing has been formatted into them yet — the state a line sits in
    /// while only its virtual rows and block shape are known.
    ///
    /// A bounded scan stops early, so a later query wanting more has to
    /// reformat; see [`FormatBound::covers`].
    pub extent: Option<FormatBound>,
    /// The horizontal clip this format was cut to, if any — `RowMap`'s own
    /// `h_window` at the moment this ran. Not a formatting input in the sense
    /// `wrap_mode`/`tab_width`/etc. are (those live on `crate::rows::line_store::FormatKey`
    /// and invalidate the whole entry on change): a windowed format *drops*
    /// leading graphemes rather than truncating, so it answers a different
    /// question from an unclipped one over the same line. Recording it here
    /// instead lets the entry's window-independent fields (block shape,
    /// virtual rows) survive a window change; only [`LineFormat::covers`]
    /// needs to tell the two formats apart.
    pub h_window: Option<Range<u32>>,
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
const DISPLAY_ROWS_CEILING: usize = 256;
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
    /// (see [`crate::rows::line_store::PaneLineStore`]) is what makes growing
    /// on demand free after the first frame anyway.
    pub fn new() -> Self {
        Self {
            display_rows: Vec::new(),
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
        self.display_rows.clear();
        self.graphemes.clear();
        self.line_texts.clear();
        self.virtual_texts.clear();
        self.extent = None;
        self.h_window = None;
    }

    /// [`Self::reset`] plus reclaiming any buffer grown past its ceiling —
    /// the frame-boundary counterpart to `reset`, and the exact shape
    /// [`VirtualRowScratch::clear_and_shrink`] takes for the same reason.
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
        shrink!(self.display_rows, DISPLAY_ROWS_CEILING);
        shrink!(self.graphemes, GRAPHEMES_CEILING);
        shrink!(self.line_texts, LINE_TEXTS_CEILING);
        shrink!(self.virtual_texts, VIRTUAL_TEXTS_CEILING);
    }

    /// Whether this format already answers a query bounded by `bound`, cut to
    /// the same `h_window` the querying map is using. A windowed format never
    /// answers for an unwindowed query or vice versa — see the field doc.
    pub fn covers(&self, bound: FormatBound, h_window: Option<&Range<u32>>) -> bool {
        self.extent.is_some_and(|e| e.covers(bound)) && self.h_window.as_ref() == h_window
    }
}

impl Default for LineFormat {
    fn default() -> Self {
        Self::new()
    }
}

/// Scratch for laying out one virtual (non-buffer) display row.
///
/// A dedicated buffer, not a reuse of `LineFormat`'s content-line fields:
/// a `Before` virtual row renders ahead of its line's content rows, and
/// those may already be formatted and cached (`rows::RowMap::block` runs the
/// formatter in wrapping mode to count wrap rows) — clobbering the shared
/// buffers to lay out the virtual row would destroy that cached format and
/// force a redundant reformat of the content rows that follow.
pub struct VirtualRowScratch {
    /// The one row laid out here. `None` before the first use.
    pub row: Option<DisplayRow>,
    /// Graphemes for `row`.
    pub graphemes: Vec<Grapheme>,
    /// Arena backing this row's `CellContent::Virtual` text ranges —
    /// entirely the provider's `VirtualLine::text`, unlike
    /// `LineFormat::virtual_texts` which backs a content line's inline
    /// decorations.
    pub texts: String,
}

/// Ceilings for [`VirtualRowScratch`], in the same sense as
/// [`GRAPHEMES_CEILING`] and friends: a size a scratch may keep between
/// frames, not one it starts at.
///
/// Lower than the content-line ceilings because a virtual row's text is a
/// display string a provider *built* (an inlay hint, a blame line, a
/// diagnostic), not a line read off disk — the megabytes-wide minified-JS
/// case that sets the content-line ceilings has no counterpart here.
const VIRTUAL_ROW_GRAPHEMES_CEILING: usize = 2048;
const VIRTUAL_ROW_TEXTS_CEILING: usize = 2048;

impl VirtualRowScratch {
    /// Empty, with nothing allocated yet.
    ///
    /// One of these exists per pane whether or not that pane has any virtual
    /// rows at all, so it grows on first use rather than charging every pane
    /// up front — the same reasoning as [`LineFormat::new`].
    pub fn new() -> Self {
        Self {
            row: None,
            graphemes: Vec::new(),
            texts: String::new(),
        }
    }

    /// Reset to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.row = None;
        self.graphemes.clear();
        self.texts.clear();
    }

    /// [`Self::clear`] plus reclaiming a buffer grown past its ceiling.
    ///
    /// Split from `clear` on the same line `LineFormat` draws between
    /// [`LineFormat::reset`] and [`LineFormat::reset_and_shrink`]: `clear`
    /// runs before laying out each virtual row and is followed immediately by
    /// filling it again, where shrinking would only force a re-grow. This one
    /// runs at the frame boundary, when the next user may be a different row
    /// or no row at all — the point where an outsized allocation is worth
    /// paying to give back.
    pub fn clear_and_shrink(&mut self) {
        self.clear();
        if self.graphemes.capacity() > VIRTUAL_ROW_GRAPHEMES_CEILING {
            self.graphemes.shrink_to(VIRTUAL_ROW_GRAPHEMES_CEILING);
        }
        if self.texts.capacity() > VIRTUAL_ROW_TEXTS_CEILING {
            self.texts.shrink_to(VIRTUAL_ROW_TEXTS_CEILING);
        }
    }
}

impl Default for VirtualRowScratch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Buffer line formatting
// ---------------------------------------------------------------------------

/// How far into a line [`format_buffer_line`] needs to scan.
///
/// A query that only wants one position out of a line — where a char offset
/// sits (`ToByte`), or which char a display column lands on (`ToDisplayCol`)
/// — has its answer as soon as the scan passes that point, so it can stop there
/// instead of walking an arbitrarily long unwrapped line to the end.
///
/// **The stop is a pure optimization, never a correctness mechanism.** A
/// bounded scan emits a strict *prefix* of what `Full` emits: it only
/// truncates, no emitted cell differs, and `clipped` suppresses only the
/// end-of-line tail. Every consumer is prefix-stable — `rows::RowMap::locate`
/// resolves by binary search and never reads past its target,
/// `char_at`/`Cell` takes the first cell containing the column, and
/// `char_at`/`NearestContent` takes the first column-nearest cell. So
/// scanning further than asked can never change an answer, which is what lets
/// `Full` stand in for any bound.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FormatBound {
    /// Scan the whole line. Required whenever the row *count* matters (any
    /// wrapping mode) or the caller reads the line's tail.
    Full,
    /// Stop after the grapheme containing this line-relative byte offset.
    ToByte(usize),
    /// Stop after the first grapheme whose own start display column is past
    /// this one.
    ToDisplayCol(u32),
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
    fn reached(self, bytes: &Range<usize>, start_display_col: u32) -> bool {
        match self {
            Self::Full => false,
            Self::ToByte(b) => bytes.contains(&b),
            Self::ToDisplayCol(t) => start_display_col > t,
        }
    }
}

/// Format one buffer line, appending zero or more `DisplayRow`s.
///
/// `h_window` clips emitted graphemes to a horizontal column range — used only
/// by the fused render pipeline in `WrapMode::None`, where a single line can be
/// arbitrarily long (a 1MB minified-JS line is a real case). Once the scan
/// passes `h_window.end`, formatting stops early (bounding CPU cost to the
/// visible prefix instead of the whole line); graphemes left of `h_window.start`
/// are scanned (needed for tab-stop column arithmetic) but not pushed, since the
/// compose stage would discard them anyway. Pass `None` for wrapping modes
/// (already bounded by `wrap_width`) and for editor-side callers, which bound
/// themselves by target position through `bound` instead — the window is a
/// *viewport* clip, and their targets are routinely outside it (secondary
/// selection heads are never tracked horizontally; the primary's own target
/// is off-window until `ensure_cursor_visible_horizontal` scrolls to it
/// afterwards). Reusing `h_window` for these queries was tried and reverted:
/// a clipped-out target silently resolves to the wrong column instead of
/// erroring.
///
/// `bound` stops the scan once the requesting query's answer is determined —
/// see [`FormatBound`]. Pass [`FormatBound::Full`] whenever the row count or
/// the line's tail matters.
#[allow(clippy::too_many_arguments)]
pub fn format_buffer_line(
    rope: &Rope,
    line_idx: usize,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    h_window: Option<Range<u32>>,
    bound: FormatBound,
    inline_inserts: &[InlineInsert],
    out: &mut LineFormat,
) {
    // The caller (`rows::RowMap::ensure_formatted`) resets `out` right before
    // this call, so `text_start` is always 0 — kept as a variable (not
    // assumed) so `line_str` below stays correct if that contract ever
    // changes. Rope chunks are valid UTF-8.
    let text_start = out.line_texts.len();
    let line_slice = rope.line(line_idx);
    // The one buffer whose final size is known before writing it. Reserving
    // turns the chunk loop into a single allocation instead of a doubling
    // chain, which matters because `LineFormat::new` deliberately hands over
    // an empty buffer.
    out.line_texts.reserve(line_slice.len_bytes());
    for chunk in line_slice.chunks() {
        out.line_texts.push_str(chunk);
    }
    // Strip the trailing `\n` ropey includes for every non-final line — the
    // EOL sentinel below is emitted only for a line that actually had one.
    let had_newline = hume_rope::lines::truncate_line_break(&mut out.line_texts);

    let line_str = &out.line_texts[text_start..];

    // Byte offset where trailing whitespace begins. A ws grapheme is
    // "trailing" iff its byte offset is at/after this point — this excludes
    // leading and interior whitespace in one check. On an all-whitespace line
    // `trim_end()` yields `""` (offset 0), so every ws char counts as trailing.
    let trailing_ws_start = line_str.trim_end().len();
    let indent_depth = hume_rope::width::indent_depth(line_str, tab_width);

    // `WrapMode { width }` stays terminal-bounded (`u16`) — widened here since
    // it's compared against `current_display_col`, which now tracks a document column
    // that can exceed a `u16`. `None` means no wrap.
    let wrap_width: Option<u32> = wrap_mode.wrap_width().map(u32::from);
    // For indent-wrap, continuation rows start at this column.
    let indent_display_cols: u32 = if matches!(wrap_mode, WrapMode::Indent { .. }) {
        hume_rope::width::indent_stop(indent_depth as u32, tab_width)
    } else {
        0
    };
    // Word/Indent backtrack to the last whitespace on overflow; Soft splits at
    // the exact wrap column.
    let word_break = matches!(wrap_mode, WrapMode::Word { .. } | WrapMode::Indent { .. });

    // ── Row / column state ────────────────────────────────────────────────
    // Aliases into the output buffers so the rest of the function can use
    // the original `rows_out` / `graphemes_out` names without further changes.
    let rows_out = &mut out.display_rows;
    let graphemes_out = &mut out.graphemes;
    let virtual_texts_out = &mut out.virtual_texts;

    let mut insert_idx = 0usize;
    let mut wrap = WrapState {
        current_display_col: 0,
        wrap_row: 0,
        row_g_start: graphemes_out.len(),
        // Word-wrap state: remember the last whitespace position in the current row.
        last_ws_g_idx: graphemes_out.len(), // grapheme index of last ws boundary
        word_break,
    };

    // Push the first row.
    rows_out.push(DisplayRow {
        kind: RowKind::LineStart { line_idx },
        graphemes: wrap.row_g_start..0, // closed later
    });

    let mut in_leading_ws = true;

    // Running absolute char position within the buffer. Populated per grapheme
    // so the style stage can resolve selection positions without rope lookups.
    let mut char_pos = rope.line_to_char(line_idx);

    // Set when the scan stopped early — either `h_window` reached its right
    // edge, or `bound` was satisfied. Everything past that point — the EOL
    // sentinel, trailing inserts, the newline indicator — sits at or beyond
    // the true end of line, so it is skipped rather than emitted at a column
    // the truncated scan never reached.
    let mut clipped = false;

    'lines: for (byte_offset, grapheme_str) in line_str.grapheme_indices(true) {
        // ── Inject inline inserts before this byte offset ─────────────────
        while insert_idx < inline_inserts.len()
            && inline_inserts[insert_idx].byte_offset <= byte_offset
        {
            if h_window
                .as_ref()
                .is_some_and(|w| wrap.current_display_col >= w.end)
            {
                clipped = true;
                break 'lines;
            }
            let ins = &inline_inserts[insert_idx];
            if wrap_width.is_none() && h_window.is_none() {
                // No wrapping and no horizontal window: `maybe_wrap` below
                // would be a no-op and the visibility check would
                // short-circuit on `h_window`'s own `None` — the only thing
                // the insert's width would answer in that case is "is it
                // empty", cheaper to ask directly than to walk every
                // grapheme cluster to sum a width nothing downstream reads.
                if !ins.text.is_empty() {
                    push_virtual_cells(
                        virtual_texts_out,
                        graphemes_out,
                        &VirtualRun {
                            text: &ins.text,
                            byte_offset,
                            char_offset: char_pos,
                            indent_depth,
                        },
                        tab_width,
                        &mut wrap.current_display_col,
                        |_| Some(ins.scope),
                    );
                }
            } else {
                // `wrap_width`/`h_window` are mutually exclusive
                // (`RowMap::with_h_window`'s own debug_assert: h_window is a
                // `WrapMode::None`-only clip), so exactly one of the two
                // branches below ever does anything to `ins_width`:
                // `maybe_wrap` moves `current_display_col` only when
                // wrapping, and the `visible` check only reads `ins_width`
                // when `h_window` is `Some` — which is only when not
                // wrapping, i.e. before `maybe_wrap` had any chance to move
                // anything. Either way `ins_width` is measured at the exact
                // column it's later used against; nothing here can go stale.
                let ins_width = hume_rope::width::str_width(
                    &ins.text,
                    wrap.current_display_col as usize,
                    tab_width,
                )
                .min(255) as u8;
                if ins_width > 0 {
                    wrap.maybe_wrap(
                        ins_width,
                        wrap_width,
                        indent_display_cols,
                        line_idx,
                        indent_depth,
                        rows_out,
                        graphemes_out,
                    );
                    let visible = h_window
                        .as_ref()
                        .is_none_or(|w| wrap.current_display_col + ins_width as u32 > w.start);
                    if visible {
                        push_virtual_cells(
                            virtual_texts_out,
                            graphemes_out,
                            &VirtualRun {
                                text: &ins.text,
                                byte_offset,
                                char_offset: char_pos,
                                indent_depth,
                            },
                            tab_width,
                            &mut wrap.current_display_col,
                            |_| Some(ins.scope),
                        );
                    } else {
                        wrap.current_display_col =
                            wrap.current_display_col.saturating_add(ins_width as u32);
                    }
                }
            }
            insert_idx += 1;
        }

        if h_window
            .as_ref()
            .is_some_and(|w| wrap.current_display_col >= w.end)
        {
            clipped = true;
            break 'lines;
        }

        // ── Skip newlines (line_str is already stripped; this guards edge cases) ──
        if grapheme_str == "\n" {
            continue;
        }
        // NOTE: newline indicator is emitted after the main loop, below.

        // ── Update leading-ws flag ─────────────────────────────────────────
        let is_ws = is_whitespace_grapheme(grapheme_str);
        if !is_ws && in_leading_ws {
            in_leading_ws = false;
        }
        let is_trailing = byte_offset >= trailing_ws_start;

        // ── Compute display width and content ─────────────────────────────
        let (width, content) = grapheme_display(
            grapheme_str,
            wrap.current_display_col,
            tab_width,
            whitespace,
            is_trailing,
            virtual_texts_out,
        );

        // ── Wrap if necessary ─────────────────────────────────────────────
        wrap.maybe_wrap(
            width,
            wrap_width,
            indent_display_cols,
            line_idx,
            indent_depth,
            rows_out,
            graphemes_out,
        );

        // A tab deferred whole to a continuation row expands from its new
        // (post-wrap) column, not the one `grapheme_display` computed it at —
        // tab width is column-dependent, unlike every other grapheme's.
        let width = if grapheme_str == "\t" {
            hume_rope::width::grapheme_width("\t", wrap.current_display_col as usize, tab_width)
                as u8
        } else {
            width
        };

        // ── Emit grapheme ─────────────────────────────────────────────────
        let char_count = grapheme_str.chars().count();
        // Read after `maybe_wrap`, which rewrites `current_display_col` when it moves
        // this grapheme to a continuation row. Shared by the pushed cell and
        // the `bound` check below so the two cannot disagree.
        let start_display_col = wrap.current_display_col;
        let byte_range = byte_offset..byte_offset + grapheme_str.len();
        let visible = h_window
            .as_ref()
            .is_none_or(|w| start_display_col + width as u32 > w.start);
        if visible {
            graphemes_out.push(Grapheme {
                byte_range: byte_range.clone(),
                char_offset: char_pos,
                display_col: start_display_col,
                width,
                content,
                indent_depth,
                scope: None,
            });
        }
        char_pos += char_count;
        wrap.current_display_col = wrap.current_display_col.saturating_add(width as u32);

        // For CJK (width == 2): emit a WidthContinuation placeholder so the
        // render stage knows not to write anything to the second cell.
        if width == 2 && visible {
            // Both cells of a double-wide char always stay on the same row.
            // Backing up the primary to avoid overflow is not yet implemented.
            graphemes_out.push(Grapheme {
                byte_range: byte_range.clone(),
                // Same char as the primary cell — this is not a distinct buffer position.
                char_offset: char_pos - char_count,
                display_col: wrap.current_display_col,
                width: 0, // zero — does not consume columns
                content: CellContent::WidthContinuation,
                indent_depth,
                scope: None,
            });
        }

        // ── Track word-break position ─────────────────────────────────────
        // Recorded after the emit above (the grapheme and, for a two-column
        // cluster, its `WidthContinuation`) so `graphemes_out.len()` already
        // points one past every cell this whitespace grapheme occupies. A
        // tab landing on a 2-column stop pushes both its own cell and a
        // continuation cell; recording the boundary before either was pushed
        // (as a bare `+ 1`) assumed one cell per whitespace grapheme and left
        // a split at this boundary stranding the continuation as the next
        // row's first cell while the tab itself stayed on the previous row.
        if is_ws && !in_leading_ws {
            wrap.last_ws_g_idx = graphemes_out.len();
        }

        // Checked here, at the very end of the iteration, so the grapheme that
        // satisfies the bound is emitted whole — with any inline inserts that
        // precede it and its own width-continuation cell. Stopping earlier
        // (inside the insert-injection loop) could leave a run of `Virtual`
        // cells as the last thing on the row, and `NearestContent` excludes
        // those, so the real grapheme they decorate would go missing.
        if bound.reached(&byte_range, start_display_col) {
            clipped = true;
            break 'lines;
        }
    }

    // ── End-of-line sentinel, trailing inserts, newline indicator ──────────
    // Skipped entirely when the h_window scan stopped early (`clipped`): all
    // three sit at or past the true end of line, which is off-screen by
    // definition once the window's right edge has been passed.
    if !clipped {
        // Emit an Empty grapheme at the char offset of the trailing `\n` whenever
        // the line has a trailing newline. This gives the cursor/selection-head a
        // cell to land on when positioned on the newline character (e.g. after `x`
        // selects the whole line). Without this, `char_offset_to_display_col` in
        // the style stage finds no grapheme at the `\n` position and leaves the cursor
        // invisible in block-cursor modes.
        //
        // For truly empty lines (just "\n") this is the only grapheme (display_col 0).
        // For non-empty lines it sits one column past the last visible character.
        if had_newline {
            // A row that fits exactly `wrap_width` columns of real content
            // has no column left for the sentinel itself — wrap it onto a
            // fresh continuation row (its own `maybe_wrap` call, same as any
            // other cell) rather than letting it land one column past the
            // pane's own right edge, where the cursor it stands in for would
            // render invisible or bleed into the divider seam.
            wrap.maybe_wrap(
                1,
                wrap_width,
                indent_display_cols,
                line_idx,
                indent_depth,
                rows_out,
                graphemes_out,
            );
            graphemes_out.push(Grapheme {
                byte_range: line_str.len()..line_str.len(),
                char_offset: char_pos, // char offset of the `\n`
                display_col: wrap.current_display_col,
                width: 1,
                content: CellContent::Empty,
                indent_depth: 0,
                scope: None,
            });
        }

        // ── Emit any trailing inline inserts ────────────────────────────────
        for ins in &inline_inserts[insert_idx..] {
            push_virtual_cells(
                virtual_texts_out,
                graphemes_out,
                &VirtualRun {
                    text: &ins.text,
                    byte_offset: line_str.len(),
                    char_offset: char_pos,
                    indent_depth,
                },
                tab_width,
                &mut wrap.current_display_col,
                |_| Some(ins.scope),
            );
        }

        // ── Newline indicator ───────────────────────────────────────────────
        // Emitted at the end of the line (after all content and trailing inserts)
        // on the last wrap row. A newline is inherently always at end-of-line,
        // so there's no "trailing vs interior" distinction here — just on/off.
        if had_newline && whitespace.newline {
            let (start, len) = push_arena_text(virtual_texts_out, whitespace.newline_char);
            graphemes_out.push(Grapheme {
                byte_range: line_str.len()..line_str.len(),
                // Same offset as the EOL sentinel (the `\n` position). Style-stage
                // lookups resolve to the *first* grapheme at a given offset, which
                // is the EOL sentinel pushed earlier in this function — the
                // indicator itself is never the cursor-cell match.
                char_offset: char_pos,
                display_col: wrap.current_display_col,
                width: 1,
                content: CellContent::Whitespace { start, len },
                indent_depth,
                scope: None,
            });
        }
    }

    // Close the last row.
    close_row_at(rows_out, wrap.row_g_start, graphemes_out.len());
}

// ---------------------------------------------------------------------------
// Wrap state
// ---------------------------------------------------------------------------

/// Mutable state for the word-wrap / soft-wrap pass inside `format_buffer_line`.
///
/// Grouping these four fields avoids threading them as separate `&mut`
/// parameters through `maybe_wrap`.
struct WrapState {
    current_display_col: u32,
    wrap_row: u16,
    /// Index into `graphemes_out` where the current display row began.
    row_g_start: usize,
    /// Grapheme index of the last seen whitespace boundary in the current
    /// row (for word-wrap backtracking) — `== row_g_start` means none has
    /// been seen yet, since a split resets both to the same value in the
    /// same `maybe_wrap` call.
    last_ws_g_idx: usize,
    /// Whether to backtrack to the last whitespace boundary on overflow.
    /// True for `Word`/`Indent`; false for `Soft`, which always splits at the
    /// exact wrap column even mid-word.
    word_break: bool,
}

impl WrapState {
    /// If adding `width` columns to `current_display_col` would overflow `wrap_width`,
    /// close the current row and start a new one. Implements word-wrap
    /// backtracking: when `word_break` is set and a whitespace boundary has
    /// been seen in the current row, the row splits there; otherwise it
    /// splits at the current grapheme (soft break, may split a word).
    #[allow(clippy::too_many_arguments)]
    fn maybe_wrap(
        &mut self,
        width: u8,
        wrap_width: Option<u32>,
        indent_display_cols: u32,
        line_idx: usize,
        indent_depth: u8,
        rows_out: &mut Vec<DisplayRow>,
        graphemes_out: &mut [Grapheme],
    ) {
        let Some(wrap_width) = wrap_width else {
            return;
        };
        if self.current_display_col + width as u32 <= wrap_width {
            return;
        }
        if self.current_display_col == 0 {
            // Single grapheme wider than the viewport — emit it anyway to avoid
            // an infinite loop. (This can happen with very wide tab stops.)
            return;
        }

        // Determine split point: backtrack to last whitespace only when word
        // breaking is enabled (Word/Indent); Soft always splits at the current
        // grapheme, mid-word if necessary.
        let split_at = if self.word_break && self.last_ws_g_idx > self.row_g_start {
            self.last_ws_g_idx
        } else {
            graphemes_out.len() // soft break: split here
        };

        // Close current row at split_at.
        close_row_at(rows_out, self.row_g_start, split_at);

        // Start new row.
        self.wrap_row += 1;
        self.row_g_start = split_at;

        // Recalculate `current_display_col` for graphemes in [split_at..] on the new row.
        let mut new_display_col = indent_display_cols;
        for g in &mut graphemes_out[split_at..] {
            g.display_col = new_display_col;
            g.indent_depth = indent_depth;
            new_display_col += g.width as u32;
        }
        self.current_display_col = new_display_col;
        self.last_ws_g_idx = split_at;

        rows_out.push(DisplayRow {
            kind: RowKind::Wrap {
                line_idx,
                wrap_row: self.wrap_row,
            },
            graphemes: self.row_g_start..0, // closed later
        });
    }
}

// ---------------------------------------------------------------------------
// Row closing helpers
// ---------------------------------------------------------------------------

/// Close the last row in `rows_out`, spanning `[row_g_start, split_at)`.
/// `split_at` is either a mid-row wrap boundary or, for the final row on a
/// line, `graphemes_out.len()` (every grapheme emitted for the line so far).
fn close_row_at(rows_out: &mut [DisplayRow], row_g_start: usize, split_at: usize) {
    if let Some(row) = rows_out.last_mut() {
        row.graphemes = row_g_start..split_at;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn is_whitespace_grapheme(s: &str) -> bool {
    s == " " || s == "\t"
}

// ---------------------------------------------------------------------------
// Grapheme display computation
// ---------------------------------------------------------------------------

/// Compute the display `width` and `CellContent` for one grapheme cluster.
///
/// Width and rendering kind both come from one `hume_rope::width::classify`
/// call — tab, space, NBSP/ideographic space, and regular graphemes alike —
/// so this and every other column computation in the workspace (editing
/// ops, Steel decorations, UI chrome) agree on where a given cluster lands,
/// and the tab-before-placeholder ordering (a tab is also a control
/// character) is decided once instead of re-tested here.
fn grapheme_display(
    grapheme_str: &str,
    current_display_col: u32,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    is_trailing: bool,
    virtual_texts: &mut String,
) -> (u8, CellContent) {
    match hume_rope::width::classify(grapheme_str, current_display_col as usize, tab_width) {
        hume_rope::width::Cluster::Tab { width } => {
            let content = if should_render_whitespace(whitespace.tab, is_trailing) {
                let (start, len) = push_arena_text(virtual_texts, whitespace.tab_char);
                CellContent::Whitespace { start, len }
            } else {
                // Tabs render as spaces when the indicator is off.
                CellContent::TabFill
            };
            (width as u8, content)
        }

        // A cluster the terminal must not be shown as itself: a control
        // character it would act on, or an invisible one it would
        // collapse. Renders as its codepoint, `<200b>`, the way Vim and
        // Emacs show them — never as a blank, which would leave a bidi
        // override looking exactly like a space. Not gated by any
        // `whitespace-*` setting: these are unrenderable rather than
        // merely invisible, and a reader who cannot see them cannot
        // review what they do. Same substitution `push_virtual_cells` and
        // `render::write_text_run` make, so the whole frame answers this
        // the same way.
        hume_rope::width::Cluster::Placeholder(p) => {
            let (start, len) = push_arena_text(virtual_texts, p.as_str());
            (len as u8, CellContent::Placeholder { start, len })
        }

        hume_rope::width::Cluster::Plain { width, .. } => {
            // Space and the invisible Unicode spaces (NBSP, ideographic
            // space) are gated by the same `space` render mode — NBSP/
            // ideographic space get a distinct glyph so a stray
            // non-breaking space stands out from an ordinary one.
            let content = if matches!(grapheme_str, " " | "\u{A0}" | "\u{3000}")
                && should_render_whitespace(whitespace.space, is_trailing)
            {
                let glyph = if grapheme_str == " " {
                    whitespace.space_char
                } else {
                    whitespace.nbsp_char
                };
                let (start, len) = push_arena_text(virtual_texts, glyph);
                CellContent::Whitespace { start, len }
            } else {
                // Regular grapheme (or a space-family one with the
                // indicator off).
                CellContent::Grapheme
            };
            (width as u8, content)
        }
    }
}

/// Push `text` into a per-frame text arena (`LineFormat::virtual_texts`
/// or `VirtualRowScratch::texts`), returning a `(start, len)` range cheap
/// enough to store in a `Copy` `CellContent`. A single line's pushed text
/// realistically never approaches the `u32`/`u16` bounds; `debug_assert`
/// catches an overflow in tests, while release saturates rather than
/// panicking (mirrors the `current_display_col` saturation pattern in
/// `format_buffer_line`).
pub(crate) fn push_arena_text(arena: &mut String, text: &str) -> (u32, u16) {
    let start = arena.len();
    arena.push_str(text);
    debug_assert!(
        u32::try_from(start).is_ok(),
        "frame arena start exceeds u32"
    );
    debug_assert!(
        u16::try_from(text.len()).is_ok(),
        "pushed text exceeds u16 length"
    );
    (
        u32::try_from(start).unwrap_or(u32::MAX),
        u16::try_from(text.len()).unwrap_or(u16::MAX),
    )
}

/// One run of virtual text to lay out, plus the buffer identity every cell it
/// produces shares. The identity is what separates the two kinds of run:
/// an inline insert decorates a real buffer grapheme and carries that
/// grapheme's position, while a virtual row has no buffer position at all
/// (`char_offset: usize::MAX`, `indent_depth: 0`).
pub(crate) struct VirtualRun<'a> {
    pub text: &'a str,
    /// A virtual cell occupies no buffer bytes, so this is never a real span
    /// — just the one position each of `push_virtual_cells`'s output
    /// `Grapheme`s reuses for both ends of their own (always-empty)
    /// `byte_range`. That value still matters: `RowMap`'s `NearestContent`
    /// filter reads emptiness to tell a `Whitespace`/`Placeholder` cell that
    /// is real content from one that only decorates, and `style_row` reads
    /// it as the byte position highlighting layers against.
    pub byte_offset: usize,
    /// For an inline insert, the char offset of the real grapheme it
    /// precedes (not `usize::MAX`): keeps the row non-decreasing in
    /// `char_offset`, which `resolve_grapheme_display_col`'s partition_point
    /// requires. Mid-line inserts are pushed before that grapheme, so ties
    /// resolve to the insert first — `resolve_grapheme_display_col` skips
    /// forward past `Virtual` cells to reach the real one. Trailing inserts
    /// share the EOL sentinel's offset (the `\n` position) since there is no
    /// later real grapheme on the row to precede.
    pub char_offset: usize,
    pub indent_depth: u8,
}

/// Push one `Grapheme`/cell per grapheme cluster of `run.text`, not one wide
/// cell for the whole string: a `Cell` renders its text at
/// exactly one column, so packing a multi-character run into a single cell
/// leaves the columns after the first unwritten by this run — whatever the
/// compose stage puts there instead (real buffer content) then wins when the
/// backend paints cell-by-cell, clobbering everything past the first
/// character.
///
/// The single emitter behind both kinds of virtual text — `format_buffer_line`'s
/// mid-line and end-of-line inline inserts, and `RowMap::segment_virtual_row`'s
/// standalone provider rows. They differ only in the identity their cells
/// carry (`run`) and in how each cell's scope resolves (`scope_at`, a
/// constant for an insert, an interval cursor for a virtual row), so
/// everything column-related — tab expansion, the control-character policy,
/// double-width continuation cells — lives here once and cannot drift
/// between them.
///
/// Widths are measured against the live `display_col`, not the run's starting
/// column, so a tab expands to the stop it actually lands on even when the
/// caller wrapped the run onto a continuation row after measuring it.
pub(crate) fn push_virtual_cells(
    arena: &mut String,
    graphemes_out: &mut Vec<Grapheme>,
    run: &VirtualRun<'_>,
    tab_width: u8,
    display_col: &mut u32,
    mut scope_at: impl FnMut(usize) -> Option<ScopeId>,
) {
    let (text_start, _) = push_arena_text(arena, run.text);
    for (byte_offset, cluster) in run.text.grapheme_indices(true) {
        // One grapheme cluster's width is always <= tab_width (u8's own max
        // 255), unlike a whole run's — no `.min(255)` cap needed before
        // narrowing.
        let classified = hume_rope::width::classify(cluster, *display_col as usize, tab_width);
        // display-width-safe: Cluster::width() reads classify()'s own decision — not a second raw measurement.
        let width = classified.width() as u8;

        // A cluster the terminal must not be shown as itself renders as its
        // codepoint, exactly as buffer text does (`grapheme_display`), and
        // occupies the columns `classify` sized for that placeholder. A tab
        // keeps its stop expansion, drawn blank like a buffer line's tab
        // with the indicator off — decoration providers have no per-line
        // whitespace setting to key off.
        //
        // `set-virtual-lines!` already substitutes control characters at the
        // Steel boundary to keep its caller's `'segments` offsets aligned,
        // but inline-insert text does not go through that path — an LSP
        // server's `InlayHint.label` reaches here verbatim — so the
        // guarantee is enforced at this chokepoint rather than at each
        // producer.
        let content = match classified {
            hume_rope::width::Cluster::Tab { .. } => CellContent::TabFill,
            hume_rope::width::Cluster::Placeholder(p) => {
                let (start, len) = push_arena_text(arena, p.as_str());
                CellContent::Placeholder { start, len }
            }
            hume_rope::width::Cluster::Plain { .. } => CellContent::Virtual {
                start: text_start + byte_offset as u32,
                len: cluster.len() as u16,
            },
        };

        graphemes_out.push(Grapheme {
            byte_range: run.byte_offset..run.byte_offset,
            char_offset: run.char_offset,
            display_col: *display_col,
            width,
            content,
            indent_depth: run.indent_depth,
            scope: scope_at(byte_offset),
        });
        *display_col = display_col.saturating_add(width as u32);

        // For a double-width cluster: a placeholder so the second cell is
        // addressable and styled with the first, matching what
        // `format_buffer_line` emits for a real buffer grapheme. Both cells
        // of a double-wide glyph always stay on the same row.
        if width == 2 {
            graphemes_out.push(Grapheme {
                byte_range: run.byte_offset..run.byte_offset,
                char_offset: run.char_offset,
                display_col: *display_col,
                width: 0, // zero — does not consume columns
                content: CellContent::WidthContinuation,
                indent_depth: run.indent_depth,
                scope: None,
            });
        }
    }
}

/// Returns `true` if a whitespace indicator should be rendered for this cell.
///
/// `is_trailing`: this grapheme's byte offset is at/after the start of the
/// line's trailing whitespace run (see `trailing_ws_start` in `format_buffer_line`).
fn should_render_whitespace(render: WhitespaceRender, is_trailing: bool) -> bool {
    match render {
        WhitespaceRender::None => false,
        WhitespaceRender::All => true,
        WhitespaceRender::Trailing => is_trailing,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
