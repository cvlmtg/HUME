# Key Reference

Every default key, by mode. Use your browser's search — or the search box above — to find one fast.

::: info
Keys marked **kitty only** require the kitty keyboard protocol, auto-detected at startup on supported terminals — see [Terminal compatibility](installation.md#terminal-compatibility). Legacy terminal encodings cannot transmit those key combinations, so the bindings are unavailable there.
:::

## Normal mode

### Movement

| Key | Action |
|-----|--------|
| `h` / `←` | Move left one grapheme |
| `l` / `→` | Move right one grapheme |
| `j` / `↓` | Move down one visual line |
| `k` / `↑` | Move up one visual line |
| `w` | Select next word (plus one adjacent whitespace run by default — see `word-selects-whitespace`) |
| `b` | Select previous word (plus one adjacent whitespace run by default) |
| `W` / `B` | WORD variants of `w` / `b` |
| `Home` | Start of line (idiomatic form is `g h`) |
| `End` | End of line (idiomatic form is `g l`) |
| `{` | Up to the blank line above this paragraph |
| `}` | Down to the start of the next paragraph |
| `PageDown` / `PageUp` | Scroll one viewport down / up |
| `Ctrl+d` / `Ctrl+u` | Scroll half a viewport down / up |
| `Ctrl+o` | Jump list back |
| `Ctrl+i` | Jump list forward |
| `Tab` | Jump list forward (under kitty, `Tab` focuses the next pane instead) |
| `Ctrl+h/j/k/l/w/b` | One-shot extend of the corresponding motion (kitty only) |

### Character find

| Key | Action |
|-----|--------|
| `f` + char | Find char forward (inclusive) |
| `F` + char | Find char backward (inclusive) |
| `t` + char | Find char forward (exclusive — stops before) |
| `T` + char | Find char backward (exclusive — stops after) |
| `=` | Repeat last find forward |
| `-` | Repeat last find backward |

After `f`/`F`/`t`/`T`, HUME waits for the target character.

### Selection

| Key | Action |
|-----|--------|
| `e` | Toggle Extend mode |
| `;` | Collapse to head, exit Extend |
| `Ctrl+;` | Collapse to anchor, exit Extend (kitty only) |
| `Ctrl+e` | Swap anchor and head |
| `%` | Select entire buffer |
| `x` | Select current line (forward) |
| `X` | Select current line (backward) |
| `Ctrl+x` | Same as `x` but always extends |
| `Ctrl+X` | Same as `X` but always extends (kitty only) |
| `S` | Split multi-line selections on newlines |
| `C` | Copy each selection to the line below |
| `_` | Trim leading/trailing whitespace from each selection |
| `,` | Keep only the primary selection |
| `Ctrl+,` | Remove primary, promote next (kitty only) |
| `(` / `)` | Cycle primary backward / forward |
| `*` | Search the whole word under the cursor |

Text objects (use the `m` prefix):

| Sequence | Action |
|----------|--------|
| `m i w` / `m a w` | Inner / around word |
| `m i W` / `m a W` | Inner / around WORD |
| `m i (` / `m a (` | Inner / around `()` (also `)`, `[`/`]`, `{`/`}`, `<`/`>`) |
| `m i "` / `m a "` | Inner / around `"…"` |
| `m i '` / `m a '` | Inner / around `'…'` |
| `` m i ` `` / `` m a ` `` | Inner / around `` `…` `` |
| `m i a` / `m a a` | Inner / around argument |
| `m i l` / `m a l` | Inner / around line |
| `m i i` | Select the text typed during the last insert |
| `m m` | Select the word under the cursor (plus one adjacent whitespace run by default, same rule as `w`/`b` — see `word-selects-whitespace`) |
| `M M` | WORD variant of `m m` |
| `m s` + char | Select surrounding delimiter pair |
| `m w` + char | Wrap each selection with a delimiter pair |
| `m /` | Turn all search matches in the buffer into selections |

### Editing

| Key | Action |
|-----|--------|
| `d` | Delete selection (to kill ring) |
| `c` | Change (delete + Insert mode) |
| `y` | Yank (clipboard + kill ring) |
| `p` | Paste after (smart source) |
| `P` | Paste before |
| `[` / `]` | Cycle kill ring older / newer and re-paste (only after a `p`/`P`) |
| `r` + char | Replace every selected character (line endings are left alone) |
| `J` | Join the selected lines into one |
| `&` | Align selections into a column |
| `u` | Undo |
| `U` / `Ctrl+r` | Redo |
| `.` | Repeat last editing action |

### Entering other modes

| Key | Action |
|-----|--------|
| `i` | Insert before selection |
| `a` | Insert after selection |
| `I` | Insert at first non-blank on line |
| `A` | Insert at end of line |
| `o` | Open new line below, insert |
| `O` | Open new line above, insert |
| `:` | Open command line |
| `/` | Search forward |
| `?` | Search backward |

### Search

| Key | Action |
|-----|--------|
| `n` | Next match |
| `N` | Previous match |
| `s` | Select within (regex filter on each selection) |
| `Ctrl+/` | Use the selected text literally as the search pattern (kitty only) |

### Macros

| Key | Action |
|-----|--------|
| `Q Q` or `Q q` | Start recording into default register `q` |
| `Q <0-9>` | Start recording into a numbered register |
| `Q` (while recording) | Stop recording |
| `q q` | Replay register `q` |
| `q <0-9>` | Replay a numbered register |
| `<count> q q` | Replay `q` `<count>` times |

Numbered registers are shared between macros and yanked text — last write wins. Recording is ignored in read-only buffers and during replay.

See [Register prefix](editing.md#register-prefix) for the full register list.

### Other

| Key | Action |
|-----|--------|
| `"` + reg | Register prefix (`0`–`9`, `k`, `c`, `b`) |
| `1`–`9` then `[0-9]*` | Numeric count prefix (`0` is a digit only inside a count; otherwise unbound) |

## Goto prefix (`g`)

Press `g` then a second key:

| Key | Action |
|-----|--------|
| `g g` | Go to first line of buffer |
| `g e` | Go to last line of buffer |
| `g h` | Go to line start |
| `g l` | Go to line end |
| `g s` | Go to first non-blank on line |
| `g u` | Lowercase the selection |
| `g U` | Uppercase the selection |
| `g C` | Capitalize each word in the selection |

## View prefix (`z`)

Press `z` then a second key:

| Key | Action |
|-----|--------|
| `z z` | Center view on cursor |
| `z t` | Scroll cursor to top of screen |
| `z b` | Scroll cursor to bottom of screen |

## Pane prefix (`Ctrl+p`)

Press `Ctrl+p` then a second key:

| Key | Action |
|-----|--------|
| `Ctrl+p p` | Focus next pane |
| `Ctrl+p h` | Focus pane to the left |
| `Ctrl+p j` | Focus pane below |
| `Ctrl+p k` | Focus pane above |
| `Ctrl+p l` | Focus pane to the right |
| `Ctrl+p s` | Split the focused pane, stacking the new pane below it |
| `Ctrl+p v` | Split the focused pane side by side |
| `Ctrl+p c` | Close the focused pane (does nothing if it's the only pane) |
| `Tab` | Focus next pane (kitty only) |

## Insert mode

| Key | Action |
|-----|--------|
| `Esc` / `Ctrl+c` | Return to Normal mode |
| `←` / `→` / `↑` / `↓` | Move cursor |
| `Home` / `End` | Go to line start / end |
| `Tab` | Insert tab (literal `\t`, or spaces to the next tab stop when `tab-style = soft`) |
| `Backspace` | Delete character before cursor; snaps to previous tab stop when in leading whitespace (auto-pairs aware) |
| `Delete` | Delete character under cursor |
| `Enter` | Insert newline, copying leading whitespace from current line (auto-pairs aware) |
| `Ctrl+w` | Delete word before cursor |
| Any other character | Insert character (auto-pairs aware) |

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
| `Up` / `Down` | History |
| `Left` / `Right` | Move the cursor |
| `Backspace` | Delete character before cursor; on empty input, dismiss the command line |
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
| `Up` / `Down` | Recall previous / next pattern from history (separate `/` and `?` rings) |
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
