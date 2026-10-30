# Editor Commands

These are the editor commands built into HUME. A key runs them and a plugin reaches them with `(call! …)`, but you can't run them by name at the `:` prompt. Typed commands are on the [Command mode](command-mode.md) page. Editor functions such as `insert-key!`, which only a script can call, are on the [Plugin API](plugin-api.md#commands-and-functions) page. A plugin's own commands are listed in its documentation.

Scheme can call the commands on this page two ways:

```scheme
;; directly, as a bare binding
(move-left pane)
;; or through the generic dispatcher, like a plugin command
(call! "move-left" pane)
```

The bare form works for the commands on this page except `insert-register`, which only `call!` and keys reach. `call!` works for any editor command, built in or defined by a script, except `repeat-last-action` and `pipe`. Those two run only from a key: calling them from Scheme raises an error.

Every command needs a `pane` that names a pane; a value with only a buffer (from `(buffers)`, or a buffer-level hook) is refused. Most commands act on that pane even when it isn't focused, as long as it is still open and still shows the same buffer. These need the focused pane:

- entering or leaving Insert mode, including `change`, `open-line-above`/`open-line-below` and `completion-trigger`
- `command-mode`
- toggling or leaving Extend mode (`toggle-extend`, `collapse-and-exit-extend`, `collapse-to-anchor-and-exit-extend`)
- paste and paste cycling
- `repeat-last-action`
- switching or opening tabs
- every pane focus, split, close and rotate command

A `pane` the command can't act on raises an error; the command never falls back to whatever is focused. See [Calling other commands](writing-plugins.md#calling-other-commands) for how a plugin uses these.

Some commands also take a value that a key press asks for. `find-forward`, `find-backward`, `till-forward`, `till-backward`, `replace` and `surround-add` take a character; `search-forward`, `search-backward` and `sift-within` take a pattern. From a script, that value is an argument, as each entry says.

For the keys that run these commands, see the [key reference](default-keys.md) and the per-topic pages. A key marked *(kitty)* needs the kitty keyboard protocol; see [Terminal compatibility](installation.md#terminal-compatibility).

## Motions

Move the cursor or selection.

| Command | Default key | Effect |
|---------|-------------|--------|
| `find-backward` | `F` + char | Find previous occurrence of a character (inclusive, backward). From `call!`, pass the character as an argument. |
| `find-forward` | `f` + char | Find next occurrence of a character (inclusive, forward). From `call!`, pass the character as an argument. |
| `goto-first-line` | `g g` | Move cursors to the first character of the buffer. |
| `goto-first-nonblank` | `g s` | Move cursors to the first non-blank character on the line. |
| `goto-last-line` | `g e` | Move cursors to the first character of the last line. |
| `goto-line-end` | `End` / `g l` | Move cursors to the last character on the line, or past it in Insert mode. |
| `goto-line-start` | `Home` / `g h` | Move cursors to the start of the line. |
| `goto-matching-pair` | `#` | Move cursors to the matching bracket or tag. |
| `goto-next-argument` | `g a` | Select the next argument. |
| `goto-next-class` | `g t` | Select the next class or type. |
| `goto-next-comment` | `g c` | Select the next comment. |
| `goto-next-function` | `g f` | Select the next function. |
| `goto-next-paragraph` | `}` | Select the next paragraph. |
| `goto-next-tag` | `g x` | Select the next tag pair (element). |
| `goto-next-test` | `g u` | Select the next unit test. |
| `goto-next-value` | `g v` | Select the next array/tuple/struct value. |
| `goto-prev-argument` | `g A` | Select the previous argument. |
| `goto-prev-class` | `g T` | Select the previous class or type. |
| `goto-prev-comment` | `g C` | Select the previous comment. |
| `goto-prev-function` | `g F` | Select the previous function. |
| `goto-prev-paragraph` | `{` | Select the previous paragraph. |
| `goto-prev-tag` | `g X` | Select the previous tag pair (element). |
| `goto-prev-test` | `g U` | Select the previous unit test. |
| `goto-prev-value` | `g V` | Select the previous array/tuple/struct value. |
| `jump-backward` | `Ctrl-o` | Navigate to the previous position in the jump list. |
| `jump-forward` | `Ctrl-i` / `Tab` (without kitty) | Navigate to the next position in the jump list. |
| `move-down` | `j` / `↓` | Move cursors down one visual line (one buffer line with a count). |
| `move-left` | `h` / `←` | Move cursors one character to the left. |
| `move-right` | `l` / `→` | Move cursors one character to the right. |
| `move-up` | `k` / `↑` | Move cursors up one visual line (one buffer line with a count). |
| `repeat-find-backward` | `-` | Repeat the last find/till motion backward. |
| `repeat-find-forward` | `=` | Repeat the last find/till motion forward. |
| `select-line` | `x` / `Ctrl-x` | Select the full current line (forward). |
| `select-line-backward` | `X` / `Ctrl-Shift-x` | Select the full current line (backward). |
| `select-next-uppercase-word` | `W` | Select the next uppercase word (whitespace-delimited). |
| `select-next-word` | `w` | Select the next word. |
| `select-prev-uppercase-word` | `B` | Select the previous uppercase word (whitespace-delimited). |
| `select-prev-word` | `b` | Select the previous word. |
| `till-backward` | `T` + char | Move to the character after the previous occurrence of a character (exclusive). From `call!`, pass the character as an argument. |
| `till-forward` | `t` + char | Move to the character before the next occurrence of a character (exclusive). From `call!`, pass the character as an argument. |

## Selection commands

Create, refine, or manipulate the selection set.

| Command | Default key | Effect |
|---------|-------------|--------|
| `collapse-and-exit-extend` | `;` | Collapse each selection to its cursor and exit Extend mode. |
| `collapse-selection` | — | Collapse each selection to a single cursor at the head. |
| `collapse-to-anchor-and-exit-extend` | `Ctrl-;` *(kitty)* | Collapse each selection to its anchor and exit Extend mode. |
| `copy-selection-on-next-line` | `C` | Duplicate each selection one selection-height below. |
| `copy-selection-on-prev-line` | — | Duplicate each selection one selection-height above. |
| `cycle-primary-backward` | `(` | Cycle the primary selection backward. |
| `cycle-primary-forward` | `)` | Cycle the primary selection forward. |
| `flip-selections` | `Ctrl-e` | Swap anchor and head for each selection. |
| `keep-primary-selection` | `,` | Remove all selections except the primary. |
| `remove-primary-selection` | `Ctrl-,` *(kitty)* | Remove the primary selection, promoting the next. |
| `select-all` | `%` | Select the entire buffer. |
| `select-last-insertion` | `m i i` | Select the text typed during the most recently completed insert session. |
| `sift-within` | `s` | Narrow each selection down to its regex matches. From `call!`, the pattern is required. |
| `split-selection-on-newlines` | `S` | Split each multi-line selection into one per line. |
| `trim-selection-whitespace` | `_` | Trim leading and trailing whitespace from each selection. |

## Text objects

Select a delimited region around the cursor. An object that needs grammar support works only in a language whose grammar provides it; see [Syntax Highlighting](syntax-highlighting.md).

| Command | Default key | Effect |
|---------|-------------|--------|
| `around-angle` | `m a <` / `m a >` | Select content including the nearest `<>`. |
| `around-argument` | `m a a` | Select the argument and its separator comma. Uses the grammar when it supports arguments, a plain-text scan otherwise. |
| `around-backtick` | `` m a ` `` | Select content including the nearest backtick pair. |
| `around-brace` | `m a {` / `m a }` | Select content including the nearest `{}`. |
| `around-bracket` | `m a [` / `m a ]` | Select content including the nearest `[]`. |
| `around-class` | `m a t` | Select the class or type including its header. Needs grammar support. |
| `around-comment` | `m a c` | Select the whole comment block. Needs grammar support. |
| `around-double-quote` | `m a "` | Select content including the nearest `"`. |
| `around-function` | `m a f` | Select the function including its signature (and attributes/decorators). Needs grammar support. |
| `around-line` | `m a l` | Select the line including its newline. |
| `around-paragraph` | `m a p` | Select the paragraph plus its trailing blank gap. |
| `around-paren` | `m a (` / `m a )` | Select content including the nearest `()`. |
| `around-single-quote` | `m a '` | Select content including the nearest `'`. |
| `around-tag` | `m a x` | Select a pair of tags and their content. Uses the grammar when it supports tags, a plain-text scan otherwise. |
| `around-test` | `m a u` | Select the whole unit test, including its attribute or decorator. Needs grammar support. |
| `around-uppercase-word` | `m a W` | Select uppercase word plus one adjacent whitespace run. |
| `around-value` | `m a v` | Select an array/tuple/struct value plus its separator comma. Needs grammar support. |
| `around-word` | `m a w` | Select word plus one adjacent whitespace run. |
| `inner-angle` | `m i <` / `m i >` | Select content inside the nearest `<>`. |
| `inner-argument` | `m i a` | Select the argument at the cursor (trimmed). Uses the grammar when it supports arguments, a plain-text scan otherwise. |
| `inner-backtick` | `` m i ` `` | Select content inside the nearest backtick pair. |
| `inner-brace` | `m i {` / `m i }` | Select content inside the nearest `{}`. |
| `inner-bracket` | `m i [` / `m i ]` | Select content inside the nearest `[]`. |
| `inner-class` | `m i t` | Select inside a class or type. Needs grammar support. |
| `inner-comment` | `m i c` | Select inside a comment. Needs grammar support. |
| `inner-double-quote` | `m i "` | Select content inside the nearest `"`. |
| `inner-function` | `m i f` | Select inside a function. Needs grammar support. |
| `inner-line` | `m i l` | Select inner line content (excluding the newline). |
| `inner-paragraph` | `m i p` | Select the paragraph's text, excluding any trailing blank gap. |
| `inner-paren` | `m i (` / `m i )` | Select content inside the nearest `()`. |
| `inner-single-quote` | `m i '` | Select content inside the nearest `'`. |
| `inner-tag` | `m i x` | Select the content between a pair of tags. Uses the grammar when it supports tags, a plain-text scan otherwise. |
| `inner-test` | `m i u` | Select inside a unit test function's body. Needs grammar support. |
| `inner-uppercase-word` | `m i W` | Select inner uppercase word (whitespace-delimited). |
| `inner-value` | `m i v` | Select inside an array/tuple/struct value. Needs grammar support. |
| `inner-word` | `m i w` | Select inner word. |
| `select-uppercase-word` | `M M` | Select the uppercase word under the cursor. |
| `select-word` | `m m` | Select the word under the cursor. |
| `select-word-nearest-on-line` | — | Select the word under the cursor, or the nearest word on the same visual line when on whitespace. The whitespace it adds follows [`word-selects-whitespace`](settings.md#buffer-options). |

## Surround

Select or wrap delimiter pairs.

| Command | Default key | Effect |
|---------|-------------|--------|
| `surround-add` | `m w` + char | Wrap each selection with a delimiter pair. Reads the next typed character to determine the pair. From `call!`, pass the character as an argument. |
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
| `align-selections` | `&` | Align each selection's anchor to the primary selection's anchor column. |
| `cancel-pipe` | `Ctrl-c` | Stop the shell command `pipe` is running in the buffer and change nothing. Says so when none is running there. |
| `change` | `c` | Delete selections onto the kill ring, then enter Insert mode (one undo step). |
| `decrement` | `Ctrl-s` | Subtract the count from the number, date or time at each selection. |
| `decrement-sequence` | `G S` | Subtract n times the count from the number, date or time at the n-th selection. |
| `delete` | `d` | Delete selections, pushing their text onto the kill ring. |
| `delete-char-backward` | — | Delete the character before each cursor. |
| `delete-char-forward` | — | Delete the character (or selection) under the cursor. |
| `delete-selection` | — | Delete all selections. |
| `delete-word-backward` | `Ctrl-w` (Insert mode) | Delete the word before each cursor. |
| `increment` | `Ctrl-a` | Add the count to the number, date or time at each selection. |
| `increment-sequence` | `G A` | Add n times the count to the number, date or time at the n-th selection. |
| `indent` | `>` | Indent every line touched by a selection by one level. |
| `join-lines-select-spaces` | `J` | Join lines inside each selection and select the inserted spaces. |
| `make-text-capitalized` | `G C` | Capitalize each word in every selection (Title Case). |
| `make-text-lowercase` | `G L` | Lowercase the text in each selection. |
| `make-text-uppercase` | `G U` | Uppercase the text in each selection. |
| `open-line-above` | `O` | Open a new line above the cursor and enter Insert mode. |
| `open-line-below` | `o` | Open a new line below the cursor and enter Insert mode. |
| `pipe` | `\|` / `Ctrl-\|` (Insert mode, *kitty*) | Run a shell command once per selection, with the selection as its input, and replace the selection with its output. In Insert mode, insert its output at each cursor. Only from a key: calling it from Scheme raises an error. |
| `replace` | `r` + char | Replace every character in each selection with the next typed character. From `call!`, pass the character as an argument. |
| `toggle-comment` | `G G` | Comment or uncomment the lines of each selection. |
| `unindent` | `<` | Unindent every line touched by a selection by one level. |

## Clipboard and registers

Move text between the buffer, the kill ring, and the system clipboard.

| Command | Default key | Effect |
|---------|-------------|--------|
| `paste-after` | — | Paste register contents after the selection. With no register prefix, reads the kill-ring head, with no clipboard fallback. |
| `paste-before` | — | Paste register contents before the selection. With no register prefix, reads the kill-ring head, with no clipboard fallback. |
| `paste-ring-newer` | `]` | Cycle kill ring one step newer and re-paste. |
| `paste-ring-older` | `[` | Cycle kill ring one step older and re-paste. |
| `smart-paste-after` | `p` | Smart paste after the selection: the kill-ring head if you have edited nothing since you last deleted, changed or yanked text, the clipboard otherwise. |
| `smart-paste-before` | `P` | Smart paste before the selection, choosing the source as `p` does. |
| `yank` | `y` | Copy selections to the clipboard and kill ring without deleting. |
| `insert-register` | `Ctrl-r` (Insert mode, `:` prompt) | Insert the register named by the next key, or the file's path (`f`) or its directory (`p`), relative to the working directory. See [Inserting a register while typing](copy-and-paste.md#inserting-a-register-while-typing). |

## Undo and repeat

Replay or roll back past changes.

| Command | Default key | Effect |
|---------|-------------|--------|
| `redo` | `U` / `Ctrl-r` | Redo the last undone change. |
| `repeat-last-action` | `.` | Repeat the last editing action. Only from a key: calling it from Scheme raises an error. |
| `undo` | `u` | Undo the last change. |

## Search

Find text and act on the matches.

| Command | Default key | Effect |
|---------|-------------|--------|
| `clear-search` | — | Clear search highlights. `Esc` also clears them. |
| `search-backward` | `?` | Enter Search mode (backward). From `call!`, the pattern is required. |
| `search-forward` | `/` | Enter Search mode (forward). From `call!`, the pattern is required. |
| `search-next` | `n` | Jump to the next search match. |
| `search-prev` | `N` | Jump to the previous search match. |
| `search-selection` | `Ctrl-/` *(kitty)* | Use the primary selection text literally as the search pattern. |
| `search-word-under-cursor` | `*` | Search the whole word under the cursor. |
| `select-all-matches` | `m /` | Turn every search match in the buffer into a selection. |

## Modes

Switch between Normal, Insert and Extend mode, and open the `:` prompt.

| Command | Default key | Effect |
|---------|-------------|--------|
| `command-mode` | `:` | Open the `:` prompt. |
| `completion-trigger` | `Ctrl-Space` (Insert mode) | Show completions at the cursor. |
| `exit-insert` | `Esc` / `Ctrl-c` | Return to Normal mode from Insert mode. |
| `insert-after` | — | Enter Insert mode after the cursor (one character right). |
| `insert-at-line-end` | `A` | Enter Insert mode after the last character on the line. |
| `insert-at-line-start` | `I` | Enter Insert mode at the first non-blank character on the line. |
| `insert-at-selection-end` | `a` | Enter Insert mode after the end of the selection. |
| `insert-at-selection-start` | `i` | Enter Insert mode at the start of the selection. |
| `insert-before` | — | Enter Insert mode; collapse each selection to its start. |
| `toggle-extend` | `e` | Toggle Extend mode. |

## Scrolling

Move the viewport without moving the cursor.

| Command | Default key | Effect |
|---------|-------------|--------|
| `bottom-view-on-cursor` | `z j` | Scroll so the primary selection head sits at the bottom of the viewport. |
| `center-view-on-cursor` | `z z` | Scroll so the primary selection head sits at the vertical center of the viewport. |
| `half-page-down` | `Ctrl-d` | Scroll down by half a viewport height. |
| `half-page-up` | `Ctrl-u` | Scroll up by half a viewport height. |
| `page-down` | `PageDown` | Scroll down by one viewport height. |
| `page-up` | `PageUp` | Scroll up by one viewport height. |
| `top-view-on-cursor` | `z k` | Scroll so the primary selection head sits at the top of the viewport. |

## Panes, buffers, and tabs

Split, focus, and close panes; switch buffers; switch tabs.

| Command | Default key | Effect |
|---------|-------------|--------|
| `goto-alternate-buffer` | — | Switch to the most-recently-focused other buffer. |
| `goto-next-buffer` | — | Switch to the next buffer, in the order they were opened. |
| `goto-next-tab` | `Ctrl-p t` | Switch to the next tab in display order. |
| `goto-prev-buffer` | — | Switch to the previous buffer, in the order they were opened. |
| `goto-prev-tab` | `Ctrl-p T` | Switch to the previous tab in display order. |
| `pane-close` | `Ctrl-p c` | Close the focused pane. |
| `pane-focus-down` | `Ctrl-p j` | Focus the pane below. |
| `pane-focus-left` | `Ctrl-p h` | Focus the pane to the left. |
| `pane-focus-next` | `Tab` *(kitty)* / `Ctrl-p p` | Focus the next pane. |
| `pane-focus-prev` | `Shift-Tab` *(kitty)* / `Ctrl-p P` | Focus the previous pane. |
| `pane-focus-right` | `Ctrl-p l` | Focus the pane to the right. |
| `pane-focus-up` | `Ctrl-p k` | Focus the pane above. |
| `pane-rotate-backward` | `Ctrl-p R` | Rotate the panes in the focused pane's row or column backward (that many places with a count). |
| `pane-rotate-forward` | `Ctrl-p r` | Rotate the panes in the focused pane's row or column forward (that many places with a count). |
| `pane-split` | `Ctrl-p s` | Split the focused pane, stacking the new pane below it. |
| `pane-vsplit` | `Ctrl-p v` | Split the focused pane side by side. |
| `tab-new` | — | Open a fresh pane viewing the focused buffer in a new tab. |
