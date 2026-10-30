# Default Keys

Every default key, by mode. Keys inside pickers, drawers, popups and menus are under [Other screens](#other-screens), and the keys core plugins add are under [Keys added by core plugins](#keys-added-by-core-plugins).

::: info
Keys with a ✓ in the **Kitty** column need the kitty keyboard protocol, which HUME detects at startup on terminals that support it. Legacy terminal encodings can't send those key combinations, so those bindings don't work there. See [Terminal compatibility](installation.md#terminal-compatibility).
:::

## Normal mode

### Movement

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `h` / `←` | `move-left` |  | Move left one grapheme |
| `l` / `→` | `move-right` |  | Move right one grapheme |
| `j` / `↓` | `move-down` |  | Move down one visual line |
| `k` / `↑` | `move-up` |  | Move up one visual line |
| `w` | `select-next-word` |  | Select next word (plus one adjacent whitespace run by default; see `word-selects-whitespace`) |
| `b` | `select-prev-word` |  | Select previous word (plus one adjacent whitespace run by default) |
| `W` / `B` | `select-next-uppercase-word` / `select-prev-uppercase-word` |  | WORD variants of `w` / `b` |
| `Home` | `goto-line-start` |  | Start of line (idiomatic form is `g h`) |
| `End` | `goto-line-end` |  | End of line (idiomatic form is `g l`) |
| `{` | `goto-prev-paragraph` |  | Select the previous paragraph |
| `}` | `goto-next-paragraph` |  | Select the next paragraph |
| `#` | `goto-matching-pair` |  | Jump to the matching bracket or tag |
| `PageDown` / `PageUp` | `page-down` / `page-up` |  | Scroll one viewport down / up |
| `Ctrl-d` / `Ctrl-u` | `half-page-down` / `half-page-up` |  | Scroll half a viewport down / up |
| `Ctrl-o` | `jump-backward` |  | Jump list back |
| `Ctrl-i` | `jump-forward` |  | Jump list forward |
| `Tab` | `jump-forward` |  | Jump list forward. Under the kitty protocol, `Tab` focuses the next pane instead (see [Pane prefix](#pane-prefix-ctrl-p)) |
| `Ctrl` + a motion key, e.g. `Ctrl-h/j/k/l/w/b` | the motion's own command, extended | ✓ | One-shot extend of that motion. Keys with their own `Ctrl` binding keep it; `Ctrl` with a key that is not a motion does nothing |

### Character find

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `f` + char | `find-forward` |  | Find char forward (inclusive) |
| `F` + char | `find-backward` |  | Find char backward (inclusive) |
| `t` + char | `till-forward` |  | Find char forward (exclusive: stops before) |
| `T` + char | `till-backward` |  | Find char backward (exclusive: stops after) |
| `=` | `repeat-find-forward` |  | Repeat last find forward |
| `-` | `repeat-find-backward` |  | Repeat last find backward |

After `f`/`F`/`t`/`T`, HUME waits for the target character. `Tab` counts as a target character; a line ending never does, since these motions stay on the current line.

### Selection

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `e` | `toggle-extend` |  | Toggle Extend mode |
| `;` | `collapse-and-exit-extend` |  | Collapse to head, exit Extend |
| `Ctrl-;` | `collapse-to-anchor-and-exit-extend` | ✓ | Collapse to anchor, exit Extend |
| `Ctrl-e` | `flip-selections` |  | Swap anchor and head |
| `%` | `select-all` |  | Select entire buffer |
| `x` | `select-line` |  | Select current line (forward) |
| `X` | `select-line-backward` |  | Select current line (backward) |
| `Ctrl-x` | `select-line` (extend) |  | Same as `x` but always extends |
| `Ctrl-Shift-x` | `select-line-backward` (extend) | ✓ | Same as `X` but always extends |
| `S` | `split-selection-on-newlines` |  | Split multi-line selections on newlines |
| `C` | `copy-selection-on-next-line` |  | Copy each selection one selection-height down (a count adds that many copies, e.g. `3C`) |
| `_` | `trim-selection-whitespace` |  | Shrink each selection to exclude leading/trailing whitespace (text unchanged) |
| `,` | `keep-primary-selection` |  | Keep only the primary selection |
| `Ctrl-,` | `remove-primary-selection` | ✓ | Remove primary, promote next |
| `(` / `)` | `cycle-primary-backward` / `cycle-primary-forward` |  | Cycle primary backward / forward |
| `*` | `search-word-under-cursor` |  | Search the whole word under the cursor |

Text objects (use the `m` prefix):

| Sequence | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `m i w` / `m a w` | `inner-word` / `around-word` |  | Inner / around word |
| `m i W` / `m a W` | `inner-uppercase-word` / `around-uppercase-word` |  | Inner / around WORD |
| `m i (` / `m a (` | `inner-paren` / `around-paren` |  | Inner / around `()` (also `)`) |
| `m i [` / `m a [` | `inner-bracket` / `around-bracket` |  | Inner / around `[]` (also `]`) |
| `m i {` / `m a {` | `inner-brace` / `around-brace` |  | Inner / around `{}` (also `}`) |
| `m i <` / `m a <` | `inner-angle` / `around-angle` |  | Inner / around `<>` (also `>`) |
| `m i "` / `m a "` | `inner-double-quote` / `around-double-quote` |  | Inner / around `"…"` |
| `m i '` / `m a '` | `inner-single-quote` / `around-single-quote` |  | Inner / around `'…'` |
| `` m i ` `` / `` m a ` `` | `inner-backtick` / `around-backtick` |  | Inner / around `` `…` `` |
| `m i a` / `m a a` | `inner-argument` / `around-argument` |  | Inner / around argument (structure-aware) |
| `m i l` / `m a l` | `inner-line` / `around-line` |  | Inner / around line |
| `m i p` / `m a p` | `inner-paragraph` / `around-paragraph` |  | Inner / around paragraph |
| `m i f` / `m a f` | `inner-function` / `around-function` |  | Inner / around function |
| `m i t` / `m a t` | `inner-class` / `around-class` |  | Inner / around class or type |
| `m i c` / `m a c` | `inner-comment` / `around-comment` |  | Inner / around comment |
| `m i u` / `m a u` | `inner-test` / `around-test` |  | Inner / around unit test |
| `m i v` / `m a v` | `inner-value` / `around-value` |  | Inner / around array/tuple/struct value |
| `m i x` / `m a x` | `inner-tag` / `around-tag` |  | Inner / around tag pair (structure-aware) |
| `m i i` | `select-last-insertion` |  | Select the text typed during the last insert |
| `m m` | `select-word` |  | Select the word under the cursor, with the same whitespace rule as `w`/`b` |
| `M M` | `select-uppercase-word` |  | WORD variant of `m m` |
| `m s` + char | `surround-paren` (and other `surround-*` delimiters) |  | Select surrounding delimiter pair |
| `m w` + char | `surround-add` |  | Wrap each selection with a delimiter pair |
| `m /` | `select-all-matches` |  | Turn all search matches in the buffer into selections |

### Editing

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `d` | `delete` |  | Delete selection (to kill ring) |
| `c` | `change` |  | Change (delete + Insert mode) |
| `y` | `yank` |  | Yank (clipboard + kill ring) |
| `p` | `smart-paste-after` |  | Paste after (smart paste). See [Copy & Paste](copy-and-paste.md) |
| `P` | `smart-paste-before` |  | Paste before (smart paste) |
| `[` / `]` | `paste-ring-older` / `paste-ring-newer` |  | Cycle kill ring older / newer and re-paste (only after a `p`/`P`) |
| `r` + char | `replace` |  | Replace every selected character except line endings. `Enter`/`Tab` replace with a newline/tab |
| `J` | `join-lines-select-spaces` |  | Join the selected lines into one |
| `&` | `align-selections` |  | Align selections into a column |
| `>` | `indent` |  | Indent lines touched by a selection |
| `<` | `unindent` |  | Unindent lines touched by a selection |
| `Ctrl-a` | `increment` |  | Add the count to the number or date at each selection. See [Numbers and dates](editing.md#numbers-and-dates) |
| `Ctrl-s` | `decrement` |  | Subtract the count from the number or date at each selection |
| `u` | `undo` |  | Undo |
| `U` / `Ctrl-r` | `redo` |  | Redo |
| `.` | `repeat-last-action` |  | Repeat last editing action |
| `\|` | `pipe` |  | Run a shell command on each selection and replace it with the output. See [Shell commands](editing.md#shell-commands) |
| `Ctrl-c` | `cancel-pipe` |  | Stop the shell command running in this buffer and change nothing |

### Entering other modes

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `i` | `insert-at-selection-start` |  | Insert before selection |
| `a` | `insert-at-selection-end` |  | Insert after selection |
| `I` | `insert-at-line-start` |  | Insert at first non-blank on line |
| `A` | `insert-at-line-end` |  | Insert at end of line |
| `o` | `open-line-below` |  | Open new line below, insert (indented like `Enter`) |
| `O` | `open-line-above` |  | Open new line above, insert (indented like `Enter`) |
| `:` | `command-mode` |  | Open the `:` prompt |
| `/` | `search-forward` |  | Search forward |
| `?` | `search-backward` |  | Search backward. `Ctrl-?` (kitty) extends the selection to the match |
| `s` | `sift-within` |  | Narrow each selection to its regex matches |

### Search

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `n` | `search-next` |  | Next match |
| `N` | `search-prev` |  | Previous match |
| `Ctrl-/` | `search-selection` | ✓ | Use the selected text literally as the search pattern |

### Macros

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `Q Q` or `Q q` | — |  | Start recording into default register `q` |
| `Q <0-9>` | — |  | Start recording into a numbered register |
| `Q` (while recording) | — |  | Stop recording |
| `q q` | — |  | Replay register `q` |
| `q <0-9>` | — |  | Replay a numbered register |
| `<count> q q` | — |  | Replay `q` `<count>` times |

Macros are stored in registers; see [Macros](copy-and-paste.md#macros) and [Register prefix](copy-and-paste.md#register-prefix).

### Other

| Key | Action |
|-----|--------|
| `Esc` | Cancel a half-typed key sequence, count, or register/macro prompt; clear search highlights; leave Extend mode |
| `"` + reg | Register prefix (`0`–`9`, `k`, `c`, `b`) |
| `1`–`9` then `[0-9]*` | Numeric count prefix (`0` is a digit only inside a count; otherwise unbound) |

## Goto prefix (`g`)

Press `g` then a second key. Each names a destination: a place in the buffer or, for the structural pairs, the next or previous instance of a kind, selected as a whole:

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `g g` | `goto-first-line` |  | Go to first line of buffer |
| `g e` | `goto-last-line` |  | Go to last line of buffer |
| `g h` | `goto-line-start` |  | Go to line start |
| `g l` | `goto-line-end` |  | Go to line end |
| `g s` | `goto-first-nonblank` |  | Go to first non-blank on line |
| `g f` / `g F` | `goto-next-function` / `goto-prev-function` |  | Next/previous function |
| `g t` / `g T` | `goto-next-class` / `goto-prev-class` |  | Next/previous class or type |
| `g a` / `g A` | `goto-next-argument` / `goto-prev-argument` |  | Next/previous argument |
| `g c` / `g C` | `goto-next-comment` / `goto-prev-comment` |  | Next/previous comment |
| `g u` / `g U` | `goto-next-test` / `goto-prev-test` |  | Next/previous unit test |
| `g v` / `g V` | `goto-next-value` / `goto-prev-value` |  | Next/previous array/tuple/struct value |
| `g x` / `g X` | `goto-next-tag` / `goto-prev-tag` |  | Next/previous tag pair |

The structural pairs need a grammar that supports them, except `g x`/`g X`, which fall back to a plain-text scan. See [Moving Around](moving-around.md#structural-navigation). `core:lsp` adds more keys under `g`; see [Keys added by core plugins](#keys-added-by-core-plugins).

## Edit prefix (`G`)

Press `G` then a second key. `G` holds edits of the selected text, where `g` holds gotos:

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `G L` | `make-text-lowercase` |  | Lowercase the selection |
| `G U` | `make-text-uppercase` |  | Uppercase the selection |
| `G C` | `make-text-capitalized` |  | Capitalize each word in the selection |
| `G G` | `toggle-comment` |  | Comment or uncomment the lines of each selection |
| `G A` | `increment-sequence` |  | Add the count to the first selection's number or date, twice the count to the second's, and so on |
| `G S` | `decrement-sequence` |  | Subtract in the same way as `G A` |

`core:lsp` adds `G R` (rename); see [Keys added by core plugins](#keys-added-by-core-plugins).

## View prefix (`z`)

Press `z` then a second key:

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `z k` | `top-view-on-cursor` |  | Scroll cursor to top of screen |
| `z z` | `center-view-on-cursor` |  | Center view on cursor |
| `z j` | `bottom-view-on-cursor` |  | Scroll cursor to bottom of screen |

`core:pickers` and `core:lsp` add more keys under `z`; see [Keys added by core plugins](#keys-added-by-core-plugins).

## Pane prefix (`Ctrl-p`)

Press `Ctrl-p` then a second key:

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `Ctrl-p p` | `pane-focus-next` |  | Focus next pane |
| `Ctrl-p P` | `pane-focus-prev` |  | Focus previous pane |
| `Ctrl-p h` | `pane-focus-left` |  | Focus pane to the left |
| `Ctrl-p j` | `pane-focus-down` |  | Focus pane below |
| `Ctrl-p k` | `pane-focus-up` |  | Focus pane above |
| `Ctrl-p l` | `pane-focus-right` |  | Focus pane to the right |
| `Ctrl-p s` | `pane-split` |  | Split the focused pane, stacking the new pane below it |
| `Ctrl-p v` | `pane-vsplit` |  | Split the focused pane side by side |
| `Ctrl-p c` | `pane-close` |  | Close the focused pane (does nothing if it's the only pane, and refuses while a `\|` run from it is running) |
| `Ctrl-p r` | `pane-rotate-forward` |  | Move each pane in the focused pane's row or column one place right or down (that many places with a count); the last one wraps to the first |
| `Ctrl-p R` | `pane-rotate-backward` |  | Move each pane in the focused pane's row or column one place left or up (that many places with a count); the first one wraps to the last |
| `Ctrl-p t` | `goto-next-tab` |  | Switch to the next tab in display order |
| `Ctrl-p T` | `goto-prev-tab` |  | Switch to the previous tab in display order |

Under the kitty protocol, two keys also move focus without the prefix:

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `Tab` | `pane-focus-next` | ✓ | Focus next pane |
| `Shift-Tab` | `pane-focus-prev` | ✓ | Focus previous pane |

## Insert mode

| Key | Command | Kitty | Action |
|-----|---------|:-----:|--------|
| `Esc` / `Ctrl-c` | `exit-insert` |  | Return to Normal mode |
| `←` / `→` / `↑` / `↓` | `move-left` / `move-right` / `move-up` / `move-down` |  | Move cursor |
| `Home` / `End` | `goto-line-start` / `goto-line-end` |  | Go to line start / past the last character |
| `Tab` | — |  | Insert a tab (a literal tab, or spaces to the next tab stop when `indent-style` is `soft`) |
| `Backspace` | — |  | Delete character before cursor; snaps to previous tab stop when in leading whitespace (auto-pairs aware) |
| `Delete` | — |  | Delete character under cursor |
| `Enter` | — |  | Insert newline, indented as described in [Indentation](editing.md#indentation) |
| `Ctrl-w` | `delete-word-backward` |  | Delete word before cursor |
| `Ctrl-Space` | `completion-trigger` |  | Show completions at the cursor from every registered completion source, such as a language server (see [Language servers](lsp.md)) |
| `Ctrl-r` | `insert-register` |  | Insert the register named by the next key, or the file's path (`f`) or directory (`p`); see [Inserting a register while typing](copy-and-paste.md#inserting-a-register-while-typing) |
| `Ctrl-\|` | `pipe` | ✓ | Run a shell command and insert its output at each cursor; see [Shell commands](editing.md#shell-commands) |
| Any other character | — |  | Insert character (auto-pairs aware) |

Typing an opening bracket or quote can also insert its closing character; see [Auto-pairs](editing.md#auto-pairs).

While a completion menu is open, typing narrows it and these keys drive it; any other key closes it:

| Key | Action |
|-----|--------|
| `Tab` / `↓` | Highlight the next entry |
| `Shift-Tab` / `↑` | Highlight the previous entry |
| `Enter` | Accept the highlighted entry |
| `Esc` | Close the menu, stay in Insert mode |
| `Ctrl-d` / `Ctrl-u` | Scroll the documentation box down / up |

A box beside the menu shows the highlighted entry's details and documentation, when it has any. The box needs room to the right or left of the menu. Without that room, or with `:set global completion-docs=false`, the details show in the menu instead.

Binding one of these keys in Insert mode replaces its behaviour. A plugin command can still fall back to it; see [Editing & navigation](plugin-api.md#editing-navigation).

## Extend mode

| Key | Action |
|-----|--------|
| `e` / `Esc` | Return to Normal mode |
| All other keys | Same as Normal mode, but motions extend or shrink the selection |

The statusline shows `EXT` in Extend mode.

## Command mode

Open the `:` prompt with `:` from Normal mode.

| Key | Action |
|-----|--------|
| `Enter` (empty input) | Close the `:` prompt |
| `Enter` (non-empty) | Execute command |
| `Esc` / `Ctrl-c` | Cancel |
| `Tab` | Complete; once the candidate list is open, pick the next candidate |
| `Shift-Tab` | Once the candidate list is open, pick the previous candidate (no-op otherwise) |
| `Up` / `Down` | Recall previous / next command starting with the typed prefix |
| `Left` / `Right` | Move the cursor |
| `Backspace` | Delete character before cursor; on empty input, close the `:` prompt |
| `Ctrl-w` | Delete word before cursor |
| `Ctrl-r` | Insert a register, or the file's path (`f`) or directory (`p`), as in Insert mode |

A key you bind in `'command` mode replaces these; see [Keys in the `:` prompt](configuration.md#keys-in-the-command-line).

## Search mode

Open it with `/` (forward) or `?` (backward). Each keystroke previews the next match from where the search started.

| Key | Action |
|-----|--------|
| `Enter` (empty input) | Cancel and return to Normal |
| `Enter` (non-empty) | Commit pattern to the search register, jump to first match, return to Normal |
| `Esc` / `Ctrl-c` | Cancel, restore pre-search selection |
| `Backspace` (input non-empty) | Delete char and re-preview |
| `Backspace` (empties input) | Restore pre-search selection, stay in Search mode |
| `Backspace` (on empty input) | Exit Search mode |
| `Ctrl-w` | Delete word before cursor |
| `Up` / `Down` | Recall previous / next pattern starting with the typed prefix (separate `/` and `?` rings) |
| `Left` / `Right` | Move the cursor |
| Any other character | Insert and re-preview |
| `Tab` / `Shift-Tab` | No-op |

## Sift mode

Open it with `s` from Normal mode; at least one selection must be wider than a single character. As you type, the matches inside the original selections preview as new selections. Sift mode leaves the search register alone, so after `Enter`, `n`/`N` continue the previous search.

| Key | Action |
|-----|--------|
| `Enter` (empty input) | Cancel, restore original selections |
| `Enter` (non-empty) | Keep the live-preview selections, return to Normal |
| `Esc` / `Ctrl-c` | Cancel, restore original selections |
| `Backspace` (input non-empty) | Delete char and re-preview |
| `Backspace` (empties input or on empty) | Restore original selections, stay in Sift mode |
| `Ctrl-w` | Delete word before cursor |
| `Left` / `Right` | Move the cursor |
| Any other character | Insert and re-preview |
| `Tab` / `Shift-Tab` / `Up` / `Down` | No-op (Sift mode has no pattern history) |

## Other screens

### Pickers

The default keys of every picker that uses `core:stdlib`'s picker keymap, the file and buffer pickers of [`core:pickers`](pickers.md) included. Typing filters the list.

| Key | Action |
|-----|--------|
| `↓` / `Ctrl-n` | Select the next item |
| `↑` / `Ctrl-p` | Select the previous item |
| `PageDown` / `PageUp` | Page the list |
| `Ctrl-d` / `Ctrl-u` | Move the selection by half a page |
| `Enter` / `Ctrl-o` | Open the selected item in the current pane |
| `Ctrl-t` | Open the selected item in a new tab |
| `Ctrl-v` | Open the selected item in a side-by-side split |
| `Ctrl-s` | Open the selected item in a stacked split |
| `Esc` | Close without opening anything |

### Drawer

The panel at the bottom of the screen that lists buffers (`:ls`), diagnostics, references and other results.

| Key | Action |
|-----|--------|
| `↓` / `↑` | Select the next / previous row |
| `Ctrl-d` / `Ctrl-u` | Move the selection by half the drawer's height |
| `Enter` | Act on the selected row, such as jumping to it |
| `Esc` | Close the drawer |

Any other key works as usual, and the drawer stays open.

### Popups

In a hover or diagnostic popup longer than its box, `Ctrl-d` / `Ctrl-u` scroll it down / up. Any other key closes the popup and then works as usual.

### Menus

Menus that plugins open, such as code actions:

| Key | Action |
|-----|--------|
| `j` / `↓` | Highlight the next entry |
| `k` / `↑` | Highlight the previous entry |
| `Enter` | Choose the highlighted entry |
| `Esc` | Close without choosing |

Any other key closes the menu and then works as usual.

### Questions in the statusline

When a file you have open changes on disk, `r` reloads it and `k` keeps your version. `Esc` dismisses the question, which comes back the next time you enter the buffer. See [External changes](files-and-buffers.md#external-changes).

When a file you open has a crash dump, `r` restores the dump, `d` discards it and `k` keeps it for later. `Esc` keeps it. See [Crash recovery](files-and-buffers.md#crash-recovery).

Any other key dismisses the question and then works as usual.

## Keys added by core plugins

These keys exist once you load the plugin; no core plugin loads on its own. See [Core Plugins](core-plugins.md).

| Plugin | Key | Action |
|--------|-----|--------|
| `core:lsp` | `g d` / `g D` | Go to definition / declaration |
| `core:lsp` | `g y` / `g i` | Go to type definition / implementation |
| `core:lsp` | `g n` / `g p` | Next / previous diagnostic |
| `core:lsp` | `K` | Hover |
| `core:lsp` | `z r` | References |
| `core:lsp` | `z a` | Code actions |
| `core:lsp` | `z s` / `z S` | Document / workspace symbols |
| `core:lsp` | `G R` | Rename |
| `core:pickers` | `z f` | Pick a file |
| `core:pickers` | `z b` | Pick an open buffer |
| `core:pickers` | `z m` | Pick a modified file in the git repository |

`core:vim-keybind`, `core:helix-surround` and `core:classic-paste` change existing keys instead; each one's section in [Core Plugins](core-plugins.md) lists them.
