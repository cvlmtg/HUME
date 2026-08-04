//! A reusable native yes/no confirmation overlay.
//!
//! Unlike [`super::popup::MenuModel`] (a Steel-facing `(show-menu! …)`
//! primitive with a `SteelVal` callback), a confirm is Rust-native: its
//! action is a plain enum matched inline, with no closure capturing
//! `&mut Editor` and no round-trip through the scripting VM. It exists for
//! editor-internal yes/no questions — disk-change reload is the first one;
//! see `docs/ROADMAP.md`'s binary/huge-file `:e` confirm for the next.

use hume_engine::pipeline::BufferId;

/// One key the user can press while a [`ConfirmModel`] is open, and its
/// display label (e.g. `"reload"` for key `'r'`).
pub(crate) struct ConfirmChoice {
    pub(crate) key: char,
    pub(crate) label: &'static str,
}

/// What happens when the user accepts the confirm (presses `choices[0].key`).
///
/// A plain enum, not a boxed closure: the handler is one `match` arm, and it
/// can't accidentally capture stale editor state. Add a variant per new
/// confirm use — see the module doc.
pub(crate) enum ConfirmAction {
    /// Reload this buffer from disk, discarding any in-editor edits.
    /// Undoable: `reload_buffer_in_place` records the reload as a single
    /// revision on top of the existing undo tree.
    ReloadBuffer(BufferId),
}

/// An open confirmation prompt, rendered in the statusline row.
///
/// `choices[0]` is the accept choice — pressing its key runs `action`.
/// Every other key, including `Esc` and every other listed choice, dismisses
/// without running `action`. There is currently never more than one
/// non-accept outcome ("keep" for the disk-change prompt), so this
/// intentionally doesn't model per-choice actions beyond the first.
pub(crate) struct ConfirmModel {
    pub(crate) prompt: String,
    pub(crate) choices: Vec<ConfirmChoice>,
    pub(crate) action: ConfirmAction,
}

impl ConfirmModel {
    /// Whether answering this confirm would act on `id` — i.e. whether `id`
    /// disappearing leaves the question unanswerable. Read by
    /// `buffer::lifecycle::close_buffer_and_notify`, which retires such a
    /// confirm rather than leaving one on screen whose only possible outcome
    /// is a silent no-op. A `match`, not a `matches!`, so the next `action`
    /// variant this module gains is forced to decide here rather than
    /// defaulting to "unaffected".
    pub(crate) fn targets_buffer(&self, id: BufferId) -> bool {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
