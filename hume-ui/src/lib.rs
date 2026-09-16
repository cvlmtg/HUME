//! Editor-widget rendering layer: overlays (popup, menu, drawer, picker)
//! and the shared box-drawing/scroll math they build on.
//!
//! No type here references `Editor` or `EditorState` — each is a value
//! object or a provider that reads from a handle it was given, not from
//! live editor state. The raw, not-yet-resolved model behind each overlay
//! (`PopupModel`, `MenuModel`, `DrawerModel`, and the confirm prompt's
//! `ConfirmModel`, which has no view-side counterpart here at all — it's
//! rendered directly by `hume-editor`'s statusline) lives in `hume-editor`
//! instead, alongside `PickerSession`: those hold editor-owned input state
//! (a not-yet-fired Steel callback, a `BufferId` to act on), not a value
//! object or a provider reading from a handle. The one widget that *does*
//! read editor state directly, the statusline, lives in `hume-editor` too
//! (see that crate's `statusline` module) rather than forcing a
//! `StatuslineData` trait on this crate for a single implementor.
//!
//! Decoration stores (the Steel-writable data a plugin's
//! `set-signs!`/`set-inlay-hints!`/etc. populates) and their concrete
//! `hume_engine::providers` implementations live in the sibling crate
//! `hume-decorations` instead — a different concern (per-buffer plugin
//! data feeding the render pipeline) from the overlay widgets here (popup/
//! menu/drawer/picker chrome), with no code shared between the two beyond
//! `hume_engine::providers` itself. [`register_overlays`] and
//! `hume_decorations::build_providers` are siblings: `hume-editor`'s pane
//! construction calls both to populate one pane's `ProviderSet`.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod completion_overlay;
pub mod drawer;
pub mod menu_box;
pub mod picker_panel;
pub mod popup;
pub mod width;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::{BottomBandProvider, ProviderSet};
use hume_engine::theme::ui_scopes;

use completion_overlay::MinibufCompletionOverlay;
use picker_panel::PickerOverlay;
use popup::PopupOverlay;

/// Every overlay view shared between the editor's per-frame write side and
/// the engine's render side. One owner, so a new overlay is one field
/// instead of a seventh hand-allocated `Arc` threaded through `Editor::
/// open`, `EditorState`, and [`register_overlays`]'s parameter list.
/// Bundling as one struct with distinctly-named fields (not loose
/// same-typed `Arc` parameters) is what stops two different slots (same
/// `SharedSlot<Option<PopupState>>` type for `popup`/`menu`/
/// `completion_menu`) from being wired to each other by mistake — a
/// `views.popup` at a construction site can't silently become `views.menu`
/// the way two positional `Arc` arguments of the same type could swap.
///
/// Fields are `pub`, not behind a getter/setter pair: every field is a
/// [`SharedSlot`], whose own `read()`/`set()` already carry the poison
/// policy and the only mutation this type ever needs — a wrapping accessor
/// pair here would have been a pass-through with nothing left to add.
#[derive(Default)]
pub struct OverlayViews {
    pub minibuf_completion: SharedSlot<Option<completion_overlay::MinibufCompletionView>>,
    pub popup: SharedSlot<Option<popup::PopupState>>,
    pub popup_band: SharedSlot<Option<popup::PopupBandState>>,
    pub menu: SharedSlot<Option<popup::PopupState>>,
    pub completion_menu: SharedSlot<Option<popup::PopupState>>,
    pub drawer: SharedSlot<Option<drawer::DrawerViewState>>,
    pub picker: SharedSlot<Option<picker_panel::PickerViewState>>,
}

impl OverlayViews {
    /// The two chrome bands (drawer, docked popup) reading this set's
    /// band-shaped slots — for `EngineView::bottom_bands`. **Call once**: a
    /// second call registers a duplicate band painting the same data twice.
    /// `Editor::open` is the only caller; `Editor::for_testing` deliberately
    /// registers none (see `lsp_popup.rs`'s doc on why).
    pub fn bottom_bands(&self) -> Vec<Box<dyn BottomBandProvider>> {
        vec![
            Box::new(drawer::DrawerWidget {
                data: self.drawer.clone(),
            }),
            Box::new(popup::PopupBandWidget {
                data: self.popup_band.clone(),
            }),
        ]
    }
}

/// Register every overlay provider (minibuffer completion, hover/hint
/// popup, selection menu, LSP completion menu, fuzzy picker) into `providers`
/// — the overlay half of pane construction. Sibling of
/// `hume_decorations::build_providers`; together the two calls populate one
/// pane's `ProviderSet`. Registration order is z-order (last registered
/// paints on top): completion overlay, then the hover/signature-help popup,
/// then the selection menu, then the LSP completion menu (an in-progress
/// completion is the most action-relevant overlay when more than one could
/// theoretically be visible), then the picker last, since it's full-modal
/// and every other synchronous opener clears or truncates it before landing
/// above it. This registration order is fixed, unlike input precedence
/// (which is push order on `InputStack`, decided per event) — the two
/// normally agree, with one known exception: an ungated `show-popup!`
/// `'scrollable` can still push a `Popup` layer above an open picker, where
/// it would own the next input event but paint underneath it.
pub fn register_overlays(providers: &mut ProviderSet, views: &OverlayViews) {
    providers.add_overlay(Box::new(MinibufCompletionOverlay {
        data: views.minibuf_completion.clone(),
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: views.popup.clone(),
        scope: ui_scopes::POPUP,
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: views.menu.clone(),
        scope: ui_scopes::MENU,
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: views.completion_menu.clone(),
        scope: ui_scopes::MENU,
    }));
    providers.add_overlay(Box::new(PickerOverlay {
        data: views.picker.clone(),
    }));
}

/// Rows of `area` as plain symbols, trailing spaces trimmed per row.
///
/// Shared by `menu_box`'s and `picker_panel`'s own test modules — both dump
/// a rendered `Grid` region to a string for `insta`/plain assertions, and
/// the dump itself is identical between the two overlay kinds.
#[cfg(test)]
pub(crate) fn symbols_in(buf: &hume_grid::Grid, area: hume_grid::Rect) -> String {
    (area.y..area.y + area.height)
        .map(|y| {
            let row: String = (area.x..area.x + area.width)
                .map(|x| buf[(x, y)].text())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
