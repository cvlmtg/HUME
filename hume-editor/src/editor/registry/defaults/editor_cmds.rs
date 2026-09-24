use crate::editor::commands::*;
use crate::editor::registry::CommandRegistry;

use super::builder::{ecmd_buffer, ecmd_focused, ecmd_global, ecmd_pane};

impl CommandRegistry {
    pub(super) fn register_editor_cmds(&mut self) {
        // ── Editor commands — mode transitions ────────────────────────────────
        ecmd_focused(
            "insert-before",
            "Enter insert mode; collapse each selection to its start.",
            cmd_insert_before,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "insert-after",
            "Enter insert mode after the cursor (move one grapheme right).",
            cmd_insert_after,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "insert-at-line-start",
            "Enter insert mode at the first non-blank character on the line.",
            cmd_insert_at_line_start,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "insert-at-line-end",
            "Enter insert mode after the last character on the line.",
            cmd_insert_at_line_end,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "insert-at-selection-start",
            "Enter insert mode at the start of the selection.",
            cmd_insert_at_selection_start,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "insert-at-selection-end",
            "Enter insert mode after the end of the selection.",
            cmd_insert_at_selection_end,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "open-line-below",
            "Open a new line below the cursor and enter insert mode.",
            cmd_open_line_below,
        )
        .repeatable()
        .reg(self);
        ecmd_focused(
            "open-line-above",
            "Open a new line above the cursor and enter insert mode.",
            cmd_open_line_above,
        )
        .repeatable()
        .reg(self);
        ecmd_global(
            "command-mode",
            "Open the command-mode mini-buffer.",
            cmd_command_mode,
        )
        .reg(self);
        ecmd_focused(
            "exit-insert",
            "Return to normal mode from insert mode.",
            cmd_exit_insert,
        )
        .reg(self);
        ecmd_focused(
            "completion-trigger",
            "Show completions at the cursor (Insert mode).",
            cmd_completion_trigger,
        )
        .reg(self);

        // ── Editor commands — edit composites ─────────────────────────────────
        ecmd_pane(
            "delete",
            "Delete selections, pushing their text onto the kill ring.",
            cmd_delete,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "change",
            "Delete selections onto the kill ring, then enter insert mode (one undo group).",
            cmd_change,
        )
        .repeatable()
        .reg(self);
        // Bound at `mii`. An `EditorCmd`, not a `Selection`, because it reads
        // buffer state (`Buffer::last_insert`) beyond the current `BufferText` +
        // `SelectionSet` — no `around` counterpart; see the doc comment on
        // `cmd_select_last_insertion` itself.
        ecmd_pane(
            "select-last-insertion",
            "Select the text typed during the most recently completed insert session.",
            cmd_select_last_insertion,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "yank",
            "Copy selections to the clipboard and kill ring without deleting.",
            cmd_yank,
        )
        .reg(self);
        ecmd_focused(
            "paste-after",
            "Paste register contents after the selection. Bare (no \"<reg> prefix) reads the kill-ring head, with no clipboard fallback.",
            cmd_paste_after,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "paste-before",
            "Paste register contents before the selection. Bare (no \"<reg> prefix) reads the kill-ring head, with no clipboard fallback.",
            cmd_paste_before,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "smart-paste-after",
            "Paste after the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise.",
            cmd_smart_paste_after,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "smart-paste-before",
            "Paste before the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise.",
            cmd_smart_paste_before,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "paste-ring-older",
            "Cycle kill ring one step older and re-paste.",
            cmd_paste_ring_older,
        )
        .defers_paste_commit()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_focused(
            "paste-ring-newer",
            "Cycle kill ring one step newer and re-paste.",
            cmd_paste_ring_newer,
        )
        .defers_paste_commit()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_pane(
            "join-lines-select-spaces",
            "Join lines inside each selection and select the inserted spaces.",
            cmd_join_lines_select_spaces,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_pane(
            "align-selections",
            "Align each selection's anchor to the primary selection's anchor column.",
            cmd_align_selections,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_pane(
            "indent",
            "Indent every line touched by a selection by one level.",
            cmd_indent,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_pane(
            "unindent",
            "Unindent every line touched by a selection by one level.",
            cmd_unindent,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd_pane("undo", "Undo the last change.", cmd_undo).reg(self);
        ecmd_pane("redo", "Redo the last undone change.", cmd_redo).reg(self);

        // ── Editor commands — selection state ────────────────────────────────
        ecmd_global(
            "toggle-extend",
            "Toggle sticky extend mode.",
            cmd_toggle_extend,
        )
        .reg(self);
        ecmd_focused(
            "collapse-and-exit-extend",
            "Collapse each selection to its cursor and exit extend mode.",
            cmd_collapse_to_head_and_exit_extend,
        )
        .reg(self);
        ecmd_focused(
            "collapse-to-anchor-and-exit-extend",
            "Collapse each selection to its anchor and exit extend mode.",
            cmd_collapse_to_anchor_and_exit_extend,
        )
        .reg(self);

        // ── Editor commands — find / till (read pending_char) ─────────────────
        ecmd_pane(
            "find-forward",
            "Find next occurrence of a character (inclusive, forward).",
            cmd_find_forward,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "find-backward",
            "Find previous occurrence of a character (inclusive, backward).",
            cmd_find_backward,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "till-forward",
            "Move to just before next occurrence of a character (exclusive).",
            cmd_till_forward,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "till-backward",
            "Move to just after previous occurrence of a character (exclusive).",
            cmd_till_backward,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "repeat-find-forward",
            "Repeat the last find/till motion forward.",
            cmd_repeat_find_forward,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "repeat-find-backward",
            "Repeat the last find/till motion backward.",
            cmd_repeat_find_backward,
        )
        .extendable()
        .reg(self);

        // ── Editor commands — replace (reads pending_char) ───────────────────
        ecmd_pane(
            "replace",
            "Replace every character in each selection with the next typed character.",
            cmd_replace,
        )
        .repeatable()
        .clears_extend()
        .reg(self);

        // ── Editor commands — page scroll ─────────────────────────────────────
        ecmd_pane(
            "page-down",
            "Scroll down by one viewport height.",
            cmd_page_down,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd_pane("page-up", "Scroll up by one viewport height.", cmd_page_up)
            .extendable()
            .jump()
            .reg(self);

        // ── Editor commands — half-page scroll ────────────────────────────────
        ecmd_pane(
            "half-page-down",
            "Scroll down by half a viewport height.",
            cmd_half_page_down,
        )
        .extendable()
        .reg(self);
        ecmd_pane(
            "half-page-up",
            "Scroll up by half a viewport height.",
            cmd_half_page_up,
        )
        .extendable()
        .reg(self);

        // ── Editor commands — view-trie scroll (z z / z k / z j) ──────────────
        // Reposition the viewport without moving the cursor.
        ecmd_pane(
            "center-view-on-cursor",
            "Scroll so the primary selection head sits at the vertical center of the viewport.",
            cmd_view_center,
        )
        .reg(self);
        ecmd_pane(
            "top-view-on-cursor",
            "Scroll so the primary selection head sits at the top of the viewport.",
            cmd_view_top,
        )
        .reg(self);
        ecmd_pane(
            "bottom-view-on-cursor",
            "Scroll so the primary selection head sits at the bottom of the viewport.",
            cmd_view_bottom,
        )
        .reg(self);

        // ── Editor commands — repeat ──────────────────────────────────────────
        // Not flagged repeatable: `.` repeating itself would be nonsensical.
        // The handler sets EditorState::pending_repeat; replay_dot does
        // the actual replay with &mut Editor after handle_key returns — the
        // handler itself still takes only a native EditorCmd's shape, no &mut Editor.
        //
        // `.defers_paste_commit()`: this dispatch itself must not commit a
        // paste session left open by a preceding `[`/`]` — replay_dot makes
        // that call once it knows which command is being replayed (see its
        // own `defers_paste_commit` builder doc for why).
        ecmd_focused(
            "repeat-last-action",
            "Repeat the last editing action.",
            cmd_repeat,
        )
        .defers_paste_commit()
        .reg(self);

        // ── Editor commands — search ──────────────────────────────────────────
        ecmd_focused(
            "search-forward",
            "Enter search mode (forward).",
            cmd_search_forward,
        )
        .reg(self);
        ecmd_focused(
            "search-backward",
            "Enter search mode (backward).",
            cmd_search_backward,
        )
        .reg(self);
        ecmd_pane(
            "search-next",
            "Jump to the next search match.",
            cmd_search_next,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd_pane(
            "search-prev",
            "Jump to the previous search match.",
            cmd_search_prev,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd_buffer(
            "clear-search",
            "Clear search highlights (`:clear-search`).",
            cmd_clear_search,
        )
        .reg(self);

        // ── Editor commands — sift ────────────────────────────────────────────
        ecmd_focused(
            "sift-within",
            "Sift each selection down to the regex matches inside it.",
            cmd_sift_within,
        )
        .reg(self);
        // `EditorCmd`, not `selection!`: the body needs `EditorState` to read
        // the buffer's search pattern, a channel `Selection`'s pure
        // `fn(&BufferText, SelectionSet, ...)` signature has no room for.
        // `.establishes_selection()` opts it into the dot-repeat recipe
        // anyway — its whole-buffer result is safe to replay from any cursor.
        ecmd_pane(
            "select-all-matches",
            "Turn every search match in the buffer into a selection.",
            cmd_select_all_matches,
        )
        .establishes_selection()
        .reg(self);
        ecmd_pane(
            "search-word-under-cursor",
            "Search the whole word under the cursor.",
            cmd_search_word_under_cursor,
        )
        .reg(self);
        ecmd_pane(
            "search-selection",
            "Use the primary selection text literally as the search pattern.",
            cmd_search_selection,
        )
        .reg(self);

        // ── Editor commands — jump list ──────────────────────────────────────
        ecmd_pane(
            "jump-backward",
            "Navigate to the previous position in the jump list.",
            cmd_jump_backward,
        )
        .reg(self);
        ecmd_pane(
            "jump-forward",
            "Navigate to the next position in the jump list.",
            cmd_jump_forward,
        )
        .reg(self);
        ecmd_pane(
            "goto-alternate-buffer",
            "Switch to the most-recently-focused other buffer.",
            cmd_goto_alternate_buffer,
        )
        .jump()
        .reg(self);
        ecmd_pane(
            "goto-next-buffer",
            "Switch to the next buffer in open-order.",
            cmd_goto_next_buffer,
        )
        .jump()
        .reg(self);
        ecmd_pane(
            "goto-prev-buffer",
            "Switch to the previous buffer in open-order.",
            cmd_goto_prev_buffer,
        )
        .jump()
        .reg(self);

        // ── Editor commands — tab pages ─────────────────────────────────────────
        // No `.jump()`: switching tabs changes `state.focus` itself (a
        // different pane, possibly in a different tab), same as the
        // pane-focus commands below — see `commands::tab`'s module doc for
        // why that disqualifies the jump-list recording `.jump()` triggers.
        ecmd_global(
            "goto-next-tab",
            "Switch to the next tab in display order.",
            cmd_goto_next_tab,
        )
        .reg(self);
        ecmd_global(
            "goto-prev-tab",
            "Switch to the previous tab in display order.",
            cmd_goto_prev_tab,
        )
        .reg(self);
        ecmd_buffer(
            "tab-new",
            "Open a fresh pane viewing the focused buffer in a new tab.",
            cmd_tab_new,
        )
        .reg(self);

        // ── Editor commands — pane focus stubs ────────────────────────────────
        ecmd_focused(
            "pane-focus-next",
            "Focus the next pane.",
            cmd_pane_focus_next,
        )
        .reg(self);
        ecmd_focused(
            "pane-focus-left",
            "Focus the pane to the left.",
            cmd_pane_focus_left,
        )
        .reg(self);
        ecmd_focused(
            "pane-focus-right",
            "Focus the pane to the right.",
            cmd_pane_focus_right,
        )
        .reg(self);
        ecmd_focused("pane-focus-up", "Focus the pane above.", cmd_pane_focus_up).reg(self);
        ecmd_focused(
            "pane-focus-down",
            "Focus the pane below.",
            cmd_pane_focus_down,
        )
        .reg(self);
        ecmd_focused(
            "pane-split",
            "Split the focused pane, stacking the new pane below it.",
            cmd_split_pane,
        )
        .reg(self);
        ecmd_focused(
            "pane-vsplit",
            "Split the focused pane side by side.",
            cmd_vsplit_pane,
        )
        .reg(self);
        ecmd_focused("pane-close", "Close the focused pane.", cmd_close_pane).reg(self);
    }
}
