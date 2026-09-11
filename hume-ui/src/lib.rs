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

use std::sync::{Arc, RwLock, RwLockReadGuard};

use hume_engine::lock::LockExt;
use hume_engine::providers::{BottomBandProvider, ProviderSet};

use completion_overlay::MinibufCompletionOverlay;
use picker_panel::PickerOverlay;
use popup::PopupOverlay;

/// Every overlay view shared between the editor's per-frame write side and
/// the engine's render side. One owner, so a new overlay is one field plus
/// one accessor pair here instead of a seventh hand-allocated `Arc` threaded
/// through `Editor::open`, `EditorState`, and [`register_overlays`]'s
/// parameter list — and nothing stops two different slots (same `Arc<RwLock<
/// Option<PopupState>>>` type for `popup`/`menu`/`completion_menu`) from
/// being wired to each other by mistake, the way loose same-typed `Arc`
/// parameters could be.
#[derive(Default)]
pub struct OverlayViews {
    minibuf_completion: Arc<RwLock<Option<completion_overlay::MinibufCompletionView>>>,
    popup: Arc<RwLock<Option<popup::PopupState>>>,
    popup_band: Arc<RwLock<Option<popup::PopupBandState>>>,
    menu: Arc<RwLock<Option<popup::PopupState>>>,
    completion_menu: Arc<RwLock<Option<popup::PopupState>>>,
    drawer: Arc<RwLock<Option<drawer::DrawerViewState>>>,
    picker: Arc<RwLock<Option<picker_panel::PickerViewState>>>,
}

impl OverlayViews {
    pub fn minibuf_completion(
        &self,
    ) -> RwLockReadGuard<'_, Option<completion_overlay::MinibufCompletionView>> {
        self.minibuf_completion.read_or_panic()
    }
    pub fn set_minibuf_completion(&self, view: Option<completion_overlay::MinibufCompletionView>) {
        *self.minibuf_completion.write_or_panic() = view;
    }

    pub fn popup(&self) -> RwLockReadGuard<'_, Option<popup::PopupState>> {
        self.popup.read_or_panic()
    }
    pub fn set_popup(&self, view: Option<popup::PopupState>) {
        *self.popup.write_or_panic() = view;
    }

    pub fn popup_band(&self) -> RwLockReadGuard<'_, Option<popup::PopupBandState>> {
        self.popup_band.read_or_panic()
    }
    pub fn set_popup_band(&self, view: Option<popup::PopupBandState>) {
        *self.popup_band.write_or_panic() = view;
    }

    pub fn menu(&self) -> RwLockReadGuard<'_, Option<popup::PopupState>> {
        self.menu.read_or_panic()
    }
    pub fn set_menu(&self, view: Option<popup::PopupState>) {
        *self.menu.write_or_panic() = view;
    }

    pub fn completion_menu(&self) -> RwLockReadGuard<'_, Option<popup::PopupState>> {
        self.completion_menu.read_or_panic()
    }
    pub fn set_completion_menu(&self, view: Option<popup::PopupState>) {
        *self.completion_menu.write_or_panic() = view;
    }

    pub fn drawer(&self) -> RwLockReadGuard<'_, Option<drawer::DrawerViewState>> {
        self.drawer.read_or_panic()
    }
    pub fn set_drawer(&self, view: Option<drawer::DrawerViewState>) {
        *self.drawer.write_or_panic() = view;
    }

    pub fn picker(&self) -> RwLockReadGuard<'_, Option<picker_panel::PickerViewState>> {
        self.picker.read_or_panic()
    }
    pub fn set_picker(&self, view: Option<picker_panel::PickerViewState>) {
        *self.picker.write_or_panic() = view;
    }

    /// The two chrome bands (drawer, docked popup) reading this set's
    /// band-shaped slots — for `EngineView::bottom_bands`. **Call once**: a
    /// second call registers a duplicate band painting the same data twice.
    /// `Editor::open` is the only caller; `Editor::for_testing` deliberately
    /// registers none (see `lsp_popup.rs`'s doc on why).
    pub fn bottom_bands(&self) -> Vec<Box<dyn BottomBandProvider>> {
        vec![
            Box::new(drawer::DrawerWidget {
                data: Arc::clone(&self.drawer),
            }),
            Box::new(popup::PopupBandWidget {
                data: Arc::clone(&self.popup_band),
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
/// theoretically be visible), then the picker last — it's full-modal and its
/// key routing (`handle_key`) sits above every other intercept, so its paint
/// must sit above every other overlay too.
pub fn register_overlays(providers: &mut ProviderSet, views: &OverlayViews) {
    providers.add_overlay(Box::new(MinibufCompletionOverlay {
        data: Arc::clone(&views.minibuf_completion),
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(&views.popup),
        scope: "ui.popup",
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(&views.menu),
        scope: "ui.menu",
    }));
    providers.add_overlay(Box::new(PopupOverlay {
        data: Arc::clone(&views.completion_menu),
        scope: "ui.menu",
    }));
    providers.add_overlay(Box::new(PickerOverlay {
        data: Arc::clone(&views.picker),
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
