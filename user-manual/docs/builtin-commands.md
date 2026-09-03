# Builtin Commands

These are editor commands — reached from a key binding or `(call! …)`, never from the command mode prompt. (Typed commands, the `:`-only kind, are listed on the [Command mode](command-mode.md) page instead.) Every command below is a native one — owned by the editor itself (plugin id `"hume"`), not a Steel plugin. They are callable two ways from Scheme:

```scheme
;; directly, as a bare binding
(move-left)
;; or through the generic dispatcher, like a plugin command
(call! "move-left")
```

The first form is only valid for builtins (they are pre-registered in the Steel engine); `call!` works for any key-bindable command, builtin or Steel-defined. Each entry lists the command name and what it does. For key bindings that trigger these commands, see the [key reference](default-keys.md) and the per-topic pages.

## Motions

Move the cursor/selection. Callable as `(name)` or `(call! "name")`.

| Command | Default key | Effect |
|---------|-------------|--------|
| `goto-first-line` | `g g` | Move cursors to the first character of the buffer. |
| `goto-first-nonblank` | `g s` | Move cursors to the first non-blank character on the line. |
| `goto-last-line` | `g e` | Move cursors to the first character of the last line. |
| `goto-line-end` | `End` / `g l` | Move cursors to the last character on the line. |
| `goto-line-start` | `Home` / `g h` | Move cursors to the start of the line. |
| `goto-matching-pair` | `#` | Move cursors to the matching bracket or tag. |
| `goto-next-argument` | `g a` | Select the next argument. |
| `goto-next-class` | `g t` | Select the next class or type. |
| `goto-next-comment` | `g c` | Select the next comment. |
| `goto-next-function` | `g f` | Select the next function. |
| `goto-next-paragraph` | `}` | Select the next paragraph. |
| `goto-next-test` | `g u` | Select the next unit test. |
| `goto-next-value` | `g v` | Select the next array/tuple/struct value. |
| `goto-prev-argument` | `g A` | Select the previous argument. |
| `goto-prev-class` | `g T` | Select the previous class or type. |
| `goto-prev-comment` | `g C` | Select the previous comment. |
| `goto-prev-function` | `g F` | Select the previous function. |
| `goto-prev-paragraph` | `{` | Select the previous paragraph. |
| `goto-prev-test` | `g U` | Select the previous unit test. |
| `goto-prev-value` | `g V` | Select the previous array/tuple/struct value. |
| `move-down` | `j` / `↓` | Move cursors down one visual line (one buffer line with a count). |
| `move-left` | `h` / `←` | Move cursors one grapheme to the left. |
| `move-right` | `l` / `→` | Move cursors one grapheme to the right. |
| `move-up` | `k` / `↑` | Move cursors up one visual line (one buffer line with a count). |
| `select-line` | `x` / `Ctrl+x` | Select the full current line (forward). |
| `select-line-backward` | `X` / `Ctrl+X` | Select the full current line (backward). |
| `select-next-uppercase-word` | `W` | Select the next uppercase word (whitespace-delimited). |
| `select-next-word` | `w` | Select the next word. |
| `select-prev-uppercase-word` | `B` | Select the previous uppercase word (whitespace-delimited). |
| `select-prev-word` | `b` | Select the previous word. |

## Selection commands

Operate on the selection set itself.

| Command | Default key | Effect |
|---------|-------------|--------|
| `collapse-selection` | — | Collapse each selection to a single cursor at the head. |
| `copy-selection-on-next-line` | `C` | Duplicate each selection on the line below. |
| `copy-selection-on-prev-line` | — | Duplicate each selection on the line above. |
| `cycle-primary-backward` | `(` | Cycle the primary selection backward. |
| `cycle-primary-forward` | `)` | Cycle the primary selection forward. |
| `flip-selections` | `Ctrl+e` | Swap anchor and head for each selection. |
| `keep-primary-selection` | `,` | Remove all selections except the primary. |
| `remove-primary-selection` | `Ctrl+,` | Remove the primary selection, promoting the next. |
| `select-all` | `%` | Select the entire buffer. |
| `split-selection-on-newlines` | `S` | Split each multi-line selection into one per line. |
| `trim-selection-whitespace` | `_` | Trim leading and trailing whitespace from each selection. |

## Text objects

Select a delimited region around the cursor.

| Command | Default key | Effect |
|---------|-------------|--------|
| `around-angle` | `m a <` / `m a >` | Select content including the nearest `<>`. |
| `around-argument` | `m a a` | Select the argument and its separator comma. Structure-aware — uses the language's `parameter` object when the grammar defines one. |
| `around-backtick` | `` m a ` `` | Select content including the nearest backtick pair. |
| `around-brace` | `m a {` / `m a }` | Select content including the nearest `{}`. |
| `around-bracket` | `m a [` / `m a ]` | Select content including the nearest `[]`. |
| `around-class` | `m a t` | Select the class or type including its header. Requires a grammar with a `textobjects.scm`. |
| `around-comment` | `m a c` | Select the whole comment block. Requires a grammar with a `textobjects.scm`. |
| `around-double-quote` | `m a "` | Select content including the nearest `"`. |
| `around-function` | `m a f` | Select the function including its signature (and attributes/decorators). Requires a grammar with a `textobjects.scm`. |
| `around-line` | `m a l` | Select the line including its newline. |
| `around-paragraph` | `m a p` | Select the paragraph plus its trailing blank gap. |
| `around-paren` | `m a (` / `m a )` | Select content including the nearest `()`. |
| `around-single-quote` | `m a '` | Select content including the nearest `'`. |
| `around-test` | `m a u` | Select the whole unit test, including its attribute or decorator. Requires a grammar with a `textobjects.scm`. |
| `around-uppercase-word` | `m a W` | Select uppercase word plus one adjacent whitespace run. |
| `around-value` | `m a v` | Select an array/tuple/struct value plus its separator comma. Requires a grammar with a `textobjects.scm`. |
| `around-word` | `m a w` | Select word plus one adjacent whitespace run. |
| `inner-angle` | `m i <` / `m i >` | Select content inside the nearest `<>`. |
| `inner-argument` | `m i a` | Select the argument at the cursor (trimmed). Structure-aware — uses the language's `parameter` object when the grammar defines one. |
| `inner-backtick` | `` m i ` `` | Select content inside the nearest backtick pair. |
| `inner-brace` | `m i {` / `m i }` | Select content inside the nearest `{}`. |
| `inner-bracket` | `m i [` / `m i ]` | Select content inside the nearest `[]`. |
| `inner-class` | `m i t` | Select inside a class or type. Requires a grammar with a `textobjects.scm`. |
| `inner-comment` | `m i c` | Select inside a comment. Requires a grammar with a `textobjects.scm`. |
| `inner-double-quote` | `m i "` | Select content inside the nearest `"`. |
| `inner-function` | `m i f` | Select inside a function. Requires a grammar with a `textobjects.scm`. |
| `inner-line` | `m i l` | Select inner line content (excluding the newline). |
| `inner-paragraph` | `m i p` | Select the paragraph's text, excluding any trailing blank gap. |
| `inner-paren` | `m i (` / `m i )` | Select content inside the nearest `()`. |
| `inner-single-quote` | `m i '` | Select content inside the nearest `'`. |
| `inner-test` | `m i u` | Select inside a unit test function's body. Requires a grammar with a `textobjects.scm`. |
| `inner-uppercase-word` | `m i W` | Select inner uppercase word (whitespace-delimited). |
| `inner-value` | `m i v` | Select inside an array/tuple/struct value. Requires a grammar with a `textobjects.scm`. |
| `inner-word` | `m i w` | Select inner word. |
| `select-uppercase-word` | `M M` | Select the uppercase word (WORD) under the cursor. |
| `select-word` | `m m` | Select the word under the cursor. |
| `select-word-nearest-on-line` | — | Select the word under the cursor, or the nearest word on the same visual line when on whitespace; span follows word-selects-whitespace. |

## Surround

Select or wrap delimiter pairs.

| Command | Default key | Effect |
|---------|-------------|--------|
| `surround-add` | `m w` + char | Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair. |
| `surround-angle` | `m s <` / `m s >` | Select surrounding `<>` delimiters. |
| `surround-backtick` | `` m s ` `` | Select surrounding `` ` `` delimiters. |
| `surround-brace` | `m s {` / `m s }` | Select surrounding `{}` delimiters. |
| `surround-bracket` | `m s [` / `m s ]` | Select surrounding `[]` delimiters. |
| `surround-double-quote` | `m s "` | Select surrounding `"` delimiters. |
| `surround-paren` | `m s (` / `m s )` | Select surrounding `()` delimiters. |
| `surround-single-quote` | `m s '` | Select surrounding `'` delimiters. |

## Edits

Modify buffer text.

| Command | Default key | Effect |
|---------|-------------|--------|
| `delete-char-backward` | — | Delete the character before each cursor. |
| `delete-char-forward` | — | Delete the character (or selection) under the cursor. |
| `delete-selection` | — | Delete all selections. |
| `delete-word-backward` | `Ctrl+w` *(insert mode)* | Delete the word before each cursor. |
| `make-text-capitalized` | `G C` | Capitalize each word in every selection (Title Case). |
| `make-text-lowercase` | `G L` | Lowercase the text in each selection. |
| `make-text-uppercase` | `G U` | Uppercase the text in each selection. |

## Editor commands

Mode transitions, paste, search, scrolling, pane management, and more.

| Command | Default key | Effect |
|---------|-------------|--------|
| `align-selections` | `&` | Align each selection's anchor to the primary selection's anchor column. |
| `bottom-view-on-cursor` | `z j` | Scroll so the primary selection head sits at the bottom of the viewport. |
| `center-view-on-cursor` | `z z` | Scroll so the primary selection head sits at the vertical center of the viewport. |
| `change` | `c` | Delete selections onto the kill ring, then enter insert mode (one undo group). |
| `clear-search` | — | Clear search highlights. `Esc` also clears them; bind a key to run this directly. |
| `collapse-and-exit-extend` | `;` | Collapse each selection to its cursor and exit extend mode. |
| `collapse-to-anchor-and-exit-extend` | `Ctrl+;` | Collapse each selection to its anchor and exit extend mode. |
| `command-mode` | `:` | Open the command mode prompt. |
| `delete` | `d` | Delete selections, pushing their text onto the kill ring. |
| `exit-insert` | `Esc` / `Ctrl+c` | Return to normal mode from insert mode. |
| `find-backward` | `F` + char | Find previous occurrence of a character (inclusive, backward). |
| `find-forward` | `f` + char | Find next occurrence of a character (inclusive, forward). |
| `goto-alternate-buffer` | — | Switch to the most-recently-focused other buffer. |
| `half-page-down` | `Ctrl+d` | Scroll down by half a viewport height. |
| `half-page-up` | `Ctrl+u` | Scroll up by half a viewport height. |
| `indent` | `>` | Indent every line touched by a selection by one level. |
| `insert-after` | — | Enter insert mode after the cursor (move one grapheme right). |
| `insert-at-line-end` | `A` | Enter insert mode after the last character on the line. |
| `insert-at-line-start` | `I` | Enter insert mode at the first non-blank character on the line. |
| `insert-at-selection-end` | `a` | Enter insert mode after the end of the selection. |
| `insert-at-selection-start` | `i` | Enter insert mode at the start of the selection. |
| `insert-before` | — | Enter insert mode; collapse each selection to its start. |
| `join-lines-select-spaces` | `J` | Join lines inside each selection and select the inserted spaces. |
| `jump-backward` | `Ctrl+o` | Navigate to the previous position in the jump list. |
| `jump-forward` | `Ctrl+i` / `Tab` | Navigate to the next position in the jump list. |
| `open-line-above` | `O` | Open a new line above the cursor and enter insert mode. |
| `open-line-below` | `o` | Open a new line below the cursor and enter insert mode. |
| `page-down` | `PageDown` | Scroll down by one viewport height. |
| `page-up` | `PageUp` | Scroll up by one viewport height. |
| `pane-close` | `Ctrl+p c` | Close the focused pane. |
| `pane-focus-down` | `Ctrl+p j` | Focus the pane below. |
| `pane-focus-left` | `Ctrl+p h` | Focus the pane to the left. |
| `pane-focus-next` | `Tab` / `Ctrl+p p` | Focus the next pane. |
| `pane-focus-right` | `Ctrl+p l` | Focus the pane to the right. |
| `pane-focus-up` | `Ctrl+p k` | Focus the pane above. |
| `pane-split` | `Ctrl+p s` | Split the focused pane, stacking the new pane below it. |
| `pane-vsplit` | `Ctrl+p v` | Split the focused pane side by side. |
| `paste-after` | — | Paste register contents after the selection. Bare (no `"<reg>` prefix) reads the kill-ring head, with no clipboard fallback. |
| `paste-before` | — | Paste register contents before the selection. Bare (no `"<reg>` prefix) reads the kill-ring head, with no clipboard fallback. |
| `paste-ring-newer` | `]` | Cycle kill ring one step newer and re-paste. |
| `paste-ring-older` | `[` | Cycle kill ring one step older and re-paste. |
| `redo` | `U` / `Ctrl+r` | Redo the last undone change. |
| `repeat-find-backward` | `-` | Repeat the last find/till motion backward. |
| `repeat-find-forward` | `=` | Repeat the last find/till motion forward. |
| `repeat-last-action` | `.` | Repeat the last editing action. |
| `replace` | `r` + char | Replace every character in each selection with the next typed character. |
| `search-backward` | `?` | Enter search mode (backward). |
| `search-forward` | `/` | Enter search mode (forward). |
| `search-next` | `n` | Jump to the next search match. |
| `search-prev` | `N` | Jump to the previous search match. |
| `search-selection` | `Ctrl+/` | Use the primary selection text literally as the search pattern. |
| `search-word-under-cursor` | `*` | Search the whole word under the cursor. |
| `select-all-matches` | `m /` | Turn every search match in the buffer into a selection. |
| `select-last-insertion` | `m i i` | Select the text typed during the most recently completed insert session. |
| `select-within` | `s` | Select regex matches within current selections. |
| `smart-paste-after` | `p` | Paste after the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise. |
| `smart-paste-before` | `P` | Paste before the selection: kill-ring head while nothing has been edited since the last capture, clipboard otherwise. |
| `till-backward` | `T` + char | Move to just after previous occurrence of a character (exclusive). |
| `till-forward` | `t` + char | Move to just before next occurrence of a character (exclusive). |
| `toggle-extend` | `e` | Toggle sticky extend mode. |
| `top-view-on-cursor` | `z k` | Scroll so the primary selection head sits at the top of the viewport. |
| `undo` | `u` | Undo the last change. |
| `unindent` | `<` | Unindent every line touched by a selection by one level. |
| `yank` | `y` | Copy selections to the clipboard and kill ring without deleting. |
