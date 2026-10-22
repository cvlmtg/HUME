//! External file-change detection (`Editor::check_buffer_disk_state` /
//! `check_all_disk_state`, the `file-reload-prompt` setting, the `:w` stale guard, and
//! the confirm overlay's reload/keep choices).

use super::*;
use crate::editor::buffer::{DiskCheckTrigger, DiskState};
use hume_grid::Rect;
use hume_platform::dirs::Dirs;

/// Overwrite `path`'s content with something a different length than
/// whatever's there now, so the on-disk signature differs by size alone,
/// robust against a filesystem (HFS+, FAT) whose mtime only has one-second
/// resolution, where two writes in the same test could otherwise land on an
/// identical mtime.
fn rewrite_externally(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).unwrap();
}

/// Two panes side by side: the left pane keeps viewing A, the right
/// (focused) pane is retargeted onto B. `:vsplit` with no argument inherits
/// the source pane's buffer, so `:e <tmp_b>` on the new pane is what actually
/// splits the two buffers apart.
fn two_panes_with_b_focused() -> (
    Editor,
    tempfile::TempPath,
    tempfile::TempPath,
    BufferId,
    BufferId,
) {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    type_cmd(&mut ed, ":vsplit");
    let (tmp_b, tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");
    (ed, tmp_a, tmp_b_guard, bid_a, bid_b)
}

mod buffer_enter_reveal;
mod confirm_hygiene;
mod detection_and_choices;
mod focus_and_inline_output;
mod overlay_stacking;
mod write_guard_and_checktime;
