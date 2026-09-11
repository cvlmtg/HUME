//! Steel-writable decoration stores plus their concrete
//! `hume_engine::providers` implementations: gutter signs, inlay hints,
//! virtual lines, end-of-line text, bracket/search/diagnostic/extra
//! highlights, and line backgrounds. `decorations` is the write half a
//! plugin's `set-signs!`/`set-inlay-hints!`/etc. populates, plus the
//! `ChangeSet` remapping that keeps entries positioned through edits;
//! `signs`/`inline_decorations`/`virtual_lines`/`line_backgrounds`/
//! `highlight_providers` are the read half, each a [`SharedSlot`] handle
//! `hume-editor`'s per-frame sync (`decoration_providers.rs`) writes into
//! and a provider impl reads during render.
//!
//! No type here references `Editor`/`EditorState` — every provider reads
//! from a handle it was given, never from live editor state.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod decorations;
mod highlight_providers;
mod inline_decorations;
mod line_backgrounds;
mod signs;
mod virtual_lines;

// Flattens the crate's public API to one level: `build_providers`/
// `PaneDecorationHandles` already live at the root, and `decorations`'s own
// types are this crate's most-referenced surface (`DecorationStores` alone
// has ~20 external call sites) — a caller shouldn't have to know the
// `decorations` submodule exists to spell them. `decorations` itself stays
// `pub` (not folded away) since `PointAnchored`/`SourceStore`'s `K`/`T`
// bounds and a few internal helpers are easiest to browse in place.
pub use decorations::{
    DecorationStores, EolTextEntry, ExtraHighlightEntry, InlayHintEntry, LineBgEntry, Positioned,
    RangeAnchored, SignEntry, SourceStore, VirtualLineEntry,
};

use rustc_hash::FxHashMap;

use hume_engine::builtins::sign_column::{Sign, SignColumn};
use hume_engine::lock::SharedSlot;
use hume_engine::providers::{HighlightTier, InlineInsert, ProviderSet, VirtualLine};
use hume_engine::types::ScopeId;
use hume_rope::column::ByteCol;
use hume_rope::line::ContentLine;

use highlight_providers::{PaneHighlights, ScopedHighlighter};
use inline_decorations::{InlineDecorationMap, InlineDecorationProvider};
use line_backgrounds::{LineBgMap, PaneLineBackgrounds};
use signs::{SharedSignSource, SignMap};
use virtual_lines::{PaneVirtualLines, VirtualLineMap};

/// A pane's six render-decoration handles, allocated together by
/// [`build_providers`] and stored as one `SecondaryMap` entry on
/// `EditorState.panes.render` — they are always seeded and dropped as a
/// unit (never independently), and every read site borrows the map
/// shared, so bundling them costs nothing and removes the "added a new
/// per-pane provider, forgot to drop it in `drop_pane_state`" bug class.
/// Fields are private — `hume-editor`'s per-frame sync writes through the
/// `set_*` methods below, one per decoration kind, rather than reaching a
/// raw [`SharedSlot`] guard directly.
pub struct PaneDecorationHandles {
    highlights: PaneHighlights,
    signs: SignMap,
    inlay_hints: InlineDecorationMap,
    virtual_lines: VirtualLineMap,
    /// EOL text (the diagnostics plugin's per-line summary is its first
    /// client) — a second `InlineDecorationProvider` instance (same
    /// INLINE-kind `DecorationSource` shape, distinct handle/`ProviderId`)
    /// fed by `decorations.eol_text` instead of `inlay_hints`, so the two
    /// coexist on the same line without one clobbering the other.
    eol_text: InlineDecorationMap,
    line_backgrounds: LineBgMap,
}

impl PaneDecorationHandles {
    pub fn set_signs(&self, by_line: FxHashMap<ContentLine, Vec<Sign>>) {
        self.signs.set(by_line);
    }

    pub fn set_inlay_hints(&self, by_line: FxHashMap<ContentLine, Vec<InlineInsert>>) {
        self.inlay_hints.set(by_line);
    }

    pub fn set_eol_text(&self, by_line: FxHashMap<ContentLine, Vec<InlineInsert>>) {
        self.eol_text.set(by_line);
    }

    pub fn set_virtual_lines(&self, by_line: FxHashMap<ContentLine, Vec<VirtualLine>>) {
        self.virtual_lines.set(by_line);
    }

    pub fn set_line_backgrounds(&self, by_line: FxHashMap<ContentLine, ScopeId>) {
        self.line_backgrounds.set(by_line);
    }

    pub fn set_search(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        self.highlights.search.set(spans);
    }

    pub fn set_diagnostics(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        self.highlights.diagnostics.set(spans);
    }

    pub fn set_extra(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        self.highlights.extra.set(spans);
    }

    /// At most one bracket match per pane: `None` blanks it (every
    /// unfocused pane, and the focused one in Insert mode).
    pub fn set_bracket(&self, span: Option<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        self.highlights.bracket.set(span.into_iter().collect());
    }
}

/// Read-only data accessors for `hume-editor`'s own test code, which
/// asserts on what a pane's decoration providers actually hold (the raw
/// per-line map) rather than what a viewport shows. Return cloned data, not
/// the underlying [`SharedSlot`] handle — mirrors the `test-util`-gated
/// accessors on `DecorationStores` below, and lets `signs`/`inline_
/// decorations`/`virtual_lines`/`line_backgrounds` stay `pub(crate)` instead
/// of `pub`: nothing outside this crate needs the handle type itself, only
/// the data it holds at the moment of the read.
#[cfg(any(test, feature = "test-util"))]
impl PaneDecorationHandles {
    /// `tier`'s spans — every [`HighlightTier`] variant `PaneHighlights`
    /// actually stores. `HighlightTier::Syntax` has no corresponding field
    /// here (syntax highlighting is a tree-sitter-backed provider, not one
    /// of this store's four plugin/cursor-driven tiers) and panics if asked
    /// for — no test needs it, so a silent empty result would hide a wrong
    /// tier passed in rather than naming it.
    pub fn highlights(&self, tier: HighlightTier) -> Vec<(ContentLine, ByteCol, ByteCol, ScopeId)> {
        let data = match tier {
            HighlightTier::BracketMatch => &self.highlights.bracket,
            HighlightTier::SearchMatch => &self.highlights.search,
            HighlightTier::Diagnostic => &self.highlights.diagnostics,
            HighlightTier::Extra => &self.highlights.extra,
            HighlightTier::Syntax => panic!(
                "PaneHighlights has no Syntax tier — it's resolved by a separate tree-sitter-backed provider"
            ),
        };
        data.read().clone()
    }

    pub fn signs(&self) -> FxHashMap<ContentLine, Vec<Sign>> {
        self.signs.read().clone()
    }

    pub fn inlay_hints(&self) -> FxHashMap<ContentLine, Vec<InlineInsert>> {
        self.inlay_hints.read().clone()
    }

    pub fn eol_text(&self) -> FxHashMap<ContentLine, Vec<InlineInsert>> {
        self.eol_text.read().clone()
    }

    pub fn virtual_lines(&self) -> FxHashMap<ContentLine, Vec<VirtualLine>> {
        self.virtual_lines.read().clone()
    }

    pub fn line_backgrounds(&self) -> FxHashMap<ContentLine, ScopeId> {
        self.line_backgrounds.read().clone()
    }
}

/// Build every decoration-facing provider for a new pane and register them
/// into `providers`: the sign-column gutter, bracket/search/diagnostic/extra
/// highlight sources, inlay-hint and EOL-text decoration, virtual-line
/// source, and line-background tint — plus the [`PaneDecorationHandles`]
/// bundle `hume-editor`'s per-frame sync writes into. Registers the sign
/// column first: the caller must add its own (non-decoration) line-number
/// gutter column right after, so the two columns render left-to-right in
/// that order. Decoration sources register in a fixed order: EOL text after
/// inlay hints, so a diagnostic's per-line summary sorts to the right of an
/// inlay hint landing at the same byte offset.
///
/// `linenr_scope` must be the same `ScopeId` the caller also hands its
/// line-number gutter column — this crate has no `ScopeRegistry` access of
/// its own to intern `hume_engine::providers::DEFAULT_GUTTER_SCOPE` itself,
/// and a blank sign slot rendering under a different `ScopeId` than the
/// line-number column's row-fill fallback would silently disagree on
/// styling.
///
/// Sibling of `hume_ui::register_overlays`: together the two calls populate
/// one pane's `ProviderSet`. Single source of truth for the decoration half
/// of pane construction, so every pane's decoration providers render
/// identically regardless of which creation site built it.
pub fn build_providers(
    providers: &mut ProviderSet,
    linenr_scope: ScopeId,
) -> PaneDecorationHandles {
    let highlights = PaneHighlights::default();
    let signs: SignMap = SharedSlot::new(FxHashMap::default());
    let inlay_hint_map: InlineDecorationMap = SharedSlot::new(FxHashMap::default());
    let eol_text_map: InlineDecorationMap = SharedSlot::new(FxHashMap::default());
    let virtual_line_map: VirtualLineMap = SharedSlot::new(FxHashMap::default());
    let line_bg_map: LineBgMap = SharedSlot::new(FxHashMap::default());

    providers.add_gutter_column(Box::new(SignColumn::new(
        Box::new(SharedSignSource::new(signs.clone())),
        linenr_scope,
    )));

    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::BracketMatch,
        data: highlights.bracket.clone(),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::SearchMatch,
        data: highlights.search.clone(),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Diagnostic,
        data: highlights.diagnostics.clone(),
    }));
    providers.add_decoration_source(Box::new(ScopedHighlighter {
        tier: HighlightTier::Extra,
        data: highlights.extra.clone(),
    }));
    providers.add_decoration_source(Box::new(InlineDecorationProvider {
        data: inlay_hint_map.clone(),
    }));
    providers.add_decoration_source(Box::new(InlineDecorationProvider {
        data: eol_text_map.clone(),
    }));
    providers.add_decoration_source(Box::new(PaneVirtualLines {
        data: virtual_line_map.clone(),
    }));
    providers.add_decoration_source(Box::new(PaneLineBackgrounds {
        data: line_bg_map.clone(),
    }));

    PaneDecorationHandles {
        highlights,
        signs,
        inlay_hints: inlay_hint_map,
        virtual_lines: virtual_line_map,
        eol_text: eol_text_map,
        line_backgrounds: line_bg_map,
    }
}
