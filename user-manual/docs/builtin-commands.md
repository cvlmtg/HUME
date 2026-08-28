# Builtin Commands

Every command below is a native editor command — owned by the editor itself (plugin id `"hume"`), not a Steel plugin. They are callable two ways from Scheme:

```scheme
;; directly, as a bare binding
(move-left)
;; or through the generic dispatcher, like a plugin command
(call! "move-left")
```

The first form is only valid for builtins (they are pre-registered in the Steel engine); `call!` works for any key-bindable command, builtin or Steel-defined. Each entry lists the command name and what it does. For key bindings that trigger these commands, see the [key reference](default-keys.md) and the per-topic pages.

## Motions

Move the cursor/selection. Callable as `(name)` or `(call! "name")`.

| Command | Effect |
|---------|--------|
| `move-right` | Move cursors one grapheme to the right. |
| `move-left` | Move cursors one grapheme to the left. |
| `move-down` | Move cursors down one visual line (one buffer line with a count). |
| `move-up` | Move cursors up one visual line (one buffer line with a count). |
| `goto-first-line` | Move cursors to the first character of the buffer. |
| `goto-last-line` | Move cursors to the first character of the last line. |
| `goto-line-start` | Move cursors to the start of the line. |
| `goto-line-end` | Move cursors to the last character on the line. |
| `goto-first-nonblank` | Move cursors to the first non-blank character on the line. |
| `select-next-word` | Select the next word. |
| `select-next-uppercase-word` | Select the next uppercase word (whitespace-delimited). |
| `select-prev-word` | Select the previous word. |
| `select-prev-uppercase-word` | Select the previous uppercase word (whitespace-delimited). |
| `next-paragraph` | Move cursors to the start of the next paragraph. |
| `prev-paragraph` | Move cursors to the first empty line above the current paragraph. |
| `select-line` | Select the full current line (forward). |
| `select-line-backward` | Select the full current line (backward). |

## Selection commands

Operate on the selection set itself.

| Command | Effect |
|---------|--------|
| `collapse-selection` | Collapse each selection to a single cursor at the head. |
| `flip-selections` | Swap anchor and head for each selection. |
| `keep-primary-selection` | Remove all selections except the primary. |
| `select-all` | Select the entire buffer. |
| `remove-primary-selection` | Remove the primary selection, promoting the next. |
| `cycle-primary-forward` | Cycle the primary selection forward. |
| `cycle-primary-backward` | Cycle the primary selection backward. |
| `split-selection-on-newlines` | Split each multi-line selection into one per line. |
| `trim-selection-whitespace` | Trim leading and trailing whitespace from each selection. |
| `copy-selection-on-next-line` | Duplicate each selection on the line below. |
| `copy-selection-on-prev-line` | Duplicate each selection on the line above. |

## Text objects

Select a delimited region around the cursor.

| Command | Effect |
|---------|--------|
| `inner-line` | Select inner line content (excluding the newline). |
| `around-line` | Select the line including its newline. |
| `inner-word` | Select inner word. |
| `select-word-nearest-on-line` | Select the word under the cursor, or the nearest word on the same visual line when on whitespace; span follows word-selects-whitespace. |
| `around-word` | Select word plus one adjacent whitespace run. |
| `inner-uppercase-word` | Select inner uppercase word (whitespace-delimited). |
| `around-uppercase-word` | Select uppercase word plus one adjacent whitespace run. |
| `select-word` | Select the word under the cursor. |
| `select-uppercase-word` | Select the uppercase word (WORD) under the cursor. |
| `inner-paren` | Select content inside the nearest `()`. |
| `around-paren` | Select content including the nearest `()`. |
| `inner-bracket` | Select content inside the nearest `[]`. |
| `around-bracket` | Select content including the nearest `[]`. |
| `inner-brace` | Select content inside the nearest `{}`. |
| `around-brace` | Select content including the nearest `{}`. |
| `inner-angle` | Select content inside the nearest `<>`. |
| `around-angle` | Select content including the nearest `<>`. |
| `inner-double-quote` | Select content inside the nearest `"`. |
| `around-double-quote` | Select content including the nearest `"`. |
| `inner-single-quote` | Select content inside the nearest `'`. |
| `around-single-quote` | Select content including the nearest `'`. |
| `inner-backtick` | Select content inside the nearest backtick pair. |
| `around-backtick` | Select content including the nearest backtick pair. |
| `inner-argument` | Select the argument at the cursor (trimmed). |
| `around-argument` | Select the argument and its separator comma. |

## Surround

Select or wrap delimiter pairs.

| Command | Effect |
|---------|--------|
| `surround-paren` | Select surrounding `()` delimiters. |
| `surround-bracket` | Select surrounding `[]` delimiters. |
| `surround-brace` | Select surrounding `{}` delimiters. |
| `surround-angle` | Select surrounding `<>` delimiters. |
| `surround-double-quote` | Select surrounding `"` delimiters. |
| `surround-single-quote` | Select surrounding `'` delimiters. |
| `surround-backtick` | Select surrounding backtick delimiters. |
| `surround-add` | Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair. |

## Edits

Modify buffer text.

| Command | Effect |
|---------|--------|
| `delete-char-forward` | Delete the character (or selection) under the cursor. |
| `delete-char-backward` | Delete the character before each cursor. |
| `delete-selection` | Delete all selections. |
| `delete-word-backward` | Delete the word before each cursor. |
| `make-text-lowercase` | Lowercase the text in each selection. |
| `make-text-uppercase` | Uppercase the text in each selection. |
| `make-text-capitalized` | Capitalize each word in every selection (Title Case). |

## Editor commands

Mode transitions, paste, search, scrolling, pane management, and more.

| Command | Effect |
|---------|--------|
| `insert-before` | Enter insert mode; collapse each selection to its start. |
| `insert-after` | Enter insert mode after the cursor (move one grapheme right). |
| `insert-at-line-start` | Enter insert mode at the first non-blank character on the line. |
| `insert-at-line-end` | Enter insert mode after the last character on the line. |
| `insert-at-selection-start` | Enter insert mode at the start of the selection. |
| `insert-at-selection-end` | Enter insert mode after the end of the selection. |
| `open-line-below` | Open a new line below the cursor and enter insert mode. |
| `open-line-above` | Open a new line above the cursor and enter insert mode. |
| `command-mode` | Open the command-mode mini-buffer. |
| `exit-insert` | Return to normal mode from insert mode. |
| `delete` | Delete selections, pushing their text onto the kill ring. |
| `change` | Delete selections onto the kill ring, then enter insert mode (one undo group). |
| `select-last-insertion` | Select the text typed during the most recently completed insert session. |
| `yank` | Copy selections to the clipboard and kill ring without deleting. |
| `paste-after` | Paste register contents after the selection. Bare (no `"<reg>` prefix) reads the kill-ring head, with no clipboard fallback. |
| `paste-before` | Paste register contents before the selection. Bare (no `"<reg>` prefix) reads the kill-ring head, with no clipboard fallback. |
| `smart-paste-after` | Paste after the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise. |
| `smart-paste-before` | Paste before the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise. |
| `paste-ring-older` | Cycle kill ring one step older and re-paste. |
| `paste-ring-newer` | Cycle kill ring one step newer and re-paste. |
| `join-lines-select-spaces` | Join lines inside each selection and select the inserted spaces. |
| `align-selections` | Align each selection's anchor to the primary selection's anchor column. |
| `undo` | Undo the last change. |
| `redo` | Redo the last undone change. |
| `toggle-extend` | Toggle sticky extend mode. |
| `collapse-and-exit-extend` | Collapse each selection to its cursor and exit extend mode. |
| `collapse-to-anchor-and-exit-extend` | Collapse each selection to its anchor and exit extend mode. |
| `find-forward` | Find next occurrence of a character (inclusive, forward). |
| `find-backward` | Find previous occurrence of a character (inclusive, backward). |
| `till-forward` | Move to just before next occurrence of a character (exclusive). |
| `till-backward` | Move to just after previous occurrence of a character (exclusive). |
| `repeat-find-forward` | Repeat the last find/till motion forward. |
| `repeat-find-backward` | Repeat the last find/till motion backward. |
| `replace` | Replace every character in each selection with the next typed character. |
| `page-down` | Scroll down by one viewport height. |
| `page-up` | Scroll up by one viewport height. |
| `half-page-down` | Scroll down by half a viewport height. |
| `half-page-up` | Scroll up by half a viewport height. |
| `center-view-on-cursor` | Scroll so the primary selection head sits at the vertical center of the viewport. |
| `top-view-on-cursor` | Scroll so the primary selection head sits at the top of the viewport. |
| `bottom-view-on-cursor` | Scroll so the primary selection head sits at the bottom of the viewport. |
| `repeat-last-action` | Repeat the last editing action. |
| `search-forward` | Enter search mode (forward). |
| `search-backward` | Enter search mode (backward). |
| `search-next` | Jump to the next search match. |
| `search-prev` | Jump to the previous search match. |
| `clear-search` | Clear search highlights (`:clear-search` / `:cs`). |
| `select-within` | Select regex matches within current selections. |
| `select-all-matches` | Turn every search match in the buffer into a selection. |
| `search-word-under-cursor` | Search the whole word under the cursor. |
| `search-selection` | Use the primary selection text literally as the search pattern. |
| `jump-backward` | Navigate to the previous position in the jump list. |
| `jump-forward` | Navigate to the next position in the jump list. |
| `goto-alternate-file` | Switch to the most-recently-focused other buffer. |
| `force-quit` | Quit the whole editor unconditionally, discarding unsaved changes in every buffer (same as :qa!). |
| `pane-focus-next` | Focus the next pane. |
| `pane-focus-left` | Focus the pane to the left. |
| `pane-focus-right` | Focus the pane to the right. |
| `pane-focus-up` | Focus the pane above. |
| `pane-focus-down` | Focus the pane below. |
| `pane-split` | Split the focused pane, stacking the new pane below it. |
| `pane-vsplit` | Split the focused pane side by side. |
| `pane-close` | Close the focused pane. |
