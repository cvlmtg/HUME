use std::sync::Arc;

use crate::providers::{GutterCell, GutterCellContent, GutterColumn, GutterCtx, GutterInputs};
use crate::types::{DisplayLineKind, ScopeId};
use hume_rope::line::ContentLine;

/// One sign a `SignSource` renders for a buffer line (diagnostic marker, git
/// change indicator, breakpoint, bookmark, ...).
#[derive(Clone, Debug)]
pub struct Sign {
    /// 1–2 cells wide; wider is truncated to the column width by
    /// `render::compose_gutter`, same as any other gutter cell text.
    /// `Arc<str>`, not `Cow<'static, str>`: the editor's per-frame sign
    /// bridge builds a `Sign` straight from its decoration-store entry's own
    /// `Arc<str>` text (`Editor::update_sign_providers` in `hume-editor`),
    /// and `render_cells` below moves it into a
    /// `GutterCellContent::Shared` unchanged. Every hop from the store to
    /// the rendered cell is a refcount bump, never a `String` allocation.
    pub text: Arc<str>,
    /// Already-interned, the same contract as every `DecorationSource`: intern
    /// at the Steel `set-signs!` boundary (`host_impl.rs` in
    /// `hume-editor`), before the first render.
    pub scope: ScopeId,
    /// Which slot of the column this sign occupies. Stable for the whole
    /// buffer, resolved upstream from the registered sign sources (see
    /// `Editor::update_sign_providers`), not recomputed here from this
    /// line's signs alone. A slot `>=` the column's configured slot count is
    /// silently dropped by `SignColumn::render_cells`.
    pub slot: u8,
}

/// A source of signs for buffer lines (diagnostics, git status, breakpoints,
/// bookmarks, ...). Production wraps exactly one `SignSource`
/// (`SharedSignSource`, `hume-editor`): every plugin channel's signs are
/// already merged into its one `line -> Vec<Sign>` map before `SignColumn`
/// ever sees them (`Editor::update_sign_providers`), so a line with several
/// signs is one `signs_for_line` call returning several entries, not several
/// sources each returning one.
pub trait SignSource {
    /// Signs for one buffer line. Order doesn't matter: each sign carries
    /// its own `slot`. Called per `LineStart` display line per frame, so
    /// implementations should be cheap lookups into their own state (same
    /// contract as `DecorationSource`).
    fn signs_for_line(&self, line_idx: hume_rope::line::ContentLine, ctx: &GutterCtx) -> Vec<Sign>;
}

/// A source with no signs at all, used wherever a test needs an inert
/// `SignColumn` and doesn't care what fires.
#[cfg(test)]
impl SignSource for () {
    fn signs_for_line(
        &self,
        _line_idx: hume_rope::line::ContentLine,
        _ctx: &GutterCtx,
    ) -> Vec<Sign> {
        Vec::new()
    }
}

/// Built-in gutter column that places its `SignSource`'s signs into their own
/// resolved `slot`s. Its width is
/// [`GutterInputs::sign_width`](crate::providers::GutterInputs), one cell per
/// slot plus a padding cell, so a sign whose slot doesn't fit is hidden.
pub struct SignColumn {
    source: Box<dyn SignSource>,
    /// Interned `"ui.linenr"`: unfilled sign slots render blank under this
    /// scope, same fallback `LineNumberColumn` uses for its own non-content
    /// display lines. Interned by the caller at pane construction.
    blank_scope: ScopeId,
}

impl SignColumn {
    /// Gutter width for `slots` sign slots: one cell per slot plus one
    /// column of right-padding, matching every other gutter column's
    /// separator convention (see `render::compose_gutter`). The single place
    /// this arithmetic direction is written; `render_cells`'s
    /// `sign_width.saturating_sub(1)` is its inverse.
    pub const fn width_for_slots(slots: u8) -> u8 {
        slots.saturating_add(1)
    }

    pub fn new(source: Box<dyn SignSource>, blank_scope: ScopeId) -> Self {
        Self {
            source,
            blank_scope,
        }
    }
}

impl GutterColumn for SignColumn {
    fn width(&self, _last_line_idx: hume_rope::line::RopeyLine, inputs: GutterInputs) -> u8 {
        inputs.sign_width
    }

    fn render_cells(&self, kind: DisplayLineKind, ctx: &GutterCtx) -> Vec<GutterCell> {
        let max_signs = ctx.inputs.sign_width.saturating_sub(1) as usize;
        let DisplayLineKind::LineStart { line_idx } = kind else {
            // Wrap/Virtual/Filler display lines never carry a sign: one blank cell
            // per configured slot so `compose_gutter`'s cell count matches
            // the column's width.
            return vec![GutterCell::blank(self.blank_scope); max_signs];
        };
        if max_signs == 0 {
            return Vec::new();
        }

        let mut cells = vec![GutterCell::blank(self.blank_scope); max_signs];
        // `line_idx` is `DisplayLineKind::LineStart`'s ropey-domain field (see its
        // doc), but `SignSource` deals only in real content. A sign can't
        // meaningfully attach to the phantom line. Trusted mint, not
        // `RopeyLine::to_content`: a real render walk never emits a
        // `LineStart` this far past content, so there is nothing here to
        // validate against `ctx.rope`, only a domain tag to attach.
        let content_line = ContentLine::new(line_idx.index());
        // This loop places, it never ranks: each sign already carries its
        // own resolved `slot` (`DecorationStores::signs_in_range`). Two
        // signs from the source claiming the same slot on one line is a
        // source bug (undefined which wins; see `SignSource::signs_for_line`'s
        // contract). A slot `>= max_signs` is dropped.
        for sign in self.source.signs_for_line(content_line, ctx) {
            if let Some(cell) = cells.get_mut(sign.slot as usize) {
                *cell = GutterCell {
                    content: GutterCellContent::Shared(Arc::clone(&sign.text)),
                    scope: sign.scope,
                };
            }
        }
        cells
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
