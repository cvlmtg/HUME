use std::borrow::Cow;

use crate::ops::edit::{
    delete_char_backward, delete_char_forward, delete_selection, delete_word_backward,
};
use crate::ops::motion::{
    cmd_goto_first_line, cmd_goto_first_nonblank, cmd_goto_last_line, cmd_goto_line_end,
    cmd_goto_line_start, cmd_move_left, cmd_move_right, cmd_next_paragraph, cmd_prev_paragraph,
    cmd_select_line, cmd_select_line_backward, cmd_select_next_uppercase_word,
    cmd_select_next_word, cmd_select_prev_uppercase_word, cmd_select_prev_word,
};
use crate::ops::selection_cmd::{
    cmd_collapse_selection_to_head, cmd_copy_selection_on_next_line,
    cmd_copy_selection_on_prev_line, cmd_cycle_primary_backward, cmd_cycle_primary_forward,
    cmd_flip_selections, cmd_keep_primary_selection, cmd_remove_primary_selection, cmd_select_all,
    cmd_split_selection_on_newlines, cmd_trim_selection_whitespace,
};
use crate::ops::surround::{
    cmd_surround_angle, cmd_surround_backtick, cmd_surround_brace, cmd_surround_bracket,
    cmd_surround_double_quote, cmd_surround_paren, cmd_surround_single_quote,
};
use crate::ops::text_object::{
    cmd_around_angle, cmd_around_argument, cmd_around_backtick, cmd_around_brace,
    cmd_around_bracket, cmd_around_double_quote, cmd_around_line, cmd_around_paren,
    cmd_around_single_quote, cmd_around_uppercase_word, cmd_around_word, cmd_inner_angle,
    cmd_inner_argument, cmd_inner_backtick, cmd_inner_brace, cmd_inner_bracket,
    cmd_inner_double_quote, cmd_inner_line, cmd_inner_paren, cmd_inner_single_quote,
    cmd_inner_uppercase_word, cmd_inner_word,
};

use super::{CommandRegistry, EditorCmdFn, MappableCommand, TypedCommand};

impl CommandRegistry {
    pub(super) fn register_defaults(&mut self) {
        // Local macros to cut down on struct-literal boilerplate.
        macro_rules! motion {
            ($name:literal, $doc:literal, $fun:expr, jump) => {
                self.register(MappableCommand::Motion {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    jump: true,
                    reaching: false,
                })
            };
            ($name:literal, $doc:literal, $fun:expr, reaching) => {
                self.register(MappableCommand::Motion {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    jump: false,
                    reaching: true,
                })
            };
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Motion {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    jump: false,
                    reaching: false,
                })
            };
        }
        macro_rules! selection {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Selection {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    jump: false,
                })
            };
            ($name:literal, $doc:literal, $fun:expr, jump) => {
                self.register(MappableCommand::Selection {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    jump: true,
                })
            };
        }
        macro_rules! edit {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Edit {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    repeatable: false,
                })
            };
            ($name:literal, $doc:literal, $fun:expr, repeatable) => {
                self.register(MappableCommand::Edit {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    fun: $fun,
                    repeatable: true,
                })
            };
        }

        // Builder for EditorCmd registration. Each flag method sets one bool;
        // .reg(registry) terminates the chain. Adding a new flag costs one
        // method — existing call sites are unaffected.
        struct EditorCmdBuilder {
            name: &'static str,
            doc: &'static str,
            fun: EditorCmdFn,
            is_paste: bool,
            defers_paste_commit: bool,
            repeatable: bool,
            jump: bool,
            visual_move: bool,
            extendable: bool,
            stamps_last_command: bool,
            clears_extend: bool,
        }
        impl EditorCmdBuilder {
            fn repeatable(mut self) -> Self {
                self.repeatable = true;
                self
            }
            fn jump(mut self) -> Self {
                self.jump = true;
                self
            }
            fn visual_move(mut self) -> Self {
                self.visual_move = true;
                self
            }
            fn extendable(mut self) -> Self {
                self.extendable = true;
                self
            }
            /// Mark as a normal paste command (p / P) for paste-after detection.
            /// Does not suppress the paste-session commit — use `paste_cycle` for that.
            fn paste(mut self) -> Self {
                self.is_paste = true;
                self
            }
            /// Mark as a ring-cycle command ([ / ]). Suppresses paste-session
            /// commit so ring cycles fold into one undo step with the original paste.
            fn paste_cycle(mut self) -> Self {
                self.is_paste = true;
                self.defers_paste_commit = true;
                self
            }
            /// Mark this command as transparent to `last_command` (smart-p).
            /// Only `exit-insert` needs this — it closes the insert session a kill
            /// (`c`) opened, so stamping it would clobber the `"change"` marker.
            fn transparent_to_last_command(mut self) -> Self {
                self.stamps_last_command = false;
                self
            }
            /// Mark this as a selection-consuming edit that exits sticky Extend mode.
            /// Use for buffer-modifying acts: delete, paste, replace, surround-add.
            /// Do NOT use for yank, change, undo, redo, or mode-entry commands.
            fn clears_extend(mut self) -> Self {
                self.clears_extend = true;
                self
            }
            fn reg(self, r: &mut CommandRegistry) {
                r.register(MappableCommand::EditorCmd {
                    name: Cow::Borrowed(self.name),
                    doc: Cow::Borrowed(self.doc),
                    fun: self.fun,
                    is_paste: self.is_paste,
                    defers_paste_commit: self.defers_paste_commit,
                    repeatable: self.repeatable,
                    jump: self.jump,
                    visual_move: self.visual_move,
                    extendable: self.extendable,
                    stamps_last_command: self.stamps_last_command,
                    clears_extend: self.clears_extend,
                });
            }
        }
        // Construct a builder for an EditorCmd. All handlers share one shape:
        // fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>.
        let ecmd = |name: &'static str, doc: &'static str, fun: EditorCmdFn| EditorCmdBuilder {
            name,
            doc,
            fun,
            is_paste: false,
            defers_paste_commit: false,
            repeatable: false,
            jump: false,
            visual_move: false,
            extendable: false,
            stamps_last_command: true,
            clears_extend: false,
        };

        // ── Character motions ─────────────────────────────────────────────────
        motion!(
            "move-right",
            "Move cursors one grapheme to the right.",
            cmd_move_right
        );
        motion!(
            "move-left",
            "Move cursors one grapheme to the left.",
            cmd_move_left
        );
        ecmd(
            "move-down",
            "Move cursors down one visual line.",
            cmd_visual_move_down,
        )
        .extendable()
        .visual_move()
        .reg(self);
        ecmd(
            "move-up",
            "Move cursors up one visual line.",
            cmd_visual_move_up,
        )
        .extendable()
        .visual_move()
        .reg(self);

        // ── Text-level goto motions ─────────────────────────────────────────
        motion!(
            "goto-first-line",
            "Move cursors to the first character of the buffer.",
            cmd_goto_first_line,
            jump
        );
        motion!(
            "goto-last-line",
            "Move cursors to the first character of the last line.",
            cmd_goto_last_line,
            jump
        );

        // ── Line-position motions ─────────────────────────────────────────────
        motion!(
            "goto-line-start",
            "Move cursors to the start of the line.",
            cmd_goto_line_start
        );
        motion!(
            "goto-line-end",
            "Move cursors to the last character on the line.",
            cmd_goto_line_end
        );
        motion!(
            "goto-first-nonblank",
            "Move cursors to the first non-blank character on the line.",
            cmd_goto_first_nonblank
        );

        // ── Word motions ──────────────────────────────────────────────────────
        // All four are reaching: Move mode anchors the selection on a word
        // reached by navigating away from the cursor. Not safe to replay
        // positionally — dot-repeat would advance past the intended word.
        motion!(
            "select-next-word",
            "Select the next word.",
            cmd_select_next_word,
            reaching
        );
        motion!(
            "select-next-uppercase-word",
            "Select the next uppercase word (whitespace-delimited).",
            cmd_select_next_uppercase_word,
            reaching
        );
        motion!(
            "select-prev-word",
            "Select the previous word.",
            cmd_select_prev_word,
            reaching
        );
        motion!(
            "select-prev-uppercase-word",
            "Select the previous uppercase word (whitespace-delimited).",
            cmd_select_prev_uppercase_word,
            reaching
        );

        // ── Paragraph motions ─────────────────────────────────────────────────
        motion!(
            "next-paragraph",
            "Move cursors to the start of the next paragraph.",
            cmd_next_paragraph
        );
        motion!(
            "prev-paragraph",
            "Move cursors to the start of the previous paragraph.",
            cmd_prev_paragraph
        );

        // ── Line selection ────────────────────────────────────────────────────
        selection!(
            "select-line",
            "Select the full current line (forward).",
            cmd_select_line
        );
        selection!(
            "select-line-backward",
            "Select the full current line (backward).",
            cmd_select_line_backward
        );

        // ── Selection commands ────────────────────────────────────────────────
        selection!(
            "collapse-selection",
            "Collapse each selection to a single cursor at the head.",
            cmd_collapse_selection_to_head
        );
        selection!(
            "flip-selections",
            "Swap anchor and head for each selection.",
            cmd_flip_selections
        );
        selection!(
            "keep-primary-selection",
            "Remove all selections except the primary.",
            cmd_keep_primary_selection
        );
        selection!(
            "select-all",
            "Select the entire buffer.",
            cmd_select_all,
            jump
        );
        selection!(
            "remove-primary-selection",
            "Remove the primary selection, promoting the next.",
            cmd_remove_primary_selection
        );
        selection!(
            "cycle-primary-forward",
            "Cycle the primary selection forward.",
            cmd_cycle_primary_forward
        );
        selection!(
            "cycle-primary-backward",
            "Cycle the primary selection backward.",
            cmd_cycle_primary_backward
        );
        selection!(
            "split-selection-on-newlines",
            "Split each multi-line selection into one per line.",
            cmd_split_selection_on_newlines
        );
        selection!(
            "trim-selection-whitespace",
            "Trim leading and trailing whitespace from each selection.",
            cmd_trim_selection_whitespace
        );
        selection!(
            "copy-selection-on-next-line",
            "Duplicate each selection on the line below.",
            cmd_copy_selection_on_next_line
        );
        selection!(
            "copy-selection-on-prev-line",
            "Duplicate each selection on the line above.",
            cmd_copy_selection_on_prev_line
        );

        // ── Text objects — line ───────────────────────────────────────────────
        selection!(
            "inner-line",
            "Select inner line content (excluding the newline).",
            cmd_inner_line
        );
        selection!(
            "around-line",
            "Select the line including its newline.",
            cmd_around_line
        );

        // ── Text objects — word ───────────────────────────────────────────────
        selection!("inner-word", "Select inner word.", cmd_inner_word);
        ecmd(
            "select-word-nearest-on-line",
            "Select inner word; on whitespace snap to nearest word on the same visual line.",
            cmd_visual_select_word_nearest_on_line,
        )
        .extendable()
        .reg(self);
        selection!(
            "around-word",
            "Select word plus surrounding whitespace.",
            cmd_around_word
        );
        selection!(
            "inner-uppercase-word",
            "Select inner uppercase word (whitespace-delimited).",
            cmd_inner_uppercase_word
        );
        selection!(
            "around-uppercase-word",
            "Select uppercase word plus surrounding whitespace.",
            cmd_around_uppercase_word
        );

        // ── Text objects — brackets ───────────────────────────────────────────
        selection!(
            "inner-paren",
            "Select content inside the nearest `()`.",
            cmd_inner_paren
        );
        selection!(
            "around-paren",
            "Select content including the nearest `()`.",
            cmd_around_paren
        );
        selection!(
            "inner-bracket",
            "Select content inside the nearest `[]`.",
            cmd_inner_bracket
        );
        selection!(
            "around-bracket",
            "Select content including the nearest `[]`.",
            cmd_around_bracket
        );
        selection!(
            "inner-brace",
            "Select content inside the nearest `{}`.",
            cmd_inner_brace
        );
        selection!(
            "around-brace",
            "Select content including the nearest `{}`.",
            cmd_around_brace
        );
        selection!(
            "inner-angle",
            "Select content inside the nearest `<>`.",
            cmd_inner_angle
        );
        selection!(
            "around-angle",
            "Select content including the nearest `<>`.",
            cmd_around_angle
        );

        // ── Text objects — quotes ─────────────────────────────────────────────
        selection!(
            "inner-double-quote",
            "Select content inside the nearest `\"`.",
            cmd_inner_double_quote
        );
        selection!(
            "around-double-quote",
            "Select content including the nearest `\"`.",
            cmd_around_double_quote
        );
        selection!(
            "inner-single-quote",
            "Select content inside the nearest `'`.",
            cmd_inner_single_quote
        );
        selection!(
            "around-single-quote",
            "Select content including the nearest `'`.",
            cmd_around_single_quote
        );
        selection!(
            "inner-backtick",
            "Select content inside the nearest backtick pair.",
            cmd_inner_backtick
        );
        selection!(
            "around-backtick",
            "Select content including the nearest backtick pair.",
            cmd_around_backtick
        );

        // ── Text objects — arguments ──────────────────────────────────────────
        selection!(
            "inner-argument",
            "Select the argument at the cursor (trimmed).",
            cmd_inner_argument
        );
        selection!(
            "around-argument",
            "Select the argument and its separator comma.",
            cmd_around_argument
        );

        // ── Surround selection ────────────────────────────────────────────
        selection!(
            "surround-paren",
            "Select surrounding `()` delimiters.",
            cmd_surround_paren
        );
        selection!(
            "surround-bracket",
            "Select surrounding `[]` delimiters.",
            cmd_surround_bracket
        );
        selection!(
            "surround-brace",
            "Select surrounding `{}` delimiters.",
            cmd_surround_brace
        );
        selection!(
            "surround-angle",
            "Select surrounding `<>` delimiters.",
            cmd_surround_angle
        );
        selection!(
            "surround-double-quote",
            "Select surrounding `\"` delimiters.",
            cmd_surround_double_quote
        );
        selection!(
            "surround-single-quote",
            "Select surrounding `'` delimiters.",
            cmd_surround_single_quote
        );
        selection!(
            "surround-backtick",
            "Select surrounding backtick delimiters.",
            cmd_surround_backtick
        );

        // ── Surround add ──────────────────────────────────────────────────────
        ecmd(
            "surround-add",
            "Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair.",
            cmd_surround_add,
        )
        .repeatable()
        .clears_extend()
        .reg(self);

        // ── Edit commands ─────────────────────────────────────────────────────
        edit!(
            "delete-char-forward",
            "Delete the character (or selection) under the cursor.",
            delete_char_forward
        );
        edit!(
            "delete-char-backward",
            "Delete the character before each cursor.",
            delete_char_backward
        );
        edit!(
            "delete-selection",
            "Delete all selections.",
            delete_selection
        );
        edit!(
            "delete-word-backward",
            "Delete the word before each cursor.",
            delete_word_backward
        );

        use super::super::commands::*;

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
        .transparent_to_last_command()
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
        ecmd(
            "yank",
            "Copy selections to the clipboard and kill ring without deleting.",
            cmd_yank,
        )
        .reg(self);
        ecmd(
            "paste-after",
            "Paste register contents after the selection.",
            cmd_paste_after,
        )
        .paste()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-before",
            "Paste register contents before the selection.",
            cmd_paste_before,
        )
        .paste()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-ring-older",
            "Cycle kill ring one step older and re-paste.",
            cmd_paste_ring_older,
        )
        .paste_cycle()
        .repeatable()
        .clears_extend()
        .reg(self);
        ecmd(
            "paste-ring-newer",
            "Cycle kill ring one step newer and re-paste.",
            cmd_paste_ring_newer,
        )
        .paste_cycle()
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
        // the actual replay with &mut Editor after handle_key returns (D7-safe).
        ecmd(
            "repeat-last-action",
            "Repeat the last editing action.",
            cmd_repeat,
        )
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
            "Clear search highlights (`:clear-search` / `:cs`).",
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
        ecmd(
            "select-all-matches",
            "Turn every search match in the buffer into a selection.",
            cmd_select_all_matches,
        )
        .reg(self);
        ecmd(
            "use-selection-as-search",
            "Use primary selection text as the search pattern.",
            cmd_use_selection_as_search,
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
            "Quit without checking for unsaved changes.",
            cmd_quit,
        )
        .reg(self);

        // ── Editor commands — pane focus stubs (M9+) ─────────────────────────
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

        // ── Typed commands (`:` command line) ─────────────────────────────────
        macro_rules! typed_cmd {
            ($name:literal, $doc:literal, $aliases:expr, $fun:expr) => {
                self.register_typed(TypedCommand {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    aliases: $aliases,
                    fun: $fun,
                })
            };
        }

        typed_cmd!("quit", "Close the editor.", &["q"], typed_quit);
        typed_cmd!(
            "quit-all",
            "Quit the editor, closing all buffers.",
            &["qa"],
            typed_quit_all
        );
        typed_cmd!("write", "Write changes to disk.", &["w"], typed_write);
        typed_cmd!(
            "write-quit",
            "Write changes and quit.",
            &["wq"],
            typed_write_quit
        );
        typed_cmd!(
            "write-all",
            "Write all modified buffers to disk.",
            &["wa"],
            typed_write_all
        );
        typed_cmd!(
            "toggle-soft-wrap",
            "Toggle soft line wrapping.",
            &["wrap"],
            typed_toggle_soft_wrap
        );
        typed_cmd!(
            "set",
            "Set a configuration value: :set global|buffer key=value.",
            &[],
            typed_set
        );
        typed_cmd!(
            "messages",
            "Show the message log in a read-only scratch buffer.",
            &["mes"],
            typed_messages
        );
        typed_cmd!(
            "reload-config",
            "Reload init.scm from scratch.",
            &[],
            typed_reload_config
        );
        typed_cmd!(
            "edit",
            "Open a file or reload current file.",
            &["e"],
            typed_edit
        );
        typed_cmd!(
            "buffer-delete",
            "Close the focused buffer.",
            &["bd"],
            typed_buffer_delete
        );
        typed_cmd!(
            "bnext",
            "Switch to next buffer in open-order.",
            &["bn"],
            typed_bnext
        );
        typed_cmd!(
            "bprev",
            "Switch to previous buffer in open-order.",
            &["bp"],
            typed_bprev
        );
        typed_cmd!(
            "split",
            "Split the current pane horizontally.",
            &["sp"],
            typed_split
        );
        typed_cmd!(
            "vsplit",
            "Split the current pane vertically.",
            &["vsp"],
            typed_vsplit
        );
        typed_cmd!(
            "theme",
            "Load a theme by name: :theme <name>. No arg shows current theme.",
            &[],
            typed_theme
        );
        typed_cmd!(
            "theme-debug",
            "Show resolved styles for key UI scopes of the active theme.",
            &[],
            typed_theme_debug
        );
        typed_cmd!(
            "change-directory",
            "Change the working directory.",
            &["cd"],
            typed_cd
        );
        typed_cmd!(
            "print-working-directory",
            "Print the current working directory.",
            &["pwd"],
            typed_pwd
        );
        typed_cmd!(
            "list-buffers",
            "List all open buffers.",
            &["ls"],
            typed_list_buffers
        );
        typed_cmd!(
            "plugin-status",
            "Show declared plugins and their load state.",
            &["plugins"],
            typed_plugin_status
        );
        typed_cmd!("buffer", "Switch to an open buffer.", &["b"], typed_buffer);
        typed_cmd!(
            "version",
            "Print the editor version.",
            &["ver"],
            typed_version
        );
        typed_cmd!("tutor", "Open the interactive tutorial.", &[], typed_tutor);
        typed_cmd!(
            "goto",
            "Jump to a 1-based line number: :goto 42.",
            &[],
            typed_goto_line
        );
    }
}
