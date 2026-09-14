//! The single emitter behind both kinds of virtual text — inline inserts
//! and standalone provider virtual lines. See [`push_virtual_cells`]'s own
//! doc for why one emitter serves both.

use hume_rope::column::{ByteCol, DisplayLineCol};
use hume_rope::offset::ExclusiveRange;
use unicode_segmentation::UnicodeSegmentation;

use crate::types::{CellContent, Grapheme, ScopeId};

/// Push `text` into a per-frame text arena (`LineFormat::virtual_texts`
/// or `VirtualLineScratch::texts`), returning a `(start, len)` range cheap
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
/// grapheme's position, while a virtual display line has no buffer position
/// at all (`char_offset: usize::MAX`, `indent_depth: 0`).
pub(crate) struct VirtualRun<'a> {
    pub text: &'a str,
    /// A virtual cell occupies no buffer bytes, so this is never a real span
    /// — just the one position each of `push_virtual_cells`'s output
    /// `Grapheme`s reuses for both ends of their own (always-empty)
    /// `byte_range`. That value still matters: `DisplayLineMap`'s `NearestContent`
    /// filter reads emptiness to tell a `Whitespace`/`Placeholder` cell that
    /// is real content from one that only decorates, and `style_display_line` reads
    /// it as the byte position highlighting layers against.
    pub byte_offset: usize,
    /// For an inline insert, the char offset of the real grapheme it
    /// precedes (not `usize::MAX`): keeps the display line non-decreasing in
    /// `char_offset`, which `resolve_grapheme_display_col`'s partition_point
    /// requires. Mid-line inserts are pushed before that grapheme, so ties
    /// resolve to the insert first — `resolve_grapheme_display_col` skips
    /// forward past `Virtual` cells to reach the real one. Trailing inserts
    /// share the EOL sentinel's offset (the `\n` position) since there is no
    /// later real grapheme on the display line to precede.
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
/// mid-line and end-of-line inline inserts, and `DisplayLineMap::segment_virtual_line`'s
/// standalone provider display lines. They differ only in the identity
/// their cells carry (`run`) and in how each cell's scope resolves
/// (`scope_at`, a constant for an insert, an interval cursor for a virtual
/// display line), so everything column-related — tab expansion, the
/// control-character policy, double-width continuation cells — lives here
/// once and cannot drift between them.
///
/// Widths are measured against the live `display_col`, not the run's starting
/// column, so a tab expands to the stop it actually lands on even when the
/// caller wrapped the run onto a continuation display line after measuring it.
pub(crate) fn push_virtual_cells(
    arena: &mut String,
    graphemes_out: &mut Vec<Grapheme>,
    run: &VirtualRun<'_>,
    tab_width: u8,
    display_col: &mut DisplayLineCol,
    mut scope_at: impl FnMut(ByteCol) -> Option<ScopeId>,
) {
    let (text_start, _) = push_arena_text(arena, run.text);
    // A virtual cell occupies no buffer bytes — see `VirtualRun::byte_offset`'s
    // doc — so every cell this run produces reuses the same always-empty range.
    let byte_range =
        ExclusiveRange::new(ByteCol::new(run.byte_offset), ByteCol::new(run.byte_offset));
    for (byte_offset, cluster) in run.text.grapheme_indices(true) {
        // One grapheme cluster's width is always <= tab_width (u8's own max
        // 255), unlike a whole run's — no `.min(255)` cap needed before
        // narrowing.
        let classified = hume_rope::width::classify(cluster, display_col.get() as usize, tab_width);
        // Cluster::width() reads classify()'s own decision — not a second raw measurement.
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
            byte_range,
            char_offset: run.char_offset,
            display_col: *display_col,
            width,
            content,
            indent_depth: run.indent_depth,
            scope: scope_at(ByteCol::new(byte_offset)),
        });
        *display_col = display_col.advance_saturating(width as u32);

        // For a double-width cluster: a placeholder so the second cell is
        // addressable and styled with the first, matching what
        // `format_buffer_line` emits for a real buffer grapheme. Both cells
        // of a double-wide glyph always stay on the same display line.
        if width == 2 {
            graphemes_out.push(Grapheme {
                byte_range,
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
