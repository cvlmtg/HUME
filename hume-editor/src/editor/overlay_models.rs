//! The raw, not-yet-resolved model behind each transient-chrome overlay
//! (popup, menu, drawer) and the native confirm prompt — `ConfigState`'s
//! input-side state, held until the next frame's `overlay_sync` resolves it
//! into a positioned `hume_ui` view state (or, for the confirm prompt,
//! which has no separate view type, rendered directly by the statusline).
//!
//! Grouped here because they're one conceptual kind: editor-owned input
//! state (a not-yet-fired Steel callback, a `BufferId` to act on), not a
//! `hume_ui` value object or provider reading from a handle it was given —
//! mirroring [`super::picker::PickerSession`], the picker's own analogous
//! model, which lives here for the same reason.

use hume_engine::pipeline::BufferId;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

/// `(show-popup! text)`'s raw, unwrapped content — held on `EditorState`
/// until the next frame's `Editor::sync_popup_view`/`sync_popup_band_view`
/// (per `layout`) resolves it into a positioned
/// `hume_ui::popup::PopupState` or a `PopupBandState`.
pub(in crate::editor) struct PopupModel {
    pub(in crate::editor) text: String,
    pub(in crate::editor) kind: hume_scripting::host::PopupKind,
    /// First visible wrapped row, for a `Scrollable` popup. Clamped against
    /// `max_scroll` in `Editor::scroll_popup` before each delta is applied,
    /// since content height (and so `max_scroll`) can shrink between key
    /// presses (e.g. the terminal grows) without this field being touched.
    /// `Editor::sync_popup_view`/`sync_popup_band_view` additionally clamp a
    /// *copy* of this value for rendering each frame — that clamp is
    /// view-only and never writes back to the model.
    pub(in crate::editor) scroll: usize,
    /// `#:lang` — rebuilt fresh on every `show-popup!`, dropped with the
    /// popup on close. No separate invalidation path: the popup's lifetime
    /// IS the syntax's (SSOT).
    pub(in crate::editor) syntax: Option<super::popup_syntax::MarkupSyntax>,
    pub(in crate::editor) layout: hume_ui::popup::PopupLayout,
    /// Lazily built from `text`/`syntax` on first [`Self::content_mut`]
    /// call — not eagerly at `show-popup!` time, because a `#:lang` popup's
    /// styled runs need a *baked* theme (`Theme::resolve` debug-asserts on
    /// an unbaked `ScopeId`, and the theme isn't baked yet when `show_popup`
    /// runs — see `frame.rs`'s `prepare_frame` step ordering). Its own
    /// internal wrap cache is then keyed by width, so an unchanged width
    /// across frames is O(1), not a re-wrap. Reset to `None` wherever
    /// `view.theme` is replaced (`settings::ops`'s `ResyncKey::theme` arm
    /// and `reset_globals`) — the baked styles are the one input here that
    /// *can* change out from under an open popup, since `text`/`syntax`
    /// don't.
    pub(in crate::editor) content: Option<hume_ui::popup::PopupContent>,
}

impl PopupModel {
    /// [`Self::content`], building it on first call. `text`/`syntax` never
    /// change during a `PopupModel`'s lifetime (a new `show-popup!` builds a
    /// fresh one), so building it once and caching it here is sound — the
    /// theme can still change under it, which is why [`Self::content`]'s own
    /// doc names the two sites that reset this back to `None` when it does.
    pub(in crate::editor) fn content_mut(
        &mut self,
        theme: &Theme,
    ) -> &mut hume_ui::popup::PopupContent {
        if self.content.is_none() {
            self.content = Some(match self.syntax.as_ref() {
                Some(syntax) => {
                    let base_style = theme.resolve_by_name(Scope("ui.popup"));
                    let runs = syntax.styled_runs(&self.text, theme, base_style);
                    hume_ui::popup::PopupContent::styled(runs)
                }
                None => hume_ui::popup::PopupContent::plain(&self.text),
            });
        }
        self.content.as_mut().expect("populated above")
    }
}

/// `(show-menu! items on-select)`'s raw content — held on `EditorState`
/// until the next frame's `Editor::sync_menu_view` resolves it into a
/// positioned `hume_ui::popup::PopupState` with `selected` set. `callback`
/// fires exactly once (one per selection or dismissal), then the whole
/// model is dropped.
pub(in crate::editor) struct MenuModel {
    pub(in crate::editor) items: std::sync::Arc<Vec<String>>,
    pub(in crate::editor) selected: usize,
    pub(in crate::editor) callback: steel::rvals::SteelVal,
}

/// `(show-drawer-list! items on-select)`'s raw state, including the
/// not-yet-exhausted Steel callback — cleared by `Esc` or `close-drawer!`,
/// not by `Enter` (the drawer stays open across selections).
pub(in crate::editor) struct DrawerModel {
    /// Shared with `hume_ui::drawer::DrawerViewState::rows` (an
    /// `Arc::clone`, not a deep copy) — `sync_drawer_view` runs
    /// unconditionally every frame while the drawer is open, and a
    /// references batch can carry thousands of rows.
    pub(in crate::editor) items: std::sync::Arc<Vec<String>>,
    pub(in crate::editor) selected: usize,
    /// Index of the first visible row — clamped to keep `selected` in view
    /// whenever the selection moves (`Editor::clamp_drawer_scroll`).
    pub(in crate::editor) scroll: usize,
    pub(in crate::editor) callback: steel::rvals::SteelVal,
}

/// One key the user can press while a [`ConfirmModel`] is open, and its
/// display label (e.g. `"reload"` for key `'r'`).
pub(in crate::editor) struct ConfirmChoice {
    pub(in crate::editor) key: char,
    pub(in crate::editor) label: &'static str,
}

/// What happens when the user accepts the confirm (presses `choices[0].key`).
///
/// A plain enum, not a boxed closure: the handler is one `match` arm, and it
/// can't accidentally capture stale editor state. Add a variant per new
/// confirm use — see [`ConfirmModel`]'s doc.
pub(in crate::editor) enum ConfirmAction {
    /// Reload this buffer from disk, discarding any in-editor edits.
    /// Undoable: `reload_buffer_in_place` records the reload as a single
    /// revision on top of the existing undo tree.
    ReloadBuffer(BufferId),
}

/// A reusable native yes/no confirmation overlay, rendered in the
/// statusline row.
///
/// Unlike [`MenuModel`] (a Steel-facing `(show-menu! …)` primitive with a
/// `SteelVal` callback), a confirm is Rust-native: its action is a plain
/// enum matched inline, with no closure capturing `&mut Editor` and no
/// round-trip through the scripting VM. It exists for editor-internal
/// yes/no questions — disk-change reload is the first one.
///
/// `choices[0]` is the accept choice — pressing its key runs `action`.
/// Every other key, including `Esc` and every other listed choice, dismisses
/// without running `action`. There is currently never more than one
/// non-accept outcome ("keep" for the disk-change prompt), so this
/// intentionally doesn't model per-choice actions beyond the first. No
/// separate view type: [`ConfirmModel::render_line`] is painted directly by
/// `hume-editor`'s statusline — `pub(crate)`, not `pub(in crate::editor)`
/// like every other type in this module, since the statusline lives at
/// `crate::statusline`, a sibling of `crate::editor` rather than a
/// descendant of it.
pub(crate) struct ConfirmModel {
    pub(in crate::editor) prompt: String,
    pub(in crate::editor) choices: Vec<ConfirmChoice>,
    pub(in crate::editor) action: ConfirmAction,
}

impl ConfirmModel {
    /// Whether answering this confirm would act on `id` — i.e. whether `id`
    /// disappearing leaves the question unanswerable. Read by
    /// `buffer::lifecycle::close_buffer_and_notify`, which retires such a
    /// confirm rather than leaving one on screen whose only possible outcome
    /// is a silent no-op. A `match`, not a `matches!`, so the next `action`
    /// variant this module gains is forced to decide here rather than
    /// defaulting to "unaffected".
    pub(in crate::editor) fn targets_buffer(&self, id: BufferId) -> bool {
        match self.action {
            ConfirmAction::ReloadBuffer(bid) => bid == id,
        }
    }

    /// The line to paint in the statusline row: prompt text followed by
    /// each choice as `[key]label`.
    pub(crate) fn render_line(&self) -> String {
        let mut out = self.prompt.clone();
        for choice in &self.choices {
            out.push_str("  [");
            out.push(choice.key);
            out.push(']');
            out.push_str(choice.label);
        }
        out
    }
}

#[cfg(test)]
mod tests;
