use crate::editor::commands::*;
use crate::editor::registry::CommandRegistry;

use super::builder::ecmd;

impl CommandRegistry {
    pub(super) fn register_editor_cmds(&mut self) {
        // ── Editor commands — mode transitions ────────────────────────────────
        ecmd(
            "insert-before",
            "Enter insert mode; collapse each selection to its start.",
            cmd_insert_before,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "insert-after",
            "Enter insert mode after the cursor (move one grapheme right).",
            cmd_insert_after,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "insert-at-line-start",
            "Enter insert mode at the first non-blank character on the line.",
            cmd_insert_at_line_start,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "insert-at-line-end",
            "Enter insert mode after the last character on the line.",
            cmd_insert_at_line_end,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "insert-at-selection-start",
            "Enter insert mode at the start of the selection.",
            cmd_insert_at_selection_start,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "insert-at-selection-end",
            "Enter insert mode after the end of the selection.",
            cmd_insert_at_selection_end,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "open-line-below",
            "Open a new line below the cursor and enter insert mode.",
            cmd_open_line_below,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "open-line-above",
            "Open a new line above the cursor and enter insert mode.",
            cmd_open_line_above,
        )
        .repeatable()
        .reg(self);
        ecmd(
            "command-mode",
            "Open the command-mode mini-buffer.",
            cmd_command_mode,
        )
        .reg(self);
        ecmd(
            "exit-insert",
            "Return to normal mode from insert mode.",
            cmd_exit_insert,
        )
        .reg(self);

        // ── Editor commands — edit composites ─────────────────────────────────
        ecmd(
            "delete",
            "Delete selections, pushing their text onto the kill ring.",
            cmd_delete,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
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
        ecmd(
            "select-last-insertion",
            "Select the text typed during the most recently completed insert session.",
            cmd_select_last_insertion,
        )
        .extendable()
        .reg(self);
        ecmd(
            "yank",
            "Copy selections to the clipboard and kill ring without deleting.",
            cmd_yank,
        )
        .reg(self);
        ecmd(
            "paste-after",
            "Paste register contents after the selection. Bare (no \"<reg> prefix) reads the kill-ring head, with no clipboard fallback.",
            cmd_paste_after,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-before",
            "Paste register contents before the selection. Bare (no \"<reg> prefix) reads the kill-ring head, with no clipboard fallback.",
            cmd_paste_before,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "smart-paste-after",
            "Paste after the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise.",
            cmd_smart_paste_after,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "smart-paste-before",
            "Paste before the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise.",
            cmd_smart_paste_before,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-ring-older",
            "Cycle kill ring one step older and re-paste.",
            cmd_paste_ring_older,
        )
        .defers_paste_commit()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-ring-newer",
            "Cycle kill ring one step newer and re-paste.",
            cmd_paste_ring_newer,
        )
        .defers_paste_commit()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "join-lines-select-spaces",
            "Join lines inside each selection and select the inserted spaces.",
            cmd_join_lines_select_spaces,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "align-selections",
            "Align each selection's anchor to the primary selection's anchor column.",
            cmd_align_selections,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "indent",
            "Indent every line touched by a selection by one level.",
            cmd_indent,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "unindent",
            "Unindent every line touched by a selection by one level.",
            cmd_unindent,
        )
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd("undo", "Undo the last change.", cmd_undo).reg(self);
        ecmd("redo", "Redo the last undone change.", cmd_redo).reg(self);

        // ── Editor commands — selection state ────────────────────────────────
        ecmd(
            "toggle-extend",
            "Toggle sticky extend mode.",
            cmd_toggle_extend,
        )
        .reg(self);
        ecmd(
            "collapse-and-exit-extend",
            "Collapse each selection to its cursor and exit extend mode.",
            cmd_collapse_to_head_and_exit_extend,
        )
        .reg(self);
        ecmd(
            "collapse-to-anchor-and-exit-extend",
            "Collapse each selection to its anchor and exit extend mode.",
            cmd_collapse_to_anchor_and_exit_extend,
        )
        .reg(self);

        // ── Editor commands — find / till (read pending_char) ─────────────────
        ecmd(
            "find-forward",
            "Find next occurrence of a character (inclusive, forward).",
            cmd_find_forward,
        )
        .extendable()
        .reg(self);
        ecmd(
            "find-backward",
            "Find previous occurrence of a character (inclusive, backward).",
            cmd_find_backward,
        )
        .extendable()
        .reg(self);
        ecmd(
            "till-forward",
            "Move to just before next occurrence of a character (exclusive).",
            cmd_till_forward,
        )
        .extendable()
        .reg(self);
        ecmd(
            "till-backward",
            "Move to just after previous occurrence of a character (exclusive).",
            cmd_till_backward,
        )
        .extendable()
        .reg(self);
        ecmd(
            "repeat-find-forward",
            "Repeat the last find/till motion forward.",
            cmd_repeat_find_forward,
        )
        .extendable()
        .reg(self);
        ecmd(
            "repeat-find-backward",
            "Repeat the last find/till motion backward.",
            cmd_repeat_find_backward,
        )
        .extendable()
        .reg(self);

        // ── Editor commands — replace (reads pending_char) ───────────────────
        ecmd(
            "replace",
            "Replace every character in each selection with the next typed character.",
            cmd_replace,
        )
        .repeatable()
        .clears_extend()
        .reg(self);

        // ── Editor commands — page scroll ─────────────────────────────────────
        ecmd(
            "page-down",
            "Scroll down by one viewport height.",
            cmd_page_down,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd("page-up", "Scroll up by one viewport height.", cmd_page_up)
            .extendable()
            .jump()
            .reg(self);

        // ── Editor commands — half-page scroll ────────────────────────────────
        ecmd(
            "half-page-down",
            "Scroll down by half a viewport height.",
            cmd_half_page_down,
        )
        .extendable()
        .reg(self);
        ecmd(
            "half-page-up",
            "Scroll up by half a viewport height.",
            cmd_half_page_up,
        )
        .extendable()
        .reg(self);

        // ── Editor commands — view-trie scroll (zz / zt / zb) ─────────────────
        // Reposition the viewport without moving the cursor.
        ecmd(
            "center-view-on-cursor",
            "Scroll so the primary selection head sits at the vertical center of the viewport.",
            cmd_view_center,
        )
        .reg(self);
        ecmd(
            "top-view-on-cursor",
            "Scroll so the primary selection head sits at the top of the viewport.",
            cmd_view_top,
        )
        .reg(self);
        ecmd(
            "bottom-view-on-cursor",
            "Scroll so the primary selection head sits at the bottom of the viewport.",
            cmd_view_bottom,
        )
        .reg(self);

        // ── Editor commands — repeat ──────────────────────────────────────────
        // Not flagged repeatable: `.` repeating itself would be nonsensical.
        // The handler sets EditorState::pending_repeat; replay_dot does
        // the actual replay with &mut Editor after handle_key returns — the
        // handler itself still takes only EditorCmdFn's shape, no &mut Editor.
        //
        // `.defers_paste_commit()`: this dispatch itself must not commit a
        // paste session left open by a preceding `[`/`]` — replay_dot makes
        // that call once it knows which command is being replayed (see its
        // own `defers_paste_commit` builder doc for why).
        ecmd(
            "repeat-last-action",
            "Repeat the last editing action.",
            cmd_repeat,
        )
        .defers_paste_commit()
        .reg(self);

        // ── Editor commands — search ──────────────────────────────────────────
        ecmd(
            "search-forward",
            "Enter search mode (forward).",
            cmd_search_forward,
        )
        .reg(self);
        ecmd(
            "search-backward",
            "Enter search mode (backward).",
            cmd_search_backward,
        )
        .reg(self);
        ecmd(
            "search-next",
            "Jump to the next search match.",
            cmd_search_next,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd(
            "search-prev",
            "Jump to the previous search match.",
            cmd_search_prev,
        )
        .extendable()
        .jump()
        .reg(self);
        ecmd(
            "clear-search",
            "Clear search highlights (`:clear-search`).",
            cmd_clear_search,
        )
        .reg(self);

        // ── Editor commands — select ─────────────────────────────────────────
        ecmd(
            "select-within",
            "Select regex matches within current selections.",
            cmd_select_within,
        )
        .reg(self);
        // `EditorCmd`, not `selection!`: the body needs `EditorState` to read
        // the buffer's search pattern, a channel `Selection`'s pure
        // `fn(&BufferText, SelectionSet, ...)` signature has no room for.
        // `.establishes_selection()` opts it into the dot-repeat recipe
        // anyway — its whole-buffer result is safe to replay from any cursor.
        ecmd(
            "select-all-matches",
            "Turn every search match in the buffer into a selection.",
            cmd_select_all_matches,
        )
        .establishes_selection()
        .reg(self);
        ecmd(
            "search-word-under-cursor",
            "Search the whole word under the cursor.",
            cmd_search_word_under_cursor,
        )
        .reg(self);
        ecmd(
            "search-selection",
            "Use the primary selection text literally as the search pattern.",
            cmd_search_selection,
        )
        .reg(self);

        // ── Editor commands — jump list ──────────────────────────────────────
        ecmd(
            "jump-backward",
            "Navigate to the previous position in the jump list.",
            cmd_jump_backward,
        )
        .reg(self);
        ecmd(
            "jump-forward",
            "Navigate to the next position in the jump list.",
            cmd_jump_forward,
        )
        .reg(self);
        ecmd(
            "goto-alternate-file",
            "Switch to the most-recently-focused other buffer.",
            cmd_goto_alternate_file,
        )
        .jump()
        .reg(self);

        // ── Editor commands — misc ────────────────────────────────────────────
        ecmd(
            "force-quit",
            "Quit the whole editor unconditionally, discarding unsaved changes in every buffer (same as :qa!).",
            cmd_quit,
        )
        .reg(self);

        // ── Editor commands — pane focus stubs ────────────────────────────────
        ecmd(
            "pane-focus-next",
            "Focus the next pane.",
            cmd_pane_focus_next,
        )
        .reg(self);
        ecmd(
            "pane-focus-left",
            "Focus the pane to the left.",
            cmd_pane_focus_left,
        )
        .reg(self);
        ecmd(
            "pane-focus-right",
            "Focus the pane to the right.",
            cmd_pane_focus_right,
        )
        .reg(self);
        ecmd("pane-focus-up", "Focus the pane above.", cmd_pane_focus_up).reg(self);
        ecmd(
            "pane-focus-down",
            "Focus the pane below.",
            cmd_pane_focus_down,
        )
        .reg(self);
        ecmd(
            "pane-split",
            "Split the focused pane, stacking the new pane below it.",
            cmd_split_pane,
        )
        .reg(self);
        ecmd(
            "pane-vsplit",
            "Split the focused pane side by side.",
            cmd_vsplit_pane,
        )
        .reg(self);
        ecmd("pane-close", "Close the focused pane.", cmd_close_pane).reg(self);
    }
}
