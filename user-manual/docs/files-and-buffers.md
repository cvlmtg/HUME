# Files & Buffers

Open several files, view one file in two places at once, and keep track of them all. This page covers how files, buffers, panes and tabs behave, plus what HUME does and doesn't guarantee about your data. The commands themselves are listed on the [Command mode](command-mode.md) page.

## Opening files

Open a file with `:e <path>`; `Tab` completes file paths. `:e` with no argument reloads the current file, and `:e!` discards unsaved changes first. See [Files](command-mode.md#files).

### Opening at a position

Append a line, or a line and column, to jump straight there: `:e file.txt:42` or `:e file.txt:42:5`. Both numbers are 1-based, as in a `file:line:col` diagnostic from a compiler, linter or `grep`, so you can paste one straight after `:e`. A path that exists as typed, on disk or already open, always wins over splitting: a file named `notes:2024` still opens as itself.

### New files

`:e filename` on a path that doesn't exist yet opens an empty buffer bound to it instead of failing. `:w` creates the file the first time you save, and `:wa` saves it along with any other modified buffer. Until then, `:e` with no argument does nothing on that buffer. If something else creates a file at that path before you save, `:w` refuses to overwrite it; `:w!` saves anyway.

### External changes

If something else changes a file you have open (another program, a formatter, `git checkout`), HUME notices the next time you switch back to its window, switch to that buffer, or run `:checktime`, and asks whether to reload:

- **`r`** (reload) replaces the buffer's content but keeps it undoable (`u` brings back what you had).
- **`k`** (keep) leaves the buffer as-is, and HUME doesn't ask again about that change. The file stays flagged as changed until you reload it or overwrite it with `:w!`.
- **Any other key**, or `Esc`, dismisses the prompt without answering, and HUME asks again the next time you land on the buffer. A key other than `Esc` still does its usual job.

The prompt only appears when it can't interrupt something else. While you're typing at a prompt, or in Insert mode, HUME warns instead and asks the next time you land on the buffer.

Turn the prompt off with `:set global file-reload-prompt=false` (or `:set buffer file-reload-prompt=false` for the current buffer only). HUME still warns you, without asking. Either way, `:w` refuses to overwrite a file that changed since you last read or saved it; `:w!` saves anyway.

## The buffer list

A **buffer** is an open file (or scratch text). HUME can have several buffers open at once; `:ls`, `:b`, `:bn`, `:bp` and `:bd` manage them (see [Buffers](command-mode.md#buffers)). When two open files share a name, `Tab` completion for `:b` shows their parent directories to tell them apart.

`:ls` opens a list across the bottom of the screen, one row per buffer: its number, `%` for the buffer you are in, `#` for the alternate, `+` for unsaved changes, its name and its path. It works like the [language server lists](lsp.md):

- `Down`/`Up` move the highlight and `Ctrl-d`/`Ctrl-u` page it.
- `Enter` switches the focused pane to the highlighted buffer and leaves the list open.
- `Esc` closes it.

To fuzzy-search files and buffers by typing a few characters instead, see [Fuzzy Finder](pickers.md).

### Alternate buffer

`:b #` switches back to the buffer you were in before this one. `:e #` does the same when that buffer has a file name, including a new file you haven't saved yet. For a scratch buffer or another buffer without a file name, use `:b #`.

## Splits and panes

A **pane** is a viewport onto a buffer. The buffer is the open file; a pane is where you view it. Two panes can show the same buffer at once, each with its own scroll position and, if pinned, its own wrap style.

`:sp` and `:vsp` split the focused pane (see [Panes](command-mode.md#panes)); without a path, the new pane views the same buffer as the focused one. The keys for splitting, focusing, closing and rotating panes are listed under [Pane prefix](default-keys.md#pane-prefix-ctrl-p) in Default Keys.

HUME refuses to split, with a message, when the pane is already too small to divide. Panes sharing a split axis are kept the same size: three `:vsplit`s in a row give three equal columns, not one half and two quarters. Closing a pane shares its space equally among the rest.

A divider is drawn between panes (the `pane-dividers` option, on by default), and panes without focus are dimmed. Wrap follows the buffer/global setting unless you pin it for a pane: `:wrap` toggles it for the focused pane, and `:set pane wrap-mode=<value>` pins its style directly. Two panes on the same buffer then wrap independently. See [Text wrap](settings.md#text-wrap).

## Tabs

A **tab** is a saved window layout (its own splits and focused pane), not a per-buffer strip. Switching away from a tab and back restores its splits and focus where you left them. The `:tab*` commands are listed under [Tabs](command-mode.md#tabs), and the keys for switching tabs under [Pane prefix](default-keys.md#pane-prefix-ctrl-p).

`:tabnext`/`:tabprev` also exist as editor commands, `goto-next-tab` and `goto-prev-tab`, for binding to a key, and so does `:tabnew` without a path, as `tab-new`.

Click a tab in the tab bar to switch to it. The bar scrolls when tabs overflow the screen width. By default it appears only once more than one tab is open; the `tabline` option changes this (see [Global options](settings.md#global-options)).

Closing a tab, with `:tabclose` or with `:q` on its last pane, leaves its buffers open in the buffer list. `:tabclose` refuses to close the only tab.

## Saving

`:w` saves; see [Quitting and saving](command-mode.md#quitting-and-saving) for every form. A `[+]` in the statusline means the buffer has unsaved changes. CRLF line endings are preserved on save (see [Persistence and safety](#persistence-and-safety)).

HUME has one working directory, which `:pwd` shows and `:cd` changes. Relative paths in commands resolve against it, which isn't necessarily the shell's.

## Quitting

`:q` closes the smallest thing that is open:

- With several panes in the tab, it closes the focused pane. The buffer stays open.
- On a tab's last pane, with other tabs open, it closes the tab. Its buffers stay open.
- On the last pane of the last tab, it closes the buffer and shows another one. When no other file, and no other buffer with unsaved changes, is open, it quits HUME instead.

Unsaved changes block closing a buffer or quitting, and so does a running shell command (started with `|`) that closing would stop; `:q!` discards the changes and stops the command. `:qa` quits everything at once. See [Quitting and saving](command-mode.md#quitting-and-saving).

## The scratch buffer

Launched with no arguments, HUME opens a scratch buffer named `*scratch*`. It has no file, so a bare `:w` fails with "no file name". Give it one with `:w <path>`.

## Read-only buffers

Some commands open read-only buffers for inspecting the editor's state:

| Command | Buffer | Contents |
|---------|--------|----------|
| `:messages` | `[messages]` | Message log |
| `:plugin-status` | `[plugin-status]` | Plugin states |
| `:lsp-status` (`core:lsp`) | `[lsp-status]` | Language server status |

The statusline shows `[RO]` for them, and editing commands are refused with a warning. Otherwise they are regular buffers: you can scroll, search, and close them with `:q` or `:bd`.

In `[messages]`, each entry's `[warning]`/`[error]`/`[trace]` tag and message text are colored by severity, and themes can restyle these independently of everything else.

Files you open are always editable, whatever their permissions on disk. A write you aren't allowed to make fails at `:w` rather than being blocked up front.

## Persistence and safety

- **Undo history is in-memory only.** It's lost when HUME exits; there is no undo across restarts (not yet).
- **No swap or backup files.** HUME writes no swap or backup files while you edit. A save writes to a temporary file and renames it into place, so the file on disk holds either the old content or the new, never a half-written mix.
- **Unsaved changes survive a crash.** If HUME itself panics, it writes them to disk and offers them back the next time you open the file; see [Crash recovery](#crash-recovery).
- **A background crash doesn't stop HUME.** If a background task crashes (syntax highlighting, a language server connection), HUME keeps running. The crash appears as an error in `:messages` and is printed on the terminal after you quit. The feature that task ran can stop working until you restart.
- **UTF-8 only.** Invalid bytes are rejected with an error (no lossy fallback). A byte-order mark isn't stripped; it appears as a character at the top of the buffer.
- **CRLF detected and preserved.** A CRLF file is normalized to LF in the buffer and re-expanded to CRLF on save; the statusline shows `CRLF` or `LF`. To convert a file, run `:set buffer line-ending=lf` or `:set buffer line-ending=crlf` and save. Text arriving from anywhere else (a paste, a register, an edit from a language server) is normalized the same way, so a buffer's lines always end in LF. A file with bare CR line endings (pre-OS X Mac) reads correctly, but HUME doesn't preserve that convention: it loads and saves as LF.
- **No on-disk log file.** `:messages` is the whole logging surface: an in-memory ring capped at 1000 entries, discarded on exit. To keep warnings or errors, copy them out of `:messages` before quitting.

### Crash recovery

If HUME panics, receives a signal such as `SIGTERM` or `SIGHUP`, or loses its terminal (you closed the window), it writes every buffer with unsaved changes to a dump file. It prints each path on stderr, if there's still a terminal to print to.

A buffer for `foo.txt` is dumped next to it as `foo.txt.dump`, with its line endings intact. Two cases have nowhere to put a dump beside the file: an unsaved scratch buffer, and a file whose folder can't be written to. These go to the `dumps` folder of HUME's data directory (`~/.local/share/hume/dumps`, or `$XDG_DATA_HOME/hume/dumps`; `%LOCALAPPDATA%\hume\dumps` on Windows) as `scratch-<pid>-<n>.dump` (`<file>-<pid>-<n>.dump` for a file). HUME doesn't offer those dumps back; open the file and copy what you need out of it. On Linux and macOS a dump is readable only by you; on Windows it gets the folder's permissions, like any new file there.

No dump is written:

- after `:q!` or `:qa!`, which discard on purpose;
- when HUME is killed outright (`SIGKILL`, a crash of the whole machine, a power cut).

A dump written moments before a machine crash or power cut may not have reached the disk. A HUME that is stuck and doesn't react to a signal within about three seconds is forced to exit without one.

The next time you open `foo.txt` and `foo.txt.dump` exists, HUME asks what to do with it:

| Key | Result |
|---|---|
| `r` | Restore: the dump replaces the buffer's content, the buffer is marked as modified, and the dump is deleted. `u` brings back the file as it is on disk. |
| `d` | Discard: the dump is deleted and the buffer stays as the file is on disk. |
| `k` or `Esc` | Keep: the dump stays on disk and HUME doesn't ask again this session. It asks again the next time you open the file. |

Any other key dismisses the prompt and still does its usual job; HUME asks again the next time you switch to the buffer. The prompt also appears for a file that doesn't exist yet but has a dump, and restoring fills the new buffer.

A dump that is a link, or that belongs to another user, is ignored with a warning and no prompt.

A restored buffer is not saved: write it with `:w` once you've looked it over.
