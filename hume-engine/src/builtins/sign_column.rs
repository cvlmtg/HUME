use std::any::Any;
use std::borrow::Cow;

use crate::providers::{GutterCell, GutterCellContent, GutterColumn, GutterRowCtx, ProviderId};
use crate::types::{RowKind, ScopeId};

/// Default configured width of a `SignColumn`: one sign cell plus one column
/// of right-padding, matching every other gutter column's separator
/// convention (see `render::compose_gutter`).
const DEFAULT_WIDTH: u8 = 2;

/// One sign a `SignSource` renders for a buffer line (diagnostic marker, git
/// change indicator, breakpoint, bookmark, ...).
#[derive(Clone, Debug)]
pub struct Sign {
    /// 1–2 cells wide; wider is truncated to the column width by
    /// `render::compose_gutter`, same as any other gutter cell text.
    pub text: Cow<'static, str>,
    /// Already-interned — same contract as every `DecorationSource`: intern
    /// at provider-construction time, before the first render.
    pub scope: ScopeId,
    /// Which slot of the column this sign occupies — stable for the whole
    /// buffer, resolved upstream from the buffer's live sign-priority ladder
    /// (see `Editor::update_sign_providers`), not recomputed here from this
    /// line's signs alone. A slot `>=` the column's configured slot count is
    /// silently dropped by `SignColumn::render_row_cells`.
    pub slot: u8,
}

/// A source of signs for buffer lines (diagnostics, git status, breakpoints,
/// bookmarks, ...). Multiple sources can share one `SignColumn`, each sign
/// landing in its own resolved `slot` — this is what lets several features
/// merge into one gutter column without their signs colliding or reordering
/// each other from line to line.
pub trait SignSource {
    /// Signs for one buffer line. Order doesn't matter — each sign carries
    /// its own `slot` — but two signs from the same source claiming the same
    /// slot on one line is a source bug (undefined which wins). Called per
    /// `LineStart` row per frame — implementations should be cheap lookups
    /// into their own state (same contract as `DecorationSource`).
    fn signs_for_line(&self, line_idx: usize, ctx: &GutterRowCtx) -> Vec<Sign>;
}

/// Built-in gutter column that merges signs from multiple `SignSource`s,
/// placing each sign into its own resolved `slot` (where slot count =
/// configured sign slots = `width - 1`). A sign whose slot doesn't fit the
/// configured width is hidden.
///
/// Registered like any other `GutterColumn` via
/// `ProviderSet::add_gutter_column`, which returns the column's own
/// `ProviderId`. Sign sources are added separately, after registration, by
/// finding the column and downcasting to `SignColumn` — the same pattern
/// `ProviderSet::sync_line_number_style` already uses to reach a
/// `LineNumberColumn` post-registration (`as_any_mut().downcast_mut::<SignColumn>()`).
pub struct SignColumn {
    sources: Vec<(ProviderId, Box<dyn SignSource>)>,
    width: u8,
    next_id: ProviderId,
    /// Interned `"ui.linenr"` — unfilled sign slots render blank under this
    /// scope, same fallback `LineNumberColumn` uses for its own non-content
    /// rows. Interned by the caller at pane construction.
    blank_scope: ScopeId,
}

impl SignColumn {
    pub fn new(blank_scope: ScopeId) -> Self {
        Self {
            sources: Vec::new(),
            width: DEFAULT_WIDTH,
            next_id: 0,
            blank_scope,
        }
    }

    /// No production caller — production always starts at `DEFAULT_WIDTH`
    /// via `new` and resizes through `set_width`; kept as a test helper for
    /// constructing a lane already at a specific width.
    #[cfg(test)]
    pub fn with_width(width: u8, blank_scope: ScopeId) -> Self {
        Self {
            width,
            ..Self::new(blank_scope)
        }
    }

    /// Register a sign source. Mints its own `ProviderId` — `SignColumn` is
    /// self-contained rather than reusing the pane's `ProviderSet` allocator,
    /// since a sign source isn't one of `ProviderSet`'s five provider kinds.
    pub fn add_source(&mut self, source: Box<dyn SignSource>) -> ProviderId {
        let id = self.next_id;
        debug_assert!(self.next_id < ProviderId::MAX, "SignColumn id overflow");
        self.next_id += 1;
        self.sources.push((id, source));
        id
    }

    /// Remove the sign source registered under `id`. Returns `true` if one
    /// was removed. No production caller — sign sources are registered for
    /// a buffer's lifetime and never individually retracted; kept as a test
    /// helper for exercising multi-source lane resolution.
    #[cfg(test)]
    pub fn remove_source(&mut self, id: ProviderId) -> bool {
        let before = self.sources.len();
        self.sources.retain(|(pid, _)| *pid != id);
        before != self.sources.len()
    }

    /// Set the configured column width. Unlike the sources themselves, width
    /// is not derived from them automatically — a caller wanting the column
    /// to collapse to `0` when no source would fire for the current buffer
    /// (or grow back to the default when one would) sets it explicitly, per
    /// frame, via the same post-registration downcast `sync_line_number_style`
    /// uses. Two calls with the same width are a cheap no-op either way.
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
        // own resolved `slot`. Sources are visited in registration order, so
        // if two ever did claim the same slot on the same line (a source
        // bug per `SignSource::signs_for_line`'s contract), the
        // later-registered source wins. A slot `>= max_signs` is dropped.
        for (_, src) in &self.sources {
            for sign in src.signs_for_line(line_idx, ctx) {
                if let Some(cell) = cells.get_mut(sign.slot as usize) {
                    *cell = GutterCell {
                        content: GutterCellContent::Text(sign.text),
                        scope: sign.scope,
                    };
                }
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
