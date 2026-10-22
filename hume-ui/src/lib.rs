//! Editor-widget rendering layer: overlays (popup, menu, drawer, picker)
//! and the shared box-drawing/scroll math they build on.
//!
//! Nothing here references `Editor` or `EditorState`: each type is a value
//! object or a provider reading from a handle it was given. The overlay
//! models holding editor-owned input state (`PopupLayer`, `MenuLayer`,
//! `DrawerLayer`, `ConfirmLayer`, `PickerSession`) and the statusline, which
//! reads editor state directly, live in `hume-editor`.
//!
//! Plugin decoration stores and their providers live in `hume-decorations`.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod drawer;
pub mod menu_box;
pub mod picker_panel;
pub mod popup;
pub mod width;

use hume_engine::lock::SharedSlot;
use hume_engine::pipeline::BOTTOM_BANDS;
use hume_engine::providers::{ChromeProvider, OverlayProvider};
use hume_engine::theme::ui_scopes;

use picker_panel::PickerOverlay;
use popup::PopupOverlay;

/// Every overlay view shared between the editor's per-frame write side and
/// the engine's render side. One owner, so a new overlay is one field
/// instead of a seventh hand-allocated `Arc` threaded through `Editor::
/// open`, `EditorState`, and [`OverlayViews::overlays`].
/// Bundling as one struct with distinctly-named fields (not loose
/// same-typed `Arc` parameters) is what stops two different slots (same
/// `SharedSlot<Option<PopupState>>` type for `minibuf_completion`/`popup`/
/// `menu`/`completion_menu`) from being wired to each other by mistake: a
/// `views.popup` at a construction site can't silently become `views.menu`
/// the way two positional `Arc` arguments of the same type could swap.
///
/// Fields are `pub`, not behind a getter/setter pair: every field is a
/// [`SharedSlot`], whose own `read()`/`set()` already carry the poison
/// policy and the only mutation this type ever needs. A wrapping accessor
/// pair here would have been a pass-through with nothing left to add.
#[derive(Default)]
pub struct OverlayViews {
    pub minibuf_completion: SharedSlot<Option<popup::PopupState>>,
    pub popup: SharedSlot<Option<popup::PopupState>>,
    pub popup_band: SharedSlot<Option<popup::PopupBandState>>,
    pub menu: SharedSlot<Option<popup::PopupState>>,
    pub completion_menu: SharedSlot<Option<popup::PopupState>>,
    pub completion_doc: SharedSlot<Option<popup::PopupState>>,
    pub drawer: SharedSlot<Option<drawer::DrawerViewState>>,
    pub picker: SharedSlot<Option<picker_panel::PickerViewState>>,
}

/// One value per bottom band. [`Self::in_stack_order`] is the one place that
/// orders the bands, for both their painters and their rows.
pub struct Bands<T> {
    pub drawer: T,
    pub popup: T,
}

impl<T> Bands<T> {
    /// The bands bottom-up: the drawer directly above the statusline, a
    /// docked popup above it.
    pub fn in_stack_order(self) -> [T; BOTTOM_BANDS] {
        let Self { drawer, popup } = self;
        [drawer, popup]
    }
}

impl OverlayViews {
    /// Painters for the two chrome bands (drawer, docked popup), each reading
    /// this set's band-shaped slot, for `EngineView::set_chrome`.
    pub fn bottom_bands(&self) -> Bands<Box<dyn ChromeProvider>> {
        Bands {
            drawer: Box::new(drawer::DrawerWidget {
                data: self.drawer.clone(),
            }),
            popup: Box::new(popup::PopupBandWidget {
                data: self.popup_band.clone(),
            }),
        }
    }

    /// Every floating overlay reading this set's slots, for
    /// `EngineView::overlays`. **Call once**: a second call paints each
    /// overlay twice.
    ///
    /// List order is z-order (last paints on top): minibuffer completion,
    /// then the hover/signature-help popup, then the selection menu, then
    /// the LSP completion menu (an in-progress completion is the most
    /// action-relevant overlay when more than one could theoretically be
    /// visible) and the box beside it with the selected candidate's
    /// documentation, then the picker last, since it's full-modal and every
    /// other opener clears or truncates it before landing above it.
    /// `show-popup!`'s own `'scrollable` arm refuses to push above an open
    /// picker for this reason, so this order and input precedence (push
    /// order on `InputStack`, decided per event) always agree.
    pub fn overlays(&self) -> Vec<Box<dyn OverlayProvider>> {
        vec![
            Box::new(PopupOverlay {
                data: self.minibuf_completion.clone(),
                scope: ui_scopes::MENU,
            }),
            Box::new(PopupOverlay {
                data: self.popup.clone(),
                scope: ui_scopes::POPUP,
            }),
            Box::new(PopupOverlay {
                data: self.menu.clone(),
                scope: ui_scopes::MENU,
            }),
            Box::new(PopupOverlay {
                data: self.completion_menu.clone(),
                scope: ui_scopes::MENU,
            }),
            Box::new(PopupOverlay {
                data: self.completion_doc.clone(),
                scope: ui_scopes::POPUP,
            }),
            Box::new(PickerOverlay {
                data: self.picker.clone(),
            }),
        ]
    }
}

/// Rows of `area` as plain symbols, trailing spaces trimmed per row.
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
