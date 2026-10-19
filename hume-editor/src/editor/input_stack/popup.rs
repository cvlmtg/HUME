//! The `Scrollable` popup layer (hover, `gn`/`gp`'s diagnostic overlay). A
//! `Sticky` popup (signature help) has no handler here: it lives in a mode
//! layer's `sticky_popup` slot instead, never its own layer; see
//! [`super::stack::Layer::sticky_popup_slot`]'s doc.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_engine::pipeline::{EngineView, ScreenLayout};
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::EditorMode;
use hume_scripting::host::HostToken;

use super::super::commands::half_page;
use super::super::host_token;
use super::super::{Editor, EditorState};
use super::paste::PastedText;
use super::placement::{focused_cursor, popup_placement};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler};

/// `(show-popup! text)`'s raw, unwrapped content, held on `EditorState`
/// until `Editor::sync_popup_view` or `EditorState::sync_popup_band_view`
/// (per `layout`) resolves it into a positioned
/// `hume_ui::popup::PopupState` or a `PopupBandState`. Never buried: every
/// layer's own `Layer::setup` retires it before landing (`PopupLayer`'s own,
/// for its self-replace contract; `MenuLayer`/`DrawerLayer`/`PickerLayer`,
/// which evict any open popup outright; a completion layer, which evicts
/// only this home via `InputStack::clear_popup_layer`, leaving a `Sticky` popup
/// in a mode layer's slot untouched) or is itself gated on the stack being
/// settled, so `close-popup!`/`popup()` never need to look past `top()`.
pub(in crate::editor) struct PopupLayer {
    pub(in crate::editor) text: String,
    /// First visible wrapped row, for a `Scrollable` popup. Clamped against
    /// `max_scroll` in [`scroll_box`] before each delta is applied, since
    /// content height (and so `max_scroll`) can shrink between key presses
    /// (e.g. the terminal grows) without this field being touched.
    /// `Editor::sync_popup_view` and `EditorState::sync_popup_band_view`
    /// additionally clamp a *copy* of this value for rendering; that clamp
    /// is view-only and never writes back to the model.
    pub(in crate::editor) scroll: usize,
    /// `#:lang`, rebuilt fresh on every `show-popup!`, dropped with the
    /// popup on close. No separate invalidation path: the popup's lifetime
    /// IS the syntax's (SSOT).
    pub(in crate::editor) syntax: Option<super::super::popup_syntax::MarkupSyntax>,
    pub(in crate::editor) layout: hume_ui::popup::PopupLayout,
    /// Built from `text`/`syntax` against the view's theme when the popup is
    /// created. Its own internal wrap cache is keyed by width, so an
    /// unchanged width across frames is O(1), not a re-wrap. Rebuilt by
    /// [`Layer::theme_changed`] (reached from `theme::set_theme`, the one
    /// chokepoint every `view.theme` write goes through), since the baked
    /// styles are the one input here that can change out from under an open
    /// popup, unlike `text`/`syntax`.
    content: hume_ui::popup::PopupContent,
    /// Identifies which `show-popup!` call opened this popup; see
    /// [`Self::token`]'s doc.
    token: HostToken,
}

impl PopupLayer {
    /// Mints a fresh token for this popup. The constructor is the only
    /// minting site, same as `DrawerLayer::new`.
    pub(in crate::editor) fn new(
        text: String,
        syntax: Option<super::super::popup_syntax::MarkupSyntax>,
        layout: hume_ui::popup::PopupLayout,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) -> Self {
        let content =
            super::super::popup_syntax::popup_content(&text, syntax.as_ref(), theme, registry);
        Self {
            text,
            scroll: 0,
            syntax,
            layout,
            content,
            token: host_token::mint(),
        }
    }

    /// `show-popup!`'s return value, checked by `close-popup!` so a caller
    /// only ever closes the popup it opened. Mirrors `DrawerLayer::token`.
    pub(in crate::editor) fn token(&self) -> HostToken {
        self.token
    }

    /// The popup's styled text, built against the current theme.
    pub(in crate::editor) fn content(&self) -> &hume_ui::popup::PopupContent {
        &self.content
    }

    /// Wrapped lines this popup has when docked on a terminal `area_width`
    /// columns wide: what its band's height and its paging are measured from.
    pub(in crate::editor) fn docked_total_rows(&self, area_width: u16) -> usize {
        hume_ui::popup::band_total_rows(&self.content, area_width)
    }
}

impl Layer for PopupLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn theme_changed(&mut self, theme: &mut Theme, registry: &ScopeRegistry) {
        self.content = super::super::popup_syntax::popup_content(
            &self.text,
            self.syntax.as_ref(),
            theme,
            registry,
        );
    }

    fn handler(&self) -> LayerHandler {
        popup_input
    }
    fn paste_handler(&self) -> PasteHandler {
        popup_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    // No `setup`/`popup_eviction` override: the trait's own default
    // (`PopupEviction::Both`, evicted automatically by `push_layer`) is
    // exactly `(show-popup! …)`'s documented replace-not-stack contract:
    // clears whichever of the two popup homes (a pushed `PopupLayer`, or
    // the current mode layer's sticky slot) already holds one, regardless
    // of which kind is landing now.
    /// Non-modal: a popup owns nothing but Ctrl-u/d and dies on the very
    /// next key, so an async opener's staleness check
    /// (`InputStack::is_settled_for`) must not treat one being open as "the
    /// stack moved", same reasoning as `DrawerLayer::is_modal`.
    fn is_modal(&self) -> bool {
        false
    }
}

impl Editor {
    /// Write the current *cursor-anchored* popup into the shared
    /// `PopupState` slot so `PopupOverlay` can render it during this frame,
    /// through [`Self::resolve_cursor_popup`]. The slot is cleared when no
    /// cursor popup is open, including for a docked popup
    /// (`PopupLayout::Docked`), which [`EditorState::sync_popup_band_view`]
    /// handles instead.
    ///
    /// Called from `prepare_frame` after its scroll step, with the cursor
    /// cell that step resolved as `cursor_cell`.
    pub(in crate::editor) fn sync_popup_view(
        &mut self,
        screen: &ScreenLayout,
        cursor_cell: Option<(u16, u16)>,
    ) {
        let resolved = self.resolve_cursor_popup(screen, cursor_cell);
        if resolved.is_some() || self.state.views.popup.read().is_some() {
            self.state.views.popup.set(resolved);
        }
    }

    /// The open cursor-anchored popup resolved against the focused pane's
    /// current rect and viewport: what a frame paints and what Ctrl-d/Ctrl-u
    /// page against. `None` with no cursor popup open, or when the cursor is
    /// not in view. Geometry (wrap width, flip/clamp position) is never
    /// computed at `show-popup!` call time, so a resize or scroll never
    /// leaves it stale. `screen` and `cursor_cell` are [`popup_placement`]'s.
    fn resolve_cursor_popup(
        &mut self,
        screen: &ScreenLayout,
        cursor_cell: Option<(u16, u16)>,
    ) -> Option<hume_ui::popup::PopupState> {
        let is_cursor = matches!(
            self.state.input.popup().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Cursor)
        );
        if !is_cursor {
            return None;
        }
        let anchor = focused_cursor(self);
        let placement = popup_placement(self, screen, cursor_cell, anchor)?;
        let border = self.state.settings.popup_border;
        let model = self
            .state
            .input
            .popup()
            .expect("popup present: is_cursor checked above");
        Some(hume_ui::popup::resolve_popup(
            model.content(),
            placement,
            model.scroll,
            border,
        ))
    }
}

impl EditorState {
    /// Write the current *docked* popup content into the shared
    /// `PopupBandState` Arc so `PopupBandWidget` can render it during this
    /// frame, the `PopupLayout::Docked` counterpart of
    /// [`Editor::sync_popup_view`]. Unlike the cursor layout, geometry isn't
    /// resolved here: only content (wrapped lines + scroll clamp), mirroring
    /// the drawer's chrome contract: the band's rows come from
    /// `EditorState::screen_layout` and its position from the engine at
    /// render time.
    ///
    /// Wraps and caps against `EngineView::terminal_area`: the band spans
    /// the full terminal.
    pub(in crate::editor) fn sync_popup_band_view(&mut self, view: &EngineView) {
        let is_docked = matches!(
            self.input.popup().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Docked)
        );
        if !is_docked {
            if self.views.popup_band.read().is_some() {
                self.views.popup_band.set(None);
            }
            return;
        }

        let area = view.terminal_area();
        let border = self.settings.popup_border;
        let model = self
            .input
            .popup()
            .expect("popup present: is_docked checked above");
        let resolved = hume_ui::popup::resolve_band(model.content(), area, model.scroll, border);

        self.views.popup_band.set(Some(resolved));
    }
}

/// Handles one event while a `Scrollable` popup is open (a `PopupLayer`; a
/// `Sticky` popup lives in a mode layer's slot instead and is never
/// dispatch's target). Ctrl-u/Ctrl-d consume the key only when
/// [`scroll_box`] finds content past one screenful; every other key
/// and any mouse gesture retire the layer and fall through this same call
/// (a paste too, through [`popup_paste`]), so a short popup never blocks
/// the buffer's own half-page scroll and a click still moves the cursor
/// underneath it. Unlike `Confirm`/`Menu`'s mouse arm, this doesn't gate on
/// `is_fresh_gesture`: nothing opens a popup mid-gesture, so there is no
/// release to protect the way a click-opened confirm needs its own `Up`
/// protected.
pub(in crate::editor) fn popup_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        InputEvent::Mouse(_) => {
            ed.state.truncate_layers(&ed.view, r);
            ed.fall_through(r, ev);
            return;
        }
    };
    if let Some(down) = half_page_key(&key) {
        let rows = popup_rows(ed);
        if let Some(popup) = ed.state.input.popup_mut()
            && scroll_box(&mut popup.scroll, down, rows)
        {
            return;
        }
    }
    ed.state.truncate_layers(&ed.view, r);
    ed.fall_through(r, InputEvent::Key(key));
}

/// Retires the popup and lets the layer below take the paste, as
/// [`popup_input`] does for a key it doesn't scroll with.
fn popup_paste(state: &mut EditorState, view: &EngineView, r: LayerRef, text: PastedText) {
    state.truncate_layers(view, r);
    state.fall_through_paste(view, r, text);
}

/// The open popup's visible content height and total row count, measured
/// from the model the way the next frame lays it out, so paging works on a
/// popup no frame has painted yet (one a macro's previous key opened). A
/// cursor popup's come from [`Editor::resolve_cursor_popup`], a docked
/// popup's from the rows its band is laid out with.
fn popup_rows(ed: &mut Editor) -> Option<(usize, usize)> {
    match ed.state.input.popup()?.layout {
        hume_ui::popup::PopupLayout::Cursor => {
            let screen = ed.state.screen_layout(&ed.view);
            ed.resolve_cursor_popup(&screen, None)
                .as_ref()
                .map(boxed_popup_rows)
        }
        hume_ui::popup::PopupLayout::Docked => {
            let area = ed.view.terminal_area();
            let total = ed.state.input.popup()?.docked_total_rows(area.width);
            let visible = hume_ui::popup::band_visible_rows(total, area.height);
            Some((visible, total))
        }
    }
}

/// A boxed popup's visible content height and total row count, read from
/// its resolved state.
pub(super) fn boxed_popup_rows(state: &hume_ui::popup::PopupState) -> (usize, usize) {
    (state.inner_height(), state.total_rows)
}

/// Whether `key` pages by half a window: `Some(true)` for Ctrl-d,
/// `Some(false)` for Ctrl-u, `None` for any other key. Every layer that
/// pages (a scrollable box, the drawer, the picker) decodes them here.
pub(super) fn half_page_key(key: &KeyEvent) -> Option<bool> {
    if !key.modifiers.contains(Modifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char('d') => Some(true),
        KeyCode::Char('u') => Some(false),
        _ => None,
    }
}

/// Pages a scrollable text box (a hover popup, the completion documentation
/// box) half its visible height, down or up, within what `rows` (visible
/// height, total row count) leaves scrollable. `false` when `rows` has none
/// to give, when the box shows no rows (a terminal with no size, or too
/// short for the box), or when the content fits, so the key falls through.
///
/// `scroll` is the model value, clamped for rendering only in the
/// per-frame view sync, so it can be stale-large after the content
/// shrinks (the terminal grows between key presses). It is clamped before
/// the step, not after.
pub(super) fn scroll_box(scroll: &mut usize, down: bool, rows: Option<(usize, usize)>) -> bool {
    let Some((inner_h, total)) = rows else {
        return false;
    };
    let max_scroll = total.saturating_sub(inner_h);
    if inner_h == 0 || max_scroll == 0 {
        return false;
    }
    let half = half_page(inner_h);
    let clamped = (*scroll).min(max_scroll);
    *scroll = if down {
        (clamped + half).min(max_scroll)
    } else {
        clamped.saturating_sub(half)
    };
    true
}
