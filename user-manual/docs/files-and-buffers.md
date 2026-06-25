# Files & Buffers

## Opening files

```
:e filename
:e path/to/file.txt
```

Tab completion is available for file paths. `:e` with no argument reloads the current file from disk.

## The buffer list

A **buffer** is an open file (or scratch text). HUME can have multiple buffers open at once.

| Command | Effect |
|---------|--------|
| `:ls` | List all open buffers |
| `:b name` | Switch to buffer by name or number |
| `:bnext` | Switch to next buffer |
| `:bprev` | Switch to previous buffer |
| `:bd` | Close (delete) current buffer |

`:b` accepts a name prefix, a full path, or a 1-based index as shown by `:ls`. If two open files share the same name, their parent directory is shown to disambiguate.

### Alternate buffer

| Key | Effect |
|-----|--------|
| `Ctrl+6` | Switch to alternate (most-recently-focused) buffer |

This works like Vim's `Ctrl+^` — toggles between the current and previous buffer.

## Saving

| Command | Effect |
|---------|--------|
| `:w` | Save current buffer |
| `:w filename` | Save as (write to a new path) |
| `:w!` | Force save (tries `chmod` + retry on permission errors) |

A `[+]` indicator in the status bar means the buffer has unsaved changes. Files using CRLF line endings are detected and preserved on save. Writes use atomic file operations (write to temp, rename).

## Working directory

| Command | Effect |
|---------|--------|
| `:cd <path>` | Change the working directory |
| `:pwd` | Print the current working directory |

## Quitting

| Command | Effect |
|---------|--------|
| `:q` | Quit (blocked if there are unsaved changes) |
| `:q!` | Quit without saving |
| `:wq` | Save and quit |
| `:qa` | Quit, closing all buffers |

## The scratch buffer

If you launch HUME with no arguments, it opens a scratch buffer named `*scratch*`. This buffer has no associated file — `:w` will ask for a filename.

## Read-only mode

Buffers can be marked read-only. The status bar shows `[RO]` when active. Editing commands are blocked on read-only buffers — attempts show a warning.

## Synthetic buffers

Some commands open special read-only buffers for inspecting internal state:

| Command | Buffer | Contents |
|---------|--------|----------|
| `:messages` | `[messages]` | Message log |
| `:ls` | `[buffers]` | Open buffer list |
| `:plugin-status` | `[plugin-status]` | Plugin states |

These are regular buffers in all other respects — you can scroll, search, and quit them with `:q` or `:bd`.