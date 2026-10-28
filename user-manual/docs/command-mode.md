# Command mode

Command mode runs commands that aren't bound to a key. Press `:` in Normal mode to open the command mode prompt, type a name, and press `Enter`. `Esc` dismisses it without running anything.

Every command in this manual is one of two kinds, each reachable only its own way:

- **Typed commands**: everything on this page. Reached only from the command mode prompt. Each takes a text argument, a `!`, both or neither, as its entry shows, and refuses one it doesn't take.
- **Editor commands**: everything in the [Editor Commands](editor-commands.md) reference. Reached only from a key or a plugin (including your `init.scm`), never by name at the command mode prompt.

::: info
`:bprev`/`:bnext` switch buffers from the command mode prompt. To bind buffer switching to keys, use the editor commands `goto-prev-buffer`/`goto-next-buffer` in `init.scm`:

```scheme
(bind-key! 'normal "ctrl-h" "goto-prev-buffer")
(bind-key! 'normal "ctrl-l" "goto-next-buffer")
```
:::

Most typed commands have a short alias; the tables list both forms, and both work. Command names ignore case. A name ends at the first character that is not a letter, digit, `-` or `_`, so put a space before an argument: `:e 1.txt`, not `:e1.txt`. `:b2` is the one exception: `:b` followed only by digits switches to that buffer.

Press `Tab` at any point to complete names and, where it makes sense, arguments. When several candidates match, the first `Tab` completes the text they share and opens the candidate list; another `Tab` picks the first candidate and `Shift-Tab` the last. `:set global command-completion=first-candidate` makes the first `Tab` fill in the first candidate at once instead.

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
| `:wa!` | Save every modified buffer as `:w!` does, including those whose file changed on disk |
| `:wq`, `:write-quit` | Save, then close as `:q` does |
| `:wq!` | Save as `:w!` does, then close as `:wq` does |

Relative paths given to `:w` resolve against HUME's working directory (`:pwd`), not the shell's.

## Files

| Command | Effect |
|---------|--------|
| `:e <path>`, `:edit <path>` | Open a file. `<path>` may end in `:line` or `:line:col` to jump there, e.g. `:e file.txt:42:5` |
| `:e` | Reload the current file. Refuses if there are unsaved changes |
| `:e!` | Reload, discarding unsaved changes |
| `:checktime` | Check every open buffer against its file on disk now, instead of waiting for the next automatic check (switching back to HUME, switching to the buffer) |

In arguments, `%` expands to the current file's path and `#` to the alternate file's, but only as a whole argument: `:w %.bak` won't expand. `:b` is the exception: it resolves `#` itself, so `:b #` works for buffers with no file on disk.

See [Files & Buffers](files-and-buffers.md#external-changes) for what happens when a file changes on disk.

## Buffers

| Command | Effect |
|---------|--------|
| `:ls`, `:list-buffers` | List open buffers in the bottom drawer |
| `:b <name>`, `:buffer <name>` | Switch buffer. Accepts a name, a unique filename prefix, a full path, a number from `:ls`, or `#` for the previous buffer (`:b#` and `:b #` both work) |
| `:bn`, `:bnext` | Next buffer |
| `:bp`, `:bprev` | Previous buffer |
| `:bd`, `:buffer-delete` | Close the buffer. Refuses if there are unsaved changes; closing the last one leaves a scratch buffer |
| `:bd!` | Close the buffer, discarding unsaved changes |

`:b#` is the quickest way back to the previous buffer. To bind the same jump to a key, use the editor command `goto-alternate-buffer`; `core:vim-keybind` binds it to `Ctrl-6` for you.

## Panes

| Command | Effect |
|---------|--------|
| `:sp`, `:split` | Split the focused pane, stacking the new pane below |
| `:vsp`, `:vsplit` | Split the focused pane side by side |

Splitting a pane that is already too small is refused with a message. Focusing, closing and rotating panes use the `Ctrl-p` prefix; see the [Default Keys](default-keys.md).

## Tabs

A tab is a saved window layout (its own set of panes and splits), not a per-buffer strip.

| Command | Effect |
|---------|--------|
| `:tabnew [path]`, `:tabe [path]` | Open a new tab. With no `path`, the new tab's pane views the current buffer; with `path`, it opens that file instead |
| `:tabclose`, `:tabc` | Close the current tab and every pane it owns. Refused with a message if it's the only tab, or if a `\|` run from one of its panes is still running; `:tabclose!` stops it and closes the tab |
| `:tabnext`, `:tabn` | Switch to the next tab in display order |
| `:tabprev`, `:tabp` | Switch to the previous tab in display order |

`Ctrl-p t`/`Ctrl-p T` also cycle tabs; see the [Default Keys](default-keys.md). By default the tab bar appears only once more than one tab is open; `:set global tabline=always`/`never`/`dynamic` changes this (see [Global options](settings.md#global-options)).

## Editing

| Command | Effect |
|---------|--------|
| `:sort` | Sort adjacent rows by their selected text |
| `:sort -r` | Sort in reverse |
| `:sort -i` | Sort case-insensitively |
| `:earlier [n\|5m]` | Step back `n` revisions (`:earlier`, `:earlier 3`), or back to how the buffer looked an age ago (`:earlier 5m`) |
| `:later [n\|5m]` | Step forward again after `:earlier`, same argument forms |

`:sort` groups your selections into runs of adjacent rows and sorts each run on its own. Each row's key is the text you selected on it, not necessarily the whole line:

- Select whole lines to sort a file (`%` selects the whole buffer).
- Select a single column across a block of lines to sort by that column.

A run sorts numerically only if every row's key looks like a number: one row with non-numeric text drops the whole run back to plain text order. Two selections on the same row combine into one key rather than counting as separate rows, so `:sort` can't sort several items within one line.

A single selection that spans several rows keeps its place in the buffer: the rows underneath it reorder, but the selection still covers the same stretch of text afterward.

`:earlier` and `:later` step through the undo history: back, then forward again along the path redo follows. A bare number counts revisions; a number with an `s`/`m`/`h`/`d` suffix names an age (`:earlier 90s`, `:later 2h`). Going past either end stops at the oldest or newest revision with a message.

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

See [Settings](settings.md) for every option.

## Display

| Command | Effect |
|---------|--------|
| `:theme <name>` | Load a theme; with no argument, show the current one |
| `:theme-debug` | Show the resolved styles for the main UI scopes |
| `:wrap`, `:toggle-soft-wrap` | Toggle line wrapping in this pane. See [wrap styles](settings.md#text-wrap) |
| `:mes`, `:messages` | Show the message log in a read-only buffer |

## Navigation

| Command | Effect |
|---------|--------|
| `:cd <path>`, `:change-directory <path>` | Change the working directory |
| `:pwd`, `:print-working-directory` | Print the working directory |
| `:goto <n>` | Jump to line `<n>`. `:42` is shorthand. Records a jump, so `Ctrl-o` comes back |

## Config and plugins

| Command | Effect |
|---------|--------|
| `:plugins`, `:plugin-status` | Show each plugin entry's state (declared, loaded or failed), what is still waiting to trigger it, and any plugin that is not installed (absent) |
| `:reload-config` | Re-run `init.scm` from scratch |
| `:ver`, `:version` | Show the editor version |
| `:tutor` | Open the interactive tutorial |

Plugins add commands of their own once you load them:

- `core:plum`: the `:plum-*` commands, which install plugins and grammars.
- `core:lsp`: `:lsp-status`, `:lsp-stop`, `:lsp-restart`, `:diagnostics`, and `:format-source`.
- `core:lsp-install`: `:lsp-install`, `:lsp-uninstall`, and `:lsp-catalog`.

None of these plugins is loaded until you ask for it in `init.scm`. See [Core Plugins](core-plugins.md).

## Finding a command

There is no listing command. Press `:`, then `Tab`: completion shows every registered typed name and alias, including those of plugins that haven't started yet. It never shows editor commands, since you can't run one by name at the `:` prompt; see [Editor Commands](editor-commands.md) for the full list.
