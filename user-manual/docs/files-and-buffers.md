# Files & Buffers

Open several files, view one file in two places at once, and keep them all straight. This page covers opening, saving, splitting, and quitting — plus what HUME does and doesn't guarantee about your data.

## Opening files

```
:e filename
:e path/to/file.txt
```

Tab completion is available for file paths.

`:e` with no argument reloads the current file from disk. If the file has been modified HUME will not reload it, unless you use `:e!`.

### External changes

If something else changes a file you have open — another program, a formatter, `git checkout` — HUME notices the next time you switch back to its window, switch to that buffer, or run `:checktime`, and asks whether to reload. Answering yes replaces the buffer's content but keeps it undoable (`u` brings back what you had). Answering no leaves the buffer as-is; the file stays flagged as changed until you reload it or explicitly overwrite it.

The prompt only appears when it can't interrupt something else you're doing — while you're typing a command or search, or in Insert mode, HUME warns instead and asks the next time you land on the buffer.

Turn the prompt off with `:set global autoread=false` (or `:set buffer autoread=false` for just the current buffer) — HUME still warns you, it just won't ask. Either way, `:w` refuses to overwrite a file that's changed since you last read or saved it; add `!` (`:w!`) to save anyway.

## The buffer list

A **buffer** is an open file (or scratch text). HUME can have multiple buffers open at once.

| Command | Effect |
|---------|--------|
| `:ls` | List all open buffers |
| `:b name` | Switch to buffer by name or number |
| `:bnext` | Switch to next buffer |
| `:bprev` | Switch to previous buffer |
| `:bd` | Close (delete) current buffer (blocked if unsaved) |
| `:bd!` | Force close, discarding unsaved changes |

`:b` accepts a name prefix, a full path, a 1-based index as shown by `:ls`, or `#` to switch to the alternate buffer. When two open files share a name, Tab completion shows their parent directories to tell them apart.

For fuzzy-searching files and buffers by typing a few characters instead, see [Fuzzy Finder](pickers.md).

Closing the last remaining buffer leaves an empty scratch buffer rather than exiting.

### Alternate buffer

`:b #` switches back to the buffer you were in before this one. `:e #` does the same, but only works when that buffer has a file on disk — for scratch and other file-less buffers, use `:b #`.

## Splits and panes

A **pane** is a viewport onto a buffer. A buffer is the open file itself; a pane is where you view it — two panes can show the same buffer at once, each scrolled and wrapped independently.

| Command | Aliases | Effect |
|---------|---------|--------|
| `:split <path>` | `:sp` | Split the focused pane, stacking the new pane below it |
| `:vsplit <path>` | `:vsp` | Split the focused pane side by side |

`<path>` is optional. Without it, the new pane views the same buffer as the focused one. With it, the new pane opens that file instead.

| Key | Effect |
|-----|--------|
| `Ctrl+p s` | Split the focused pane, stacking the new pane below it |
| `Ctrl+p v` | Split the focused pane side by side |
| `Ctrl+p p` | Focus next pane |
| `Ctrl+p h` / `j` / `k` / `l` | Focus the pane to the left / below / above / to the right |
| `Ctrl+p c` | Close the focused pane (does nothing if it's the only pane) |

Splitting is refused with a message when the pane is already too small to divide.

`:q` is pane-aware: with multiple panes open it closes the focused pane and leaves the buffer in the buffer list. With a single pane it closes the current buffer and moves you to another one, quitting HUME only when there's nothing left to go back to. Unsaved changes block it — use `:q!` to discard them.

A divider is drawn between panes (controlled by the `pane-dividers` option, on by default), and the pane without focus is dimmed. Soft wrap is per-pane, so two panes on the same buffer can wrap independently: `:wrap` toggles it on/off for the focused pane, and `:set pane wrap-mode=<value>` changes its style directly — see [Text wrap](configuration.md#text-wrap).

## Saving

| Command | Effect |
|---------|--------|
| `:w` | Save current buffer |
| `:w filename` | Save as (write to a new path) |
| `:w!` | Force save (tries `chmod` + retry on permission errors; also overwrites a file changed on disk since you last read or saved it — see [External changes](#external-changes)) |
| `:wa` | Save every modified buffer |

A `[+]` indicator in the status bar means the buffer has unsaved changes. Files using CRLF line endings are detected and preserved on save. Relative paths are resolved against HUME's working directory (`:pwd`), which isn't necessarily the shell's.

## Working directory

| Command | Effect |
|---------|--------|
| `:cd <path>` | Change the working directory |
| `:pwd` | Print the current working directory |

## Quitting

| Command | Effect |
|---------|--------|
| `:q` | Close the focused pane if others are open; otherwise close the buffer, quitting on the last one |
| `:q!` | Same, discarding unsaved changes |
| `:wq` | Save, then close the focused pane if others are open; otherwise close the buffer, quitting on the last one |
| `:qa` | Quit everything. Refuses if any buffer has unsaved changes, and takes you to the first one |
| `:qa!` | Quit everything, discarding unsaved changes |

## The scratch buffer

If you launch HUME with no arguments, it opens a scratch buffer named `*scratch*`. This buffer has no associated file — `:w` will ask for a filename.

## Read-only buffers

The editor's own informational buffers — `:messages`, `:ls`, `:plugin-status` — are read-only. The status bar shows `[RO]`, and editing commands are refused with a warning. Files you open are always editable, whatever their permissions on disk; a write you aren't allowed to make fails at `:w` rather than being blocked up front.

## Synthetic buffers

Some commands open special read-only buffers for inspecting the editor's state:

| Command | Buffer | Contents |
|---------|--------|----------|
| `:messages` | `[messages]` | Message log |
| `:ls` | `[buffers]` | Open buffer list |
| `:plugin-status` | `[plugin-status]` | Plugin states |

These are regular buffers in all other respects — you can scroll, search, and quit them with `:q` or `:bd`.

## Persistence and safety

A few things worth knowing before trusting HUME with real work:

- **Undo history is in-memory only.** It's **lost when HUME exits** — there is no undo across restarts.
- **No swap or backup files.** HUME does not write Vim-style `.swp` files. Saves write to a temporary file and rename it into place, so the file on disk holds either the old content or the new, never a half-written mix.
- **UTF-8 only.** Files must be valid UTF-8 — invalid bytes are rejected with an error (no lossy fallback). A byte-order mark isn't stripped; it will appear as a character at the top of the buffer.
- **CRLF detected and preserved.** Files containing `\r\n` are normalized to `\n` in the buffer and re-expanded to `\r\n` on save; the status bar shows `CRLF` or `LF`. Bare `\r` (old Mac) is left as-is.
- **No on-disk log file.** `:messages` is the entire logging surface — an in-memory ring capped at 1000 entries, discarded on exit. If you need to keep warnings/errors, copy them out of `:messages` before quitting.
