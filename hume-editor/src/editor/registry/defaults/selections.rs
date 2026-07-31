use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, MappableCommand};
use crate::ops::selection_cmd::{
    cmd_collapse_selection_to_head, cmd_copy_selection_on_next_line,
    cmd_copy_selection_on_prev_line, cmd_cycle_primary_backward, cmd_cycle_primary_forward,
    cmd_flip_selections, cmd_keep_primary_selection, cmd_remove_primary_selection, cmd_select_all,
    cmd_split_selection_on_newlines, cmd_trim_selection_whitespace,
};

impl CommandRegistry {
    pub(super) fn register_selections(&mut self) {
        // ── Selection commands ────────────────────────────────────────────────
        super::selection!(
            self,
            "collapse-selection",
            "Collapse each selection to a single cursor at the head.",
            cmd_collapse_selection_to_head
        );
        super::selection!(
            self,
            "flip-selections",
            "Swap anchor and head for each selection.",
            cmd_flip_selections
        );
        super::selection!(
            self,
            "keep-primary-selection",
            "Remove all selections except the primary.",
            cmd_keep_primary_selection
        );
        super::selection!(
            self,
            "select-all",
            "Select the entire buffer.",
            cmd_select_all,
            jump
        );
        super::selection!(
            self,
            "remove-primary-selection",
            "Remove the primary selection, promoting the next.",
            cmd_remove_primary_selection
        );
        super::selection!(
            self,
            "cycle-primary-forward",
            "Cycle the primary selection forward.",
            cmd_cycle_primary_forward
        );
        super::selection!(
            self,
            "cycle-primary-backward",
            "Cycle the primary selection backward.",
            cmd_cycle_primary_backward
        );
        super::selection!(
            self,
            "split-selection-on-newlines",
            "Split each multi-line selection into one per line.",
            cmd_split_selection_on_newlines
        );
        super::selection!(
            self,
            "trim-selection-whitespace",
            "Trim leading and trailing whitespace from each selection.",
            cmd_trim_selection_whitespace
        );
        super::selection!(
            self,
            "copy-selection-on-next-line",
            "Duplicate each selection on the line below.",
            cmd_copy_selection_on_next_line
        );
        super::selection!(
            self,
            "copy-selection-on-prev-line",
            "Duplicate each selection on the line above.",
            cmd_copy_selection_on_prev_line
        );
    }
}
