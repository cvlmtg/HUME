# Default Keys

Every default key, by mode. Use your browser's search — or the search box above — to find one fast.

::: info
Keys marked **kitty only** require the kitty keyboard protocol, auto-detected at startup on supported terminals — see [Terminal compatibility](installation.md#terminal-compatibility). Legacy terminal encodings cannot transmit those key combinations, so the bindings are unavailable there.
:::

## Normal mode

### Movement

| Key | Command | Action |
|-----|---------|--------|
| `h` / `←` | `move-left` | Move left one grapheme |
| `l` / `→` | `move-right` | Move right one grapheme |
| `j` / `↓` | `move-down` | Move down one visual line |
| `k` / `↑` | `move-up` | Move up one visual line |
| `w` | `select-next-word` | Select next word (plus one adjacent whitespace run by default — see `word-selects-whitespace`) |
| `b` | `select-prev-word` | Select previous word (plus one adjacent whitespace run by default) |
| `W` / `B` | `select-next-uppercase-word` / `select-prev-uppercase-word` | WORD variants of `w` / `b` |
| `Home` | `goto-line-start` | Start of line (idiomatic form is `g h`) |
| `End` | `goto-line-end` | End of line (idiomatic form is `g l`) |
| `{` | `goto-prev-paragraph` | Select the previous paragraph |
| `}` | `goto-next-paragraph` | Select the next paragraph |
| `#` | `goto-matching-pair` | Jump to the matching bracket or tag |
| `PageDown` / `PageUp` | `page-down` / `page-up` | Scroll one viewport down / up |
| `Ctrl+d` / `Ctrl+u` | `half-page-down` / `half-page-up` | Scroll half a viewport down / up |
| `Ctrl+o` | `jump-backward` | Jump list back |
| `Ctrl+i` | `jump-forward` | Jump list forward |
| `Tab` | `jump-forward` (legacy) / `pane-focus-next` (kitty) | Jump list forward (under kitty, `Tab` focuses the next pane instead) |
| `Ctrl+h/j/k/l/w/b` | `move-left` / `move-right` / `move-down` / `move-up` / `select-next-word` / `select-prev-word` (extend) | One-shot extend of the corresponding motion (kitty only) |

### Character find

| Key | Command | Action |
|-----|---------|--------|
| `f` + char | `find-forward` | Find char forward (inclusive) |
| `F` + char | `find-backward` | Find char backward (inclusive) |
| `t` + char | `till-forward` | Find char forward (exclusive — stops before) |
| `T` + char | `till-backward` | Find char backward (exclusive — stops after) |
| `=` | `repeat-find-forward` | Repeat last find forward |
| `-` | `repeat-find-backward` | Repeat last find backward |

After `f`/`F`/`t`/`T`, HUME waits for the target character. `Tab` counts as a
target character; a line ending never does, since these motions never leave
the current line.

### Selection

| Key | Command | Action |
|-----|---------|--------|
| `e` | `toggle-extend` | Toggle Extend mode |
| `;` | `collapse-and-exit-extend` | Collapse to head, exit Extend |
| `Ctrl+;` | `collapse-to-anchor-and-exit-extend` | Collapse to anchor, exit Extend (kitty only) |
| `Ctrl+e` | `flip-selections` | Swap anchor and head |
| `%` | `select-all` | Select entire buffer |
| `x` | `select-line` | Select current line (forward) |
| `X` | `select-line-backward` | Select current line (backward) |
| `Ctrl+x` | `select-line` (extend) | Same as `x` but always extends |
| `Ctrl+X` | `select-line-backward` (extend) | Same as `X` but always extends (kitty only) |
| `S` | `split-selection-on-newlines` | Split multi-line selections on newlines |
| `C` | `copy-selection-on-next-line` | Copy each selection to the line below (a count prefix copies onto that many lines, e.g. `3C`) |
| `_` | `trim-selection-whitespace` | Trim leading/trailing whitespace from each selection |
| `,` | `keep-primary-selection` | Keep only the primary selection |
| `Ctrl+,` | `remove-primary-selection` | Remove primary, promote next (kitty only) |
| `(` / `)` | `cycle-primary-backward` / `cycle-primary-forward` | Cycle primary backward / forward |
| `*` | `search-word-under-cursor` | Search the whole word under the cursor |

Text objects (use the `m` prefix):

| Sequence | Command | Action |
|----------|---------|--------|
| `m i w` / `m a w` | `inner-word` / `around-word` | Inner / around word |
| `m i W` / `m a W` | `inner-uppercase-word` / `around-uppercase-word` | Inner / around WORD |
| `m i (` / `m a (` | `inner-paren` / `around-paren` | Inner / around `()` (also `)`, `[`/`]`, `{`/`}`, `<`/`>`) |
| `m i "` / `m a "` | `inner-double-quote` / `around-double-quote` | Inner / around `"…"` |
| `m i '` / `m a '` | `inner-single-quote` / `around-single-quote` | Inner / around `'…'` |
| `` m i ` `` / `` m a ` `` | `inner-backtick` / `around-backtick` | Inner / around `` `…` `` |
| `m i a` / `m a a` | `inner-argument` / `around-argument` | Inner / around argument (structure-aware) |
| `m i l` / `m a l` | `inner-line` / `around-line` | Inner / around line |
| `m i p` / `m a p` | `inner-paragraph` / `around-paragraph` | Inner / around paragraph |
| `m i f` / `m a f` | `inner-function` / `around-function` | Inner / around function |
| `m i t` / `m a t` | `inner-class` / `around-class` | Inner / around class or type |
| `m i c` / `m a c` | `inner-comment` / `around-comment` | Inner / around comment |
| `m i u` / `m a u` | `inner-test` / `around-test` | Inner / around unit test |
| `m i v` / `m a v` | `inner-value` / `around-value` | Inner / around array/tuple/struct value |
| `m i i` | `select-last-insertion` | Select the text typed during the last insert |
| `m m` | `select-word` | Select the word under the cursor (plus one adjacent whitespace run by default, same rule as `w`/`b` — see `word-selects-whitespace`) |
| `M M` | `select-uppercase-word` | WORD variant of `m m` |
| `m s` + char | `surround-paren` (and other `surround-*` delimiters) | Select surrounding delimiter pair |
| `m w` + char | `surround-add` | Wrap each selection with a delimiter pair |
| `m /` | `select-all-matches` | Turn all search matches in the buffer into selections |

### Editing

| Key | Command | Action |
|-----|---------|--------|
| `d` | `delete` | Delete selection (to kill ring) |
| `c` | `change` | Change (delete + Insert mode) |
| `y` | `yank` | Yank (clipboard + kill ring) |
| `p` | `smart-paste-after` | Smart-paste after — see [Copy & Paste](copy-and-paste.md) |
| `P` | `smart-paste-before` | Smart-paste before |
| `[` / `]` | `paste-ring-older` / `paste-ring-newer` | Cycle kill ring older / newer and re-paste (only after a `p`/`P`) |
| `r` + char | `replace` | Replace every selected character (line endings are left alone). `Enter`/`Tab` count as the character, replacing with a newline/tab |
| `J` | `join-lines-select-spaces` | Join the selected lines into one |
| `&` | `align-selections` | Align selections into a column |
| `>` | `indent` | Indent lines touched by a selection |
| `<` | `unindent` | Unindent lines touched by a selection |
| `u` | `undo` | Undo |
| `U` / `Ctrl+r` | `redo` | Redo |
| `.` | `repeat-last-action` | Repeat last editing action |

### Entering other modes

| Key | Command | Action |
|-----|---------|--------|
| `i` | `insert-at-selection-start` | Insert before selection |
| `a` | `insert-at-selection-end` | Insert after selection |
| `I` | `insert-at-line-start` | Insert at first non-blank on line |
| `A` | `insert-at-line-end` | Insert at end of line |
| `o` | `open-line-below` | Open new line below, insert |
| `O` | `open-line-above` | Open new line above, insert |
| `:` | `command-mode` | Open command mode prompt |
| `/` | `search-forward` | Search forward |
| `?` | `search-backward` | Search backward |

### Search

| Key | Command | Action |
|-----|---------|--------|
| `n` | `search-next` | Next match |
| `N` | `search-prev` | Previous match |
| `s` | `select-within` | Select within (regex filter on each selection) |
| `Ctrl+/` | `search-selection` | Use the selected text literally as the search pattern (kitty only) |

### Macros

| Key | Command | Action |
|-----|---------|--------|
| `Q Q` or `Q q` | — | Start recording into default register `q` |
| `Q <0-9>` | — | Start recording into a numbered register |
| `Q` (while recording) | — | Stop recording |
| `q q` | — | Replay register `q` |
| `q <0-9>` | — | Replay a numbered register |
| `<count> q q` | — | Replay `q` `<count>` times |

Numbered registers are shared between macros and yanked text — last write wins. Recording is ignored in read-only buffers and during replay.

See [Register prefix](copy-and-paste.md#register-prefix) for the full register list.

### Other

| Key | Action |
|-----|--------|
| `"` + reg | Register prefix (`0`–`9`, `k`, `c`, `b`) |
| `1`–`9` then `[0-9]*` | Numeric count prefix (`0` is a digit only inside a count; otherwise unbound) |

## Goto prefix (`g`)

Press `g` then a second key. Every one names a destination — a place in the buffer, or (for
the structural pairs) the next/previous instance of a kind, selected as a whole:

| Key | Command | Action |
|-----|---------|--------|
| `g g` | `goto-first-line` | Go to first line of buffer |
| `g e` | `goto-last-line` | Go to last line of buffer |
| `g h` | `goto-line-start` | Go to line start |
| `g l` | `goto-line-end` | Go to line end |
| `g s` | `goto-first-nonblank` | Go to first non-blank on line |
| `g f` / `g F` | `goto-next-function` / `goto-prev-function` | Next/previous function |
| `g t` / `g T` | `goto-next-class` / `goto-prev-class` | Next/previous class or type |
| `g a` / `g A` | `goto-next-argument` / `goto-prev-argument` | Next/previous argument |
| `g c` / `g C` | `goto-next-comment` / `goto-prev-comment` | Next/previous comment |
| `g u` / `g U` | `goto-next-test` / `goto-prev-test` | Next/previous unit test |
| `g v` / `g V` | `goto-next-value` / `goto-prev-value` | Next/previous array/tuple/struct value |

The structural pairs need a grammar with a `textobjects.scm` — see [Moving Around](moving-around.md#structural-navigation).

## `G` prefix

Press `G` then a second key. Not a "case prefix" — `G` holds the commands Vim files under
`g` that aren't gotos (`G L`/`G U`/`G C` are Vim's `gu`/`gU`/`g~`):

| Key | Command | Action |
|-----|---------|--------|
| `G L` | `make-text-lowercase` | Lowercase the selection |
| `G U` | `make-text-uppercase` | Uppercase the selection |
| `G C` | `make-text-capitalized` | Capitalize each word in the selection |

`G U`/`G C` differ from `g U`/`g C` (previous unit test / previous comment) only in the prefix's
case — worth knowing before it's muscle memory. `core:lsp` adds a fourth key here, `G R` for
`lsp-rename` — see [Language Servers](lsp.md).

## View prefix (`z`)

Press `z` then a second key:

| Key | Command | Action |
|-----|---------|--------|
| `z k` | `top-view-on-cursor` | Scroll cursor to top of screen |
| `z z` | `center-view-on-cursor` | Center view on cursor |
| `z j` | `bottom-view-on-cursor` | Scroll cursor to bottom of screen |

`k` is up and `j` is down, the same axis the motion keys use. `core:pickers` and `core:lsp`
add more keys under `z` — see [Fuzzy Finder](pickers.md) and [Language Servers](lsp.md).

## Pane prefix (`Ctrl+p`)

Press `Ctrl+p` then a second key:

| Key | Command | Action |
|-----|---------|--------|
| `Ctrl+p p` | `pane-focus-next` | Focus next pane |
| `Ctrl+p h` | `pane-focus-left` | Focus pane to the left |
| `Ctrl+p j` | `pane-focus-down` | Focus pane below |
| `Ctrl+p k` | `pane-focus-up` | Focus pane above |
| `Ctrl+p l` | `pane-focus-right` | Focus pane to the right |
| `Ctrl+p s` | `pane-split` | Split the focused pane, stacking the new pane below it |
| `Ctrl+p v` | `pane-vsplit` | Split the focused pane side by side |
| `Ctrl+p c` | `pane-close` | Close the focused pane (does nothing if it's the only pane) |
| `Tab` | `pane-focus-next` | Focus next pane (kitty only) |

## Insert mode

| Key | Command | Action |
|-----|---------|--------|
| `Esc` / `Ctrl+c` | `exit-insert` | Return to Normal mode |
| `←` / `→` / `↑` / `↓` | `move-left` / `move-right` / `move-up` / `move-down` | Move cursor |
| `Home` / `End` | `goto-line-start` / `goto-line-end` | Go to line start / end |
| `Tab` | — | Insert tab (literal `\t`, or spaces to the next tab stop when `tab-style = soft`) |
| `Backspace` | — | Delete character before cursor; snaps to previous tab stop when in leading whitespace (auto-pairs aware) |
| `Delete` | — | Delete character under cursor |
| `Enter` | — | Insert newline, copying leading whitespace from current line (auto-pairs aware) |
| `Ctrl+w` | `delete-word-backward` | Delete word before cursor |
| Any other character | — | Insert character (auto-pairs aware) |

Insert mode handles auto-pair insertion: typing `(`, `[`, `{`, `"`, `'`, or `` ` `` inserts the matching close character. Backspace inside an empty pair deletes both characters.

## Extend mode

| Key | Action |
|-----|--------|
| All other keys | Same as Normal mode, but motions extend or shrink the selection |

The status bar shows `EXT` in Extend mode.

## Command line

| Key | Action |
|-----|--------|
| `Enter` | Execute command |
| `Esc` / `Ctrl+c` | Cancel |
| `Tab` | Complete |
| `Shift+Tab` | Complete (previous) |
| `Up` / `Down` | Recall previous / next command starting with the typed prefix |
| `Left` / `Right` | Move the cursor |
| `Backspace` | Delete character before cursor; on empty input, dismiss the command mode prompt |
| `Ctrl+w` | Delete word before cursor |

## Search mode

Entered with `/` (forward) or `?` (backward). Every keystroke live-previews the next match from the position where search was opened.

| Key | Action |
|-----|--------|
| `Enter` (empty input) | Cancel and return to Normal |
| `Enter` (non-empty) | Commit pattern to the search register, jump to first match, return to Normal |
| `Esc` / `Ctrl+c` | Cancel, restore pre-search selection |
| `Backspace` (input non-empty) | Delete char and re-preview |
| `Backspace` (empties input) | Restore pre-search selection, stay in Search mode |
| `Backspace` (on empty input) | Exit Search mode |
| `Ctrl+w` | Delete word before cursor |
| `Up` / `Down` | Recall previous / next pattern starting with the typed prefix (separate `/` and `?` rings) |
| `Left` / `Right` | Move the cursor |
| Any other character | Insert and re-preview |
| `Tab` / `Shift+Tab` | No-op |

## Select mode

Entered with `s` from Normal mode (requires at least one non-collapsed selection). Live-previews sub-match selections within the original selections. Does **not** overwrite the search register, so `n`/`N` continue the prior search after `Enter`.

| Key | Action |
|-----|--------|
| `Enter` (empty input) | Cancel, restore original selections |
| `Enter` (non-empty) | Keep the live-preview selections, return to Normal |
| `Esc` / `Ctrl+c` | Cancel, restore original selections |
| `Backspace` (input non-empty) | Delete char and re-preview |
| `Backspace` (empties input or on empty) | Restore original selections, stay in Select mode |
| `Ctrl+w` | Delete word before cursor |
| `Left` / `Right` | Move the cursor |
| Any other character | Insert and re-preview |
| `Tab` / `Shift+Tab` / `Up` / `Down` | No-op (Select mode has no pattern history) |
