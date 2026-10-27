//! Opaque keys for the buffers and panes the editor owns.

use slotmap::new_key_type;

new_key_type! {
    /// Opaque handle to a buffer.
    pub struct BufferId;
    /// Opaque handle to a pane.
    pub struct PaneId;
}
