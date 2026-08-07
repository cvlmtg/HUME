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
    /// Higher wins when multiple sources fire for the same line. Ties
    /// resolve to the later-registered source (registration order in
    /// `SignColumn::sources`).
    pub priority: i16,
}

/// A source of signs for buffer lines (diagnostics, git status, breakpoints,
/// bookmarks, ...). Multiple sources can share one `SignColumn`, which keeps
/// the top N priority-ordered signs per line (where N = configured sign slots)
/// — this is what lets several features merge into one gutter column, with
/// the column's width deciding how many coexisting signs actually show.
pub trait SignSource {
    /// Signs for one buffer line, ordered by the source's own preference
    /// (highest priority first when it has several). Called per `LineStart`
    /// row per frame — implementations should be cheap lookups into their
    /// own state (same contract as `DecorationSource`).
    fn signs_for_line(&self, line_idx: usize, ctx: &GutterRowCtx) -> Vec<Sign>;
}

/// Built-in gutter column that merges signs from multiple `SignSource`s,
/// keeping the top N priority-ordered signs per line (where N = configured
/// sign slots = `width - 1`). Lower-priority signs that don't fit are hidden.
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
    /// was removed.
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

        let mut collected: Vec<(Sign, usize)> = self
            .sources
            .iter()
            .enumerate()
            .flat_map(|(src_idx, (_, src))| {
                src.signs_for_line(line_idx, ctx)
                    .into_iter()
                    .map(move |s| (s, src_idx))
            })
            .collect();
        // Sort by (priority desc, source_index desc) — higher priority first,
        // ties resolve to the later-registered source (matches the documented
        // tie-break rule). This is the *only* place in the sign pipeline that
        // makes an explicit same-priority tie-break decision: the editor's
        // plugin-sign pre-merge (`Editor::update_sign_providers`,
        // hume-editor/src/editor/lifecycle.rs) sorts by priority only and
        // relies on this being the sole arbiter — it must stay that way, or a
        // same-priority sign could be discarded upstream by a rule that
        // disagrees with this one. Diagnostics' severity collapse (many
        // diagnostics on a line -> the one worst) is a distinct, unrelated
        // reduction that happens before a diagnostic ever becomes a `Sign`.
        collected.sort_by(|a, b| b.0.priority.cmp(&a.0.priority).then(b.1.cmp(&a.1)));
        collected.truncate(max_signs);

        let mut cells: Vec<GutterCell> = collected
            .into_iter()
            .map(|(sign, _)| GutterCell {
                content: GutterCellContent::Text(sign.text),
                scope: sign.scope,
            })
            .collect();
        // Pad any unused slots with blanks so the cell count equals the
        // configured sign slots — `compose_gutter` relies on this to lay
        // out the column at its full width.
        while cells.len() < max_signs {
            cells.push(GutterCell::blank(self.blank_scope));
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
