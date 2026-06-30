# Files & Buffers

## Opening files

```
:e filename
:e path/to/file.txt
```

Tab completion is available for file paths.

`:e` with no argument reloads the current file from disk. If the file has been modified HUME will not reload it, unless you use `:e!`.

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

`:b` accepts a name prefix, a full path, a 1-based index as shown by `:ls`, or `#` to switch to the alternate buffer (same as `Ctrl+6`). If two open files share the same name, their parent directory is shown to disambiguate.

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

## Persistence and safety

A few things worth knowing before trusting HUME with real work:

- **Undo tree is in-memory only.** Branching history is preserved for the session but **lost when HUME exits**. There is no persistent undo across restarts.
- **No swap or backup files.** HUME does not write Vim-style `.swp` files. Saves use an atomic temp-file-plus-rename, so on POSIX the target either has the old content or the new content, never a partial write.
- **UTF-8 only.** Files are read with `std::fs::read_to_string` — invalid UTF-8 returns an error (no lossy fallback). No BOM detection or stripping.
- **CRLF detected and preserved.** Files containing `\r\n` are normalized to `\n` in the buffer and re-expanded to `\r\n` on save; the status bar shows `CRLF` or `LF`. Bare `\r` (old Mac) is left as-is.
- **No on-disk log file.** `:messages` is the entire logging surface — an in-memory ring capped at 1000 entries, discarded on exit. If you need to keep warnings/errors, copy them out of `:messages` before quitting.
