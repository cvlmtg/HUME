//! Steel-writable decoration stores plus their concrete
//! `hume_engine::providers` implementations: gutter signs, inlay hints,
//! virtual lines, end-of-line text, bracket/search/diagnostic/extra
//! highlights, and line backgrounds. `decorations` is the write half a
//! plugin's `set-signs!`/`set-inlay-hints!`/etc. populates, plus the
//! `ChangeSet` remapping that keeps entries positioned through edits;
//! `signs`/`inline_decorations`/`virtual_lines`/`line_backgrounds`/
//! `highlight_providers` are the read half, each an `Arc<RwLock<_>>` handle
//! `hume-editor`'s per-frame sync (`decoration_providers.rs`) writes into
//! and a provider impl reads during render.
//!
//! No type here references `Editor`/`EditorState` — every provider reads
//! from a handle it was given, never from live editor state.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod decorations;
pub mod highlight_providers;
pub mod inline_decorations;
pub mod line_backgrounds;
pub mod signs;
pub mod virtual_lines;

use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use hume_engine::builtins::sign_column::{Sign, SignColumn};
use hume_engine::lock::LockExt;
use hume_engine::providers::{
    DecorationSource, GutterColumn, HighlightTier, InlineInsert, VirtualLine,
};
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
/// raw `Arc<RwLock<_>>` guard directly.
pub struct PaneDecorationHandles {
    highlights: PaneHighlights,
    signs: SignMap,
    inlay_hints: InlineDecorationMap,
    virtual_lines: VirtualLineMap,
    /// EOL text (the diagnostics plugin's per-line summary is its first
    /// client) — a second `InlineDecorationProvider` instance (same
    /// INLINE-kind `DecorationSource` shape, distinct Arc/`ProviderId`) fed
    /// by `decorations.eol_text` instead of `inlay_hints`, so the two
    /// coexist on the same line without one clobbering the other.
    eol_text: InlineDecorationMap,
    line_backgrounds: LineBgMap,
}

impl PaneDecorationHandles {
    pub fn set_signs(&self, by_line: FxHashMap<ContentLine, Vec<Sign>>) {
        *self.signs.write_or_panic() = by_line;
    }

    pub fn set_inlay_hints(&self, by_line: FxHashMap<ContentLine, Vec<InlineInsert>>) {
        *self.inlay_hints.write_or_panic() = by_line;
    }

    pub fn set_eol_text(&self, by_line: FxHashMap<ContentLine, Vec<InlineInsert>>) {
        *self.eol_text.write_or_panic() = by_line;
    }

    pub fn set_virtual_lines(&self, by_line: FxHashMap<ContentLine, Vec<VirtualLine>>) {
        *self.virtual_lines.write_or_panic() = by_line;
    }

    pub fn set_line_backgrounds(&self, by_line: FxHashMap<ContentLine, ScopeId>) {
        *self.line_backgrounds.write_or_panic() = by_line;
    }

    pub fn set_search(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        *self.highlights.search.write_or_panic() = spans;
    }

    pub fn set_diagnostics(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        *self.highlights.diagnostics.write_or_panic() = spans;
    }

    pub fn set_extra(&self, spans: Vec<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        *self.highlights.extra.write_or_panic() = spans;
    }

    /// At most one bracket match per pane: `None` blanks it (every
    /// unfocused pane, and the focused one in Insert mode).
    pub fn set_bracket(&self, span: Option<(ContentLine, ByteCol, ByteCol, ScopeId)>) {
        let mut data = self.highlights.bracket.write_or_panic();
        data.clear();
        data.extend(span);
    }
}

/// Read-only field accessors for `hume-editor`'s own test code, which
/// asserts on what a pane's decoration providers actually hold (Arc
/// identity across a resync, the raw per-line map) rather than what a
/// viewport shows. Mirrors the `test-util`-gated accessors on
/// `DecorationStores` below.
#[cfg(any(test, feature = "test-util"))]
impl PaneDecorationHandles {
    pub fn highlights(&self) -> &PaneHighlights {
        &self.highlights
    }

    pub fn signs(&self) -> &SignMap {
        &self.signs
    }

    pub fn inlay_hints(&self) -> &InlineDecorationMap {
        &self.inlay_hints
    }

    pub fn eol_text(&self) -> &InlineDecorationMap {
        &self.eol_text
    }

    pub fn virtual_lines(&self) -> &VirtualLineMap {
        &self.virtual_lines
    }

    pub fn line_backgrounds(&self) -> &LineBgMap {
        &self.line_backgrounds
    }
}

/// Build every decoration-facing provider for a new pane: the sign-column
/// gutter, bracket/search/diagnostic/extra highlight sources, inlay-hint and
/// EOL-text decoration, virtual-line source, and line-background tint —
/// plus the [`PaneDecorationHandles`] bundle `hume-editor`'s per-frame sync
/// writes into. Decoration sources are returned in registration order,
/// which the caller must preserve (`add_decoration_source` in a loop over
/// the `Vec`): EOL text is ordered after inlay hints so a diagnostic's
/// per-line summary sorts to the right of an inlay hint landing at the same
/// byte offset.
///
/// `linenr_scope` must be the same `ScopeId` the caller also hands its
/// (non-decoration) line-number gutter column — this crate has no
/// `ScopeRegistry` access of its own to intern
/// `hume_engine::providers::DEFAULT_GUTTER_SCOPE` itself, and a blank sign
/// slot rendering under a different `ScopeId` than the line-number column's
/// row-fill fallback would silently disagree on styling.
///
/// Sibling of `hume_ui::register_overlays`: together the two calls populate
/// one pane's `ProviderSet`. Single source of truth for the decoration half
/// of pane construction, so every pane's decoration providers render
/// identically regardless of which creation site built it.
pub fn build_providers(
    linenr_scope: ScopeId,
) -> (
    Box<dyn GutterColumn>,
    Vec<Box<dyn DecorationSource>>,
    PaneDecorationHandles,
) {
    let highlights = PaneHighlights::default();
    let signs: SignMap = Arc::new(RwLock::new(FxHashMap::default()));
    let inlay_hint_map: InlineDecorationMap = Arc::new(RwLock::new(FxHashMap::default()));
    let eol_text_map: InlineDecorationMap = Arc::new(RwLock::new(FxHashMap::default()));
    let virtual_line_map: VirtualLineMap = Arc::new(RwLock::new(FxHashMap::default()));
    let line_bg_map: LineBgMap = Arc::new(RwLock::new(FxHashMap::default()));

    let sign_column: Box<dyn GutterColumn> = Box::new(SignColumn::new(
        Box::new(SharedSignSource::new(Arc::clone(&signs))),
        linenr_scope,
    ));

    let sources: Vec<Box<dyn DecorationSource>> = vec![
        Box::new(ScopedHighlighter {
            tier: HighlightTier::BracketMatch,
            data: Arc::clone(&highlights.bracket),
        }),
        Box::new(ScopedHighlighter {
            tier: HighlightTier::SearchMatch,
            data: Arc::clone(&highlights.search),
        }),
        Box::new(ScopedHighlighter {
            tier: HighlightTier::Diagnostic,
            data: Arc::clone(&highlights.diagnostics),
        }),
        Box::new(ScopedHighlighter {
            tier: HighlightTier::Extra,
            data: Arc::clone(&highlights.extra),
        }),
        Box::new(InlineDecorationProvider {
            data: Arc::clone(&inlay_hint_map),
        }),
        Box::new(InlineDecorationProvider {
            data: Arc::clone(&eol_text_map),
        }),
        Box::new(PaneVirtualLines {
            data: Arc::clone(&virtual_line_map),
        }),
        Box::new(PaneLineBackgrounds {
            data: Arc::clone(&line_bg_map),
        }),
    ];

    (
        sign_column,
        sources,
        PaneDecorationHandles {
            highlights,
            signs,
            inlay_hints: inlay_hint_map,
            virtual_lines: virtual_line_map,
            eol_text: eol_text_map,
            line_backgrounds: line_bg_map,
        },
    )
}
