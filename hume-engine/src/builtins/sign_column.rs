use std::any::Any;
use std::sync::Arc;

use crate::providers::{GutterCell, GutterCellContent, GutterColumn, GutterRowCtx};
use crate::types::{RowKind, ScopeId};

/// One sign a `SignSource` renders for a buffer line (diagnostic marker, git
/// change indicator, breakpoint, bookmark, ...).
#[derive(Clone, Debug)]
pub struct Sign {
    /// 1–2 cells wide; wider is truncated to the column width by
    /// `render::compose_gutter`, same as any other gutter cell text.
    /// `Arc<str>`, not `Cow<'static, str>`: the editor's per-frame sign
    /// bridge builds a `Sign` straight from its decoration-store entry's own
    /// `Arc<str>` text (`Editor::update_sign_providers` in `hume-editor`),
    /// and `render_row_cells` below moves it into a
    /// `GutterCellContent::Shared` unchanged — every hop from the store to
    /// the rendered cell is a refcount bump, never a `String` allocation.
    pub text: Arc<str>,
    /// Already-interned — same contract as every `DecorationSource`: intern
    /// at the Steel `set-signs!` boundary (`host_impl.rs` in
    /// `hume-editor`), before the first render.
    pub scope: ScopeId,
    /// Which slot of the column this sign occupies — stable for the whole
    /// buffer, resolved upstream from the registered sign sources (see
    /// `Editor::update_sign_providers`), not recomputed here from this
    /// line's signs alone. A slot `>=` the column's configured slot count is
    /// silently dropped by `SignColumn::render_row_cells`.
    pub slot: u8,
}

/// A source of signs for buffer lines (diagnostics, git status, breakpoints,
/// bookmarks, ...). Production wraps exactly one `SignSource`
/// (`SharedSignSource`, `hume-editor`) — every plugin channel's signs are
/// already merged into its one `line -> Vec<Sign>` map before `SignColumn`
/// ever sees them (`Editor::update_sign_providers`), so a line with several
/// signs is one `signs_for_line` call returning several entries, not several
/// sources each returning one.
pub trait SignSource {
    /// Signs for one buffer line. Order doesn't matter — each sign carries
    /// its own `slot`. Called per `LineStart` row per frame — implementations
    /// should be cheap lookups into their own state (same contract as
    /// `DecorationSource`).
    fn signs_for_line(&self, line_idx: usize, ctx: &GutterRowCtx) -> Vec<Sign>;
}

/// A source with no signs at all — used wherever a test needs an inert
/// `SignColumn` and doesn't care what fires.
#[cfg(test)]
impl SignSource for () {
    fn signs_for_line(&self, _line_idx: usize, _ctx: &GutterRowCtx) -> Vec<Sign> {
        Vec::new()
    }
}

/// Built-in gutter column that places its `SignSource`'s signs into their own
/// resolved `slot`s (where slot count = configured sign slots = `width - 1`).
/// A sign whose slot doesn't fit the configured width is hidden.
///
/// Registered like any other `GutterColumn` via
/// `ProviderSet::add_gutter_column`, which returns the column's own
/// `ProviderId`. Width is adjusted separately, after registration, by
/// finding the column and downcasting to `SignColumn` — the same pattern
/// `ProviderSet::sync_line_number_style` already uses to reach a
/// `LineNumberColumn` post-registration (`as_any_mut().downcast_mut::<SignColumn>()`).
pub struct SignColumn {
    source: Box<dyn SignSource>,
    width: u8,
    /// Interned `"ui.linenr"` — unfilled sign slots render blank under this
    /// scope, same fallback `LineNumberColumn` uses for its own non-content
    /// rows. Interned by the caller at pane construction.
    blank_scope: ScopeId,
}

impl SignColumn {
    /// Gutter width for `slots` sign slots: one cell per slot plus one
    /// column of right-padding, matching every other gutter column's
    /// separator convention (see `render::compose_gutter`). The single place
    /// this arithmetic direction is written — `render_row_cells`'s
    /// `width.saturating_sub(1)` is its inverse, computing slots from a
    /// stored width instead.
    pub const fn width_for_slots(slots: u8) -> u8 {
        slots.saturating_add(1)
    }

    pub fn new(source: Box<dyn SignSource>, blank_scope: ScopeId) -> Self {
        Self {
            source,
            width: Self::width_for_slots(1),
            blank_scope,
        }
    }

    /// No production caller — production always starts at one slot via `new`
    /// and resizes through `set_width`; kept as a test helper for
    /// constructing a lane already at a specific width.
    #[cfg(test)]
    pub fn with_width(width: u8, source: Box<dyn SignSource>, blank_scope: ScopeId) -> Self {
        Self {
            width,
            ..Self::new(source, blank_scope)
        }
    }

    /// Set the configured column width. Unlike the source itself, width
    /// is not derived from it automatically — a caller wanting the column
    /// to collapse to `0` when the source would fire nothing for the current
    /// buffer (or grow back to the default when it would) sets it
    /// explicitly, per frame, via the same post-registration downcast
    /// `sync_line_number_style` uses. Two calls with the same width are a
    /// cheap no-op either way.
    pub fn set_width(&mut self, width: u8) {
        self.width = width;
    }
}

impl GutterColumn for SignColumn {
    fn width(&self, _last_line_idx: usize) -> u8 {
        // Returns the stored field verbatim — never recomputed inline here,
        // unlike `LineNumberColumn::width()`'s whole-file-max rule, which
        // derives its answer from `last_line_idx` on every call. `auto` mode
        // still changes what's stored, just from outside this method: the
        // caller recomputes and calls `set_width` once per frame (see that
        // method's doc comment), so `width()` itself stays a plain getter.
        self.width
    }

    fn render_row_cells(&self, kind: RowKind, ctx: &GutterRowCtx) -> Vec<GutterCell> {
        let max_signs = self.width.saturating_sub(1) as usize;
        let RowKind::LineStart { line_idx } = kind else {
            // Wrap/Virtual/Filler rows never carry a sign — one blank cell
            // per configured slot so `compose_gutter`'s cell count matches
            // the column's width.
            return vec![GutterCell::blank(self.blank_scope); max_signs];
        };
        if max_signs == 0 {
            return Vec::new();
        }

        let mut cells = vec![GutterCell::blank(self.blank_scope); max_signs];
        // This loop places, it never ranks — each sign already carries its
        // own resolved `slot` (`DecorationStores::signs_in_range`). Two
        // signs from the source claiming the same slot on one line is a
        // source bug (undefined which wins — see `SignSource::signs_for_line`'s
        // contract). A slot `>= max_signs` is dropped.
        for sign in self.source.signs_for_line(line_idx, ctx) {
            if let Some(cell) = cells.get_mut(sign.slot as usize) {
                *cell = GutterCell {
                    content: GutterCellContent::Shared(Arc::clone(&sign.text)),
                    scope: sign.scope,
                };
            }
        }
        cells
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
