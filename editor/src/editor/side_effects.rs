/// Side-effects returned by an [`EditorCmd`] handler.
///
/// Currently a unit struct — all hook firing goes through
/// `EditorState::pending_hooks`, drained by `Editor::drain_hooks` after
/// each command. Reserved for future extensions.
///
/// [`EditorCmd`]: crate::editor::registry::MappableCommand::EditorCmd
pub(crate) struct SideEffects;

impl SideEffects {
    pub(crate) fn none() -> Self {
        Self
    }
}
