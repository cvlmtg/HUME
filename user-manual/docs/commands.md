# Commands

Anything that doesn't deserve a keystroke lives behind `:`. Press `:` in Normal mode to open the command line, type a name, and press `Enter`. `Esc` dismisses without running.

Most commands have a short alias — both forms are listed below, and both work. Press `Tab` at any point for completion of names and, where it makes sense, arguments.

For running HUME from a shell instead, see [Command-line Flags](cli.md).

## Quitting and saving

| Command | Effect |
|---------|--------|
| `:q`, `:quit` | Close the focused pane; with one pane, close the buffer, and quit HUME when it's the last one |
| `:q!` | Same, discarding unsaved changes |
| `:qa`, `:quit-all` | Quit everything. Refuses if any buffer has unsaved changes, and jumps to the first one |
| `:qa!` | Quit everything, discarding unsaved changes |
| `:w`, `:write` | Save |
| `:w <path>` | Save as |
| `:w!` | Save, retrying with a permission change if the first attempt is refused; also overwrites a file changed on disk since it was last read or saved |
| `:wa`, `:write-all` | Save every modified buffer |
| `:wq`, `:write-quit` | Save, then close the focused pane; with one pane, close the buffer, and quit HUME when it's the last one |

Relative paths given to `:w` resolve against HUME's working directory (`:pwd`), not the shell's.

## Files

| Command | Effect |
|---------|--------|
| `:e <path>`, `:edit <path>` | Open a file |
| `:e` | Reload the current file. Refuses if there are unsaved changes |
| `:e!` | Reload, discarding unsaved changes |
| `:checktime` | Check every open buffer against its file on disk right now, instead of waiting for the next automatic check (switching back to HUME, switching to the buffer) |

In arguments, `%` expands to the current file's path and `#` to the alternate file's. Both only work as a whole argument — `:w %.bak` won't expand. `:b` is the exception: it resolves `#` itself, which is why `:b #` works for buffers with no file on disk.

See [Files & Buffers](files-and-buffers.md#external-changes) for what happens when a file changes on disk.

## Buffers

| Command | Effect |
|---------|--------|
| `:ls`, `:list-buffers` | List open buffers |
| `:b <name>`, `:buffer <name>` | Switch buffer. Accepts a name, a unique filename prefix, a full path, a number from `:ls`, or `#` for the previous buffer |
| `:bn`, `:bnext` | Next buffer |
| `:bp`, `:bprev` | Previous buffer |
| `:bd`, `:buffer-delete` | Close the buffer. Refuses if there are unsaved changes; closing the last one leaves a scratch buffer |
| `:bd!` | Close the buffer, discarding unsaved changes |

`:b #` is the quickest way back to the previous buffer. There's no default key for it, but `:goto-alternate-file` can be bound to one — and `core:vim-keybind` binds it to `Ctrl+6` for you.

## Panes

| Command | Effect |
|---------|--------|
| `:sp`, `:split` | Split the focused pane, stacking the new pane below |
| `:vsp`, `:vsplit` | Split the focused pane side by side |

Splitting is refused with a message when the pane is already too small. Focus and closing use the `Ctrl+p` prefix (`Ctrl+p` then `h`/`j`/`k`/`l`/`p`/`s`/`v`/`c`) — see the [Default Keys](default-keys.md).

## Editing

| Command | Effect |
|---------|--------|
| `:sort` | Sort adjacent rows by their selected text |
| `:sort -r` | Sort in reverse |
| `:sort -i` | Sort case-insensitively |

`:sort` groups your selections into runs of adjacent rows and sorts each run
independently, keyed by whatever text you selected on that row — not
necessarily the whole line. Select whole lines (`%` selects the whole buffer)
to sort a file; select a single column across a block of lines to sort by
that column instead. A run sorts numerically only if *every* row's key looks
like a number — one row with non-numeric text drops the whole run back to
plain text order. Two selections on the same row combine into one key rather
than being treated as separate rows, so sorting several items *within* one
line isn't what `:sort` does.

A single selection that spans several rows keeps its place in the buffer —
the rows underneath it reorder, but the selection itself still covers the
same stretch of text afterward.

<div class="key-demo">
<strong>Select the count on each row, then run <code>:sort</code></strong><br>
mia&nbsp;&nbsp;&nbsp;<span class="sel">3<span class="head">4</span></span><br>
zoe&nbsp;&nbsp;&nbsp;<span class="sel">1<span class="head">9</span></span><br>
kai&nbsp;&nbsp;&nbsp;<span class="sel">5<span class="head">2</span></span><br>
<br>
<strong>Result</strong><br>
zoe&nbsp;&nbsp;&nbsp;19<br>
mia&nbsp;&nbsp;&nbsp;34<br>
kai&nbsp;&nbsp;&nbsp;52
</div>

## Settings

| Command | Effect |
|---------|--------|
| `:set global <option>=<value>` | Set an option everywhere |
| `:set buffer <option>=<value>` | Set an option for this buffer only |
| `:set pane <option>=<value>` | Set an option for this pane only (`wrap-mode` only) |

See [Configuration](configuration.md) for every option.

## Display

| Command | Effect |
|---------|--------|
| `:theme <name>` | Load a theme; with no argument, show the current one |
| `:theme-debug` | Show the resolved styles for the main UI scopes |
| `:wrap`, `:toggle-soft-wrap` | Toggle line wrapping in this pane — see [wrap styles](configuration.md#text-wrap) |
| `:mes`, `:messages` | Show the message log in a read-only buffer |
| `:clear-search` | Clear search highlights (`Esc` also clears them) |

## Navigation

| Command | Effect |
|---------|--------|
| `:cd <path>`, `:change-directory <path>` | Change the working directory |
| `:pwd`, `:print-working-directory` | Print the working directory |
| `:goto <n>` | Jump to line `<n>`. `:42` is shorthand. Records a jump, so `Ctrl+o` comes back |

## Config and plugins

| Command | Effect |
|---------|--------|
| `:plugins`, `:plugin-status` | Show declared plugins and whether they've loaded |
| `:reload-config` | Re-run `init.scm` from scratch |
| `:ver`, `:version` | Show the editor version |
| `:tutor` | Open the interactive tutorial |

Plugins add commands of their own once you load them. The `:plum-*` commands (installing plugins and grammars) come from `core:plum`, and the `:lsp-*` and `:diagnostics` commands from `core:lsp` — neither is loaded until you ask for it in `init.scm`. See [Core Plugins](core-plugins.md).

## Finding a command

There is no listing command. Open the command line with `:` and press `Tab` — completion shows every registered name and alias, including stubs for plugins that haven't loaded yet.

## Running key commands from `:`

Every command that can be bound to a key can also be typed at `:`, even without a short alias — `:undo`, `:select-all-matches`, `:goto-alternate-file`, and so on. Typed this way they take no count and run once.
