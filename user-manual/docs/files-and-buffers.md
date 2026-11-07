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

`:ls` opens a list across the bottom of the screen, one row per buffer: its number, `%` for the buffer you are in, `#` for the alternate, `+` for unsaved changes, its name and its path. It works like the [language server lists](lsp.md#the-list-drawer):

- `Down`/`Up` move the highlight and `Ctrl-d`/`Ctrl-u` page it.
- `Enter` switches the focused pane to the highlighted buffer and leaves the list open.
- `Esc` closes it.

A theme can set rows apart. The row of a buffer with unsaved changes has the `diagnostic.warning.message-text` scope, and the whole row has `diagnostic.warning.message` when it is the selected row. Otherwise the row of the buffer you are in has `markup.bold`.

To fuzzy-search files and buffers by typing a few characters instead, see [Fuzzy Finder](pickers.md).

### Alternate buffer

`:b #` switches back to the buffer you were in before this one. `:e #` does the same when that buffer has a file name, including a new file you haven't saved yet. For a scratch buffer or another buffer without a file name, use `:b #`.

## Splits and panes

A **pane** is a viewport onto a buffer. The buffer is the open file; a pane is where you view it. Two panes can show the same buffer at once, each with its own scroll position and, if pinned, its own wrap style.

`:sp` and `:vsp` split the focused pane (see [Panes](command-mode.md#panes)); without a path, the new pane views the same buffer as the focused one. The keys for splitting, focusing, closing and rotating panes are listed under [Pane prefix](default-keys.md#pane-prefix-ctrl-p) in Default Keys.

HUME refuses to split, with a message, when a pane would end up too small. Until you resize one of them, panes sharing a split axis stay the same size: three `:vsplit`s in a row give three equal columns, not one half and two quarters, and closing a pane shares its space equally among the rest.

`Ctrl-p >` and `Ctrl-p <` widen and narrow the focused pane, and `Ctrl-p +` and `Ctrl-p -` make it taller and shorter: by one column or row, or by a count typed first (`5 Ctrl-p >`). The space comes from the next pane in the innermost row or column the focused pane belongs to, or from the one before when it is the last there. Panes further out keep their size. No pane shrinks below its minimum size. With `mouse-select` and `pane-dividers` on, you can also drag the divider between two panes.

Once a row or column has been resized, a split there halves the focused pane and leaves the other panes as they are, and closing a pane gives its space to the pane beside it. `Ctrl-p =` makes every row and column of the tab even again.

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

## Scratch buffers

Launched with no arguments, HUME opens a scratch buffer named `*scratch*`. It has no file, so a bare `:w` fails with "no file name". Give it one with `:w <path>`.

`:new` (or `:n`) opens another empty scratch buffer in the current pane. Each one gets the next number, `*scratch-2*`, `*scratch-3*` and so on, and a number is not used again in the session, even after its buffer is closed or saved to a file.

## Read-only buffers

Some commands open read-only buffers for inspecting the editor's state:

| Command | Buffer | Contents |
|---------|--------|----------|
| `:messages` | `[messages]` | Message log |
| `:plugin-status` | `[plugin-status]` | Plugin states |
| `:lsp-status` (`core:lsp`) | `[lsp-status]` | Language server status |

The statusline shows `[RO]` for them, and editing commands are refused with a warning. Otherwise they are regular buffers: you can scroll, search, and close them with `:q` or `:bd`.

In `[messages]`, each entry's `[warning]`/`[error]`/`[trace]` tag and message text are colored by severity, and themes can restyle these independently of everything else.

`[messages]` stays up to date while it is open: new messages appear at the end, and changing `messages-min-severity` shows or hides entries. Your cursor stays on its entry. If that entry is hidden or dropped from the log, the cursor moves to the next entry, or to the one before when it was the last. Messages count as read once the end of `[messages]` is on screen, and the statusline stops counting them.

Files you open are always editable, whatever their permissions on disk. A write you aren't allowed to make fails at `:w` rather than being blocked up front.

## Persistence and safety

- **Undo history lasts until HUME exits, unless you save it.** With `undo-persist` on, saving a file saves its undo history too; see [Saved undo history](#saved-undo-history).
- **Open files and layout last until HUME exits, unless you save the session.** With `session-persist` on, quitting saves them for the session root, by default the directory HUME started in; see [Saved sessions](#saved-sessions).
- **No swap or backup files.** HUME writes no swap or backup files while you edit. A save writes to a temporary file and renames it into place, so the file on disk holds either the old content or the new, never a half-written mix.
- **Unsaved changes survive a crash.** If HUME itself panics, it writes them to disk and offers them back the next time you open the file; see [Crash recovery](#crash-recovery).
- **A background crash doesn't stop HUME.** If a background task crashes (syntax highlighting, a language server connection), HUME keeps running. The crash appears as an error in `:messages` and is printed on the terminal after you quit. The feature that task ran can stop working until you restart.
- **UTF-8 only.** Invalid bytes are rejected with an error (no lossy fallback). A byte-order mark isn't stripped; it appears as a character at the top of the buffer.
- **CRLF detected and preserved.** A CRLF file is normalized to LF in the buffer and re-expanded to CRLF on save; the statusline shows `CRLF` or `LF`. To convert a file, run `:set buffer line-ending=lf` or `:set buffer line-ending=crlf` and save. Text arriving from anywhere else (a paste, a register, an edit from a language server) is normalized the same way, so a buffer's lines always end in LF. A file with bare CR line endings (pre-OS X Mac) reads correctly, but HUME doesn't preserve that convention: it loads and saves as LF.
- **No on-disk log file.** `:messages` is the whole logging surface: an in-memory ring capped at 1000 entries, discarded on exit. When it is full, the oldest trace entry goes first, then the oldest warning, then the oldest error. `:messages` shows warnings and errors; set `messages-min-severity` to `trace` to also show the detail useful for debugging, or to `error` to show errors only. The setting changes only what is shown, so lowering it also shows messages reported before. To keep warnings or errors, copy them out of `:messages` before quitting.

### Saved undo history

With `undo-persist` on, every save of a file, and every reload of it from disk, also saves its undo history: every revision, including branches you undid away from. The next time you open the file, `u`, `U`, `:earlier` and `:later` walk that history as if you had never closed it, and `m u` brings back the selections you edited with.

- Turn it on everywhere with `(set-option! "undo-persist" #t)` in `init.scm`, or for one buffer with `:set buffer undo-persist=true`. To keep some files out, such as files holding secrets, set it to `false` for them from an `on-language-set` hook.
- Opening a file brings its saved history back whatever `undo-persist` says. Turning the option off stops saving new history but deletes nothing.
- The history comes back only when the file still holds the text it was saved with: a file changed elsewhere since the save opens with fresh history. So does a file whose history file is damaged, can't be read, or isn't a HUME history file, with a warning.
- When two HUME instances save the same file, the last save's history wins.

Saved history lives in HUME's state directory: `~/.local/state/hume/undo` (or `$XDG_STATE_HOME/hume/undo`), `%LOCALAPPDATA%\hume\undo` on Windows. Set `undo-directory` to keep it elsewhere. Inside it, each file's history sits at the file's own full path with `.undo` added, so the history of `/home/me/notes.txt` is `undo/home/me/notes.txt.undo` (on Windows the drive letter comes first, as in `undo/C/Users/me/notes.txt.undo`). A file opened through a symlink shares its history with the file it points to. Saved history keeps text you deleted, so on Linux and macOS its files and the folders HUME creates for them are readable only by you.

Nothing removes old history files. To forget a file's history, delete its `.undo` file; to forget a whole project's, delete its folder.

### Saved sessions

With `session-persist` on, quitting HUME saves the session of its session root, by default the directory HUME started in, and starting `hume` with no files and the same root brings it back:

- the files you had open, and the one you saw before the current one, so `:b#` still goes back to it;
- your tabs and splits, their sizes, and which pane had focus;
- in each pane, the selections, scroll position and wrap setting of every file it showed, and its jump list;
- the registers `q` and `0`–`9`, recorded macros included;
- what you typed at the `:`, `/`, `?` and `|` prompts, and the last search, so `n` works at once.

Scratch buffers, read-only views such as `[messages]`, unsaved changes, the kill ring `k` and the clipboard `c` are not kept: a session records which files were open, so save your changes to keep them.

- Turn it on with `(set-option! "session-persist" #t)` in `init.scm`. Turning it off stops both saving and restoring, and deletes nothing.
- Starting HUME with files, as in `hume notes.txt`, opens just those files and leaves the saved session alone: quitting that run doesn't save over it. Run `:save-session` to make it a session after all: it saves the session at once, replacing the saved one, and quitting saves it again. In any session, `:save-session` saves it right away.
- `:cd` doesn't move the session: it stays with the session root, and restoring one leaves you in the directory you started in.
- To share one session across a project, set `session-root` from `init.scm`. With `core:stdlib` loaded, this keys sessions by git repository, and by the starting directory outside one:

  ```scheme
  (set-option! "session-root" (or (call! "stdlib/git-toplevel") (cwd)))
  ```

  Only the value `init.scm` sets decides which session comes back. `:reload-config` runs `init.scm` again, so a root worked out from the current directory, as this one is, follows a `:cd` made before the reload. A session root that isn't a directory is reported at startup, and nothing is saved to it.
- HUME saves it however it ends, including when you close the terminal window, but not when HUME itself crashes.
- A file changed since the session was saved comes back in its place in the layout, at its top. A file that has been deleted since is left out. A damaged session file is ignored, with a warning.
- A pane that would be too small in the current terminal is left out, with a warning, and its space stays with the pane it was split from.
- When two HUME instances with the same session root quit, the last one's session wins, wherever under the root each was started.

Saved sessions live in HUME's state directory: `~/.local/state/hume/sessions` (or `$XDG_STATE_HOME/hume/sessions`), `%LOCALAPPDATA%\hume\sessions` on Windows. Set `session-directory` to keep them elsewhere. Each session sits at its root's own full path with `.session` added, so the session of `/home/me/project` is `sessions/home/me/project.session`. A root reached through a symlink shares its session with the directory it points to. On Linux and macOS its files and the folders HUME creates for them are readable only by you.

Nothing removes old sessions. To forget one, delete its `.session` file.

### Crash recovery

If HUME panics, receives a signal such as `SIGTERM` or `SIGHUP`, or loses its terminal (you closed the window), it writes every buffer with unsaved changes to a dump file. It prints each path on stderr, if there's still a terminal to print to.

Where the dump goes:

- A buffer for `foo.txt` is dumped next to it as `foo.txt.dump`, with its line endings intact.
- An unsaved scratch buffer, or a file whose folder can't be written to, has nowhere to put a dump beside the file. These go to the `dumps` folder of HUME's data directory (`~/.local/share/hume/dumps`, or `$XDG_DATA_HOME/hume/dumps`; `%LOCALAPPDATA%\hume\dumps` on Windows) as `scratch-<pid>-<n>.dump` (`<file>-<pid>-<n>.dump` for a file). HUME doesn't offer those dumps back; open the file and copy what you need out of it.

On Linux and macOS a dump is readable only by you; on Windows it gets the folder's permissions, like any new file there.

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
