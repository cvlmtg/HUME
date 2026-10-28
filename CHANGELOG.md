# Changelog

## Unreleased

### Breaking changes
- `on-text-changed` handlers take `(pane changes)`: add the second parameter, `changes`, the hunks since the previous fire or `#f`.
- `buffer-generation` returns an opaque value instead of an integer: compare two with `equal?`. `#:expect-generation` takes only that value, and raises for one from another buffer.

### Plugins & scripting
- New `buffer-changes-since` lists the lines that changed since a `buffer-generation` value was read.
- New `buffer-search` finds a pattern in a buffer without moving the cursor or changing the current search, and `word-search-pattern` gives the pattern `*` would search for on a word.

## [0.17.0] - 2026-10-28

### Breaking changes
- `(hume/yield!)` is gone: the script budget stops a script on its own, so delete any call to it.
- `stdlib/run!` is gone: call `run-capture!`.
- `stdlib/selection-anchor`, `-head`, `-start-char`, `-end-char` and `-primary?` are gone: call the built-in `selection-anchor`, `selection-head`, `selection-start-char`, `selection-end-char` and `selection-primary?`.
- `grammar-url`, `grammar-rev`, `grammar-symbol` and `grammar-subpath` are gone, and `hume-version` and `bundled-language-grammar` return a hash instead of a list: read a field with `hash-ref`, such as `(hash-ref g 'url)`.
- `log!` spells its warning severity `'warning`: replace `'warn`.
- `core:buffer-words`'s `"lines"` option is `"chars"`, counted in characters: replace it in your `#:config`, or drop it.
- A plugin no longer overrides your own setup, even when it loads later: its `define-language!`, `register-grammar!`, `register-lsp-server!`, `unregister-lsp-server!`, `set-language-servers!`, `bind-key!`, `unbind-key!`, `set-option!` and `set-buffer-option!` leave alone what your `init.scm`, its hooks or `:set` set. A command you run, such as `:lsp-install`, still replaces it for the session.
- A second `define-language!` for one language in your `init.scm` is ignored with a warning: merge the two.
- `set-option!` and `set-buffer-option!` take a value only in the type `get-option` returns, and a named value is a string both ways: write `"soft"` for `'soft`, `4` for `"4"`, `#t` for `"true"` and `'("fish" "-c")` for `"fish -c"`. `wrap-mode` reads back as written, `"word"` and not `"word:0"`.
- `(get-option "statusline")` returns `(hash 'left 'center 'right)`, each a list of element names, instead of the `"a,b|c|d"` text, and `set-option!` takes that hash.
- A plugin's `#:config` and the `stdlib/config-*` readers take symbol keys: write `(hash 'signs #f)` for `(hash "signs" #f)`. `load-plugin!` raises for a string key or a value that isn't a hash.
- `plugin-config` and `plugin-dir` raise outside a plugin's code, such as at the top level of `init.scm`.
- `stdlib/config-integer` takes an integer minimum: replace a `#f` minimum with the lowest value you accept.
- `declare-plugin!` raises, instead of ignoring the entry, for an entry declared twice, a command name already taken or listed twice, or a declaration after the plugin's `load-plugin!`: merge, rename or move it. Your own `declare-plugin!` still replaces a plugin's `manifest.scm`.
- `call!` raises instead of returning `#f` for a command it can't reach (an unknown name, a typed command such as `write`, a key-only one such as `pipe`), for a built-in command while `init.scm` or a plugin loads, and for a negative count. `command-exists?` answers `#f` for a key-only command: probe an optional command with it first.
- A built-in `:` command refuses an argument or a `!` it doesn't use, `:b!` and `:e! <path>` included.
- A callback handed to HUME (a `register-hook!` or `register-lsp-notification-hook!` handler, a timer, a prompt, menu, drawer or picker callback, a background job's or language server's reply, a completion source) raises when it is passed if it can't take the arguments it will get, instead of failing when it runs.
- A `spawn-async!` callback takes one argument, the hash `run-capture!` returns: write `(lambda (result) ...)` and read `(hash-ref result 'exit)`.
- `on-trigger-char` passes a Scheme char, and the trigger setters take a list of chars: write `'(#\.)` for `'(".")`.
- `after!`, `spawn-async!` and `path-programs` return an opaque timer or job instead of an integer: compare two with `equal?`. `cancel-timer!` takes only a timer and `cancel-async!` only a job.
- `path-programs` lists in the background: it takes a `(lambda (names done?) ...)` callback and returns a job. Move code that used its result into the callback, and call it from a command, hook, timer or callback, not at the top level of `init.scm` or of a plugin loaded at startup.
- `run-capture!` and `spawn-async!` keep only the first and last 4 KiB of a stderr past 8 KiB, and `run-capture!` fails past 64 MiB of stdout, as `spawn-async!` does.
- `run-inline-output!` raises outside an `#:inline-output` command, the top level of `init.scm` included: move the call into one, or use `run-capture!`.
- Steel's own print functions (`display`, `displayln`, `newline`, …) write as Steel defines them, over the editor's screen too: report with `log!`, or write a line inside an `#:inline-output` command with the new `print!`.
- `close-buffer!` raises for a buffer with unsaved changes: pass `#:discard-unsaved #t` to drop them.
- `paste-text!` raises for a read-only buffer instead of doing nothing.
- `edit-text!` edits for no pane when `pane` no longer shows its buffer, as `apply-text-edits!` does.
- `switch-to-buffer!`, `show-read-only-buffer!` and `goto-location!` return the pane now showing their target: use it for the next call on that buffer.
- A path given to a HUME call resolves `..` the way the operating system does, and raises for a `..` after a directory that doesn't exist.
- `define-language!` raises, instead of skipping the entry, for an invalid glob, an extension with a leading or trailing dot or a `/`, or a shebang with a space or a `/`: give the interpreter's name alone, such as `"python"`.
- The decoration and trigger setters take the pane first: write `(set-signs! pane source signs)` for `(set-signs! source pane signs)`, and the same for `set-inlay-hints!`, `register-sign-source!`, `set-virtual-lines!`, `set-eol-text!`, `set-extra-highlights!`, `set-line-backgrounds!`, `set-statusline-text!` and the two `set-attachment-*-triggers!`. `set-hook-triggers!` and `set-completion-triggers!` take the language first.
- A virtual line names its side with `'side` instead of `'anchor`, and its `'segments` take `'start-char-col`/`'end-char-col`, char columns in its `'text`, instead of `'start-char`/`'end-char`.
- The decoration, trigger and completion-source builtins raise for an empty or blank source name.
- `show-menu!`, `show-drawer-list!` and `update-drawer-list!` take `(display . payload)` pairs, as `picker!` does, and `on-select` gets the row's payload, or its key with `#:render`, instead of its index.
- `update-drawer-list!` raises on an empty list instead of returning `#f`: close the drawer instead.
- `picker!` and `live-picker!` return `#f` when `.` answers them without opening, as `show-menu!` does.
- Keymap names are strings, like command and plugin names: write `#:keymap "my-keys"` for `#:keymap 'my-keys`.
- `#:supersede` takes a key made by the new `lsp-supersede-key` instead of a string, so a plugin can't cancel another's request by reusing its name: define one with `(define my-key (lsp-supersede-key))` at your plugin's top level.
- `diagnostic-counts` returns `(hash 'error 'warning 'info 'hint)` instead of `'errors`/`'warnings`, and counts only severities at or above `lsp-diagnostics-min-severity` or its new `#:severity`. The statusline's counts follow the same floor.
- `diff-lines`, `diff-buffer-lines` and `buffer-revision-diff` return `(hash 'hunks … 'deadline-hit …)`, as `diff-words` does: read `(hash-ref result 'hunks)`. A hunk's `'words` is `#f` when it was too large to compare word by word.
- `completion-emit!` for a request already answered without `#:more` is dropped and returns `#f`: answer with `#:more #t` until the last part.
- A `'minibuf` completion source raises for a nonzero `#:priority` and for `#:incomplete #t`, which never had an effect there.

### Syntax & language awareness
- HUME starts faster: each installed grammar loads the first time a file or an embedded block needs it, not at startup, and a grammar that can't load is reported in `:messages` then.

### Plugins & scripting
- `steel-command-budget-ms` and `steel-init-budget-ms` stop a script that runs past them, so a plugin stuck in a loop can't freeze HUME.
- `init.scm` and plugins compile faster, so HUME starts sooner.
- `lsp-stop!` and `lsp-restart!` take a server value, to stop or restart that one server.
- `completion-emit!` takes `#:more #t` to answer a request in parts, and `register-completion-source!` takes `#:cancel` to stop work for a request HUME no longer waits on.
- `completion-emit!` with `#f` for its id returns `#f` instead of raising.
- `plugin-config` and `plugin-dir` work in a plugin's commands, hooks and callbacks, not only in its body.
- `edit-text!` takes `#:expect-generation`.
- `set-cwd!` works from `init.scm` too.
- New `buffer-char-count`, `find-program`, `remove-dir!`, `replace-dir!`, `try-lock-file!` and `unlock-file!`.
- An error from a HUME function starts with the name you called it by, once, and names the argument at fault.
- `set-option!` and `get-option` on `language` or `line-ending` point to `set-buffer-option!` and `get-buffer-option` instead of reporting an unknown setting.
- `.` passes a `#:repeatable` command the same `count` its first run got.

### Fixes
- On Windows, closing the terminal window lets HUME shut down cleanly instead of ending it at once, also while a command's output is shown.
- `diff-lines` and `diff-words` work at the top level of `init.scm` and in a plugin body.
- `lsp-show-status!` opens the status view on the pane it was given.
- `lsp-language-servers` sees a `set-language-servers!` or a registration made earlier in the same call.
- `:b`, `:goto` and `:set` without an argument report what they take as an error in `:messages`.
- `apply-text-edits!` does nothing for an empty edit list instead of raising, and `apply-workspace-edit!` leaves a file with no edits alone instead of opening it.
- A language server's own workspace edit, such as one a code action asks for, shows how many buffers it changed, and an edit that changes nothing shows nothing.
- `:lsp-install` refuses a server archive with a symbolic link that points outside the server's directory, and a failed install removes what it had installed.
- A HUME that exited can't leave `:lsp-install` locked.
- The Linux archive runs on glibc 2.28 and later: Ubuntu 20.04, Debian 10, RHEL 8 and newer.
- Plugin code that runs later (debounced and timed functions, background process and language server replies, picker actions) no longer reads wrong values or crashes HUME after a while.
- A plugin error that passes through nested `with-handler`s no longer crashes HUME.
- Loading some plugin code no longer prints stray text over the screen.
- `cancel-timer!` and `cancel-async!` also drop a timer or callback that was already due but hadn't run yet, so a debounced function no longer runs twice, and `cancel-timer!` never cancels one of HUME's own timers.
- An error in a timer, a background reply or another plugin's callback no longer empties a completion menu that is still waiting on a source.
- `core:git-diff`'s inline diff keeps its word highlights when the machine is busy.
- `show-menu!` raises on an empty list instead of opening a menu that crashed HUME on Enter.
- Built-in languages are detected by file patterns that name a directory, such as GitHub workflow files and `.vscode/*.json`, and by extensions with several parts, such as `.js.map`.
- Scripts whose first line runs a versioned interpreter, such as `#!/usr/bin/env python3` or `#!/usr/bin/lua5.4`, get their language, and so do V scripts.
- Signs, virtual lines, end-of-line text and line backgrounds no longer disappear when you delete the first character of their line, or its whole indent.
- A plugin's gutter signs keep working after `:e!` reloads a changed file.
- A completion item whose range a language server sent backwards no longer moves the completion menu.
- `:e!`, and reloading a file that changed on disk, keep a language you set with `:set buffer language=` or `set-buffer-option!`.
- `Tab` on the `|` prompt lists programs as it finds them, so HUME keeps responding on a slow `PATH`, like WSL's Windows directories.
- `core:buffer-words` no longer stalls typing on a file with very long lines, nor resets the highlighted item of an open completion menu while it indexes.
- `Ctrl-c` during a command that shows a program's output, such as a grammar install, stops that program instead of quitting HUME.
- A file opened through a symlink is named after the link you opened, not its target, in the statusline, tab labels, `:ls` and `:b`.
- When the symlink you opened a file through starts pointing at another file, `:w` refuses to overwrite that file (`:w!` writes it), `:checktime` reports the change, and `:e!` reads the new file.
- `..` in a path you type to `:e`, `:w`, `:cd` or path completion leaves the directory it follows the way your shell does: after a symlink, it goes to the link target's parent.
- `goto-location!` takes a path as written, like every other path a plugin passes: a `$` or `~` in a file name no longer opens some other file.
- Every file named on the command line expands `~` and variables like `$HOME`, the first one included, and so do `--config` and `--output`.
- `r`, `J`, `&`, `>`, `<`, surround, case changes and a terminal paste on a read-only buffer say so instead of doing nothing, and every read-only refusal names the buffer.
- Macros record and replay in a read-only buffer such as `:messages`, and an edit a macro replays there is refused out loud.
- A failed `compile-grammar!` includes the compiler's error output, and the compiler never writes over the screen.

## [0.16.1] - 2026-10-22

### Fixes
- Installing into `~/.local` no longer mixes the bundled plugins and themes with your own: the macOS and Linux archives keep them in `share/hume/runtime/`. HUME refuses to start when `HUME_RUNTIME` points at the data directory.

## [0.16.0] - 2026-10-22

### Breaking changes
- Grammars are compiled by `hume-grammar-build`, which ships beside `hume`. Copy it along with `hume` when you install by hand, and name both packages with `cargo install`: `cargo install --git https://github.com/cvlmtg/HUME hume-editor hume-grammar-build`.
- Key-bindable commands of the core plugins take their plugin's prefix. Rename them in your bindings and `call!`s: `lsp-fmt` is `lsp/format-source`, the other `lsp-*` commands and `goto-next-diagnostic`/`goto-prev-diagnostic` take `lsp/`, `toggle-undotree` is `undotree/toggle`, `picker-*` are `pickers/*`, `plum-ensure-grammars` is `plum/ensure-grammars`, `classic-*` and `vim-*` are `classic-paste/*` and `vim-keybind/*`, and `helix-delete-surround`/`helix-replace-surround` are `helix-surround/delete`/`helix-surround/replace`. Typed `:` commands keep their names.
- Options start with the area they belong to. Rename them in your `init.scm` and `:set` calls: `tab-width` → `indent-width`, `tab-style` → `indent-style`, `smart-indent` → `indent-smart`, `auto-pairs` → `insert-auto-pairs`, `cursor-shape-insert` → `insert-cursor-shape`, `select-inserted-text` → `insert-select-on-exit`, `line-number-style` → `gutter-line-numbers`, `signcolumn` → `gutter-signs`, `object-jump-align` → `scroll-jump-align`, `jump-line-threshold` → `jump-list-min-lines`, `history-capacity` → `prompt-history-levels`, `prefer-line-comments` → `comment-prefer-line`, `auto-read` → `file-reload-prompt`, `before-save-timeout-ms` → `hook-before-save-timeout-ms`, `viewport-debounce-ms` → `hook-viewport-debounce-ms`, `statusline.mode-colors` → `statusline-mode-colors`, `lsp.diagnostics-severity-floor` → `lsp-diagnostics-min-severity`, and the other `lsp.` options take `lsp-` (`lsp-request-timeout-ms`).
- `offset->line`, `line->offset`, `lsp-position->offset` and `lsp-range->offsets` are `char-offset->line`, `line->char-offset`, `lsp-position->char-offset` and `lsp-range->char-range`, and `core:stdlib`'s `stdlib/cursor-char-index`, `stdlib/selection-start` and `stdlib/selection-end` are `stdlib/cursor-char-offset`, `stdlib/selection-start-char` and `stdlib/selection-end-char`.
- `lsp-locations->display-parts` is `lsp-locations->places`, which no longer merges locations; `lsp-locations->merged-places` does.
- Position hashes name their unit: `'start`/`'end` are `'start-char`/`'end-char` or `'start-line`/`'end-line` in `viewport-range`, `lsp-range->char-range`, `set-extra-highlights!`, `set-virtual-lines!` segments and `diagnostics-for-buffer`, whose `'line` is `'start-line` and whose `'end-line` is one past the diagnostic's last line. `set-inlay-hints!` takes `'char-offset` for `'offset`.
- `diff-words` hunks carry `'old-start-char`, `'old-end-char`, `'new-start-char` and `'new-end-char`. `diff-lines`, `diff-buffer-lines` and `buffer-revision-diff` hunks carry `'old-start-line`, `'old-end-line`, `'new-start-line` and `'new-end-line` (end exclusive) in place of `'old-start`, `'old-count`, `'new-start` and `'new-count`, and their `'words` spans change shape.
- A line range is one value, `(hash 'start-line s 'end-line e)`: `viewport-range` returns it, `buffer-lines` takes it as `#:range` in place of `#:start` and `#:end`, and `on-viewport-change` handlers receive `(pane range)`.
- `char-offset->line` and `buffer-cursor-line` put the end of the buffer on its last line and raise for an offset past it. `lsp-range->char-range` returns `#f`, and `apply-text-edits!` and `apply-workspace-edit!` raise, for a range past the end of the document instead of clamping it.
- `goto-location!` returns `#t` or `#f`, a pane target takes `'char-offset` in place of `'line` and `'char-col`, and a URI is no longer a target: pass the path. `diagnostics-for-buffer` entries have no `'char-col`: jump with `'start-char`.
- Calls that need language servers, diagnostics, completion, timers or background processes raise where those are unavailable, such as in a drawer's `#:render`, instead of returning an empty answer.
- `live-picker!` takes `#:source (picker-command-source argv-for)` in place of `#:command`.
- Typed command names can contain digits, so an argument needs a space before it: `:e 1.txt`, not `:e1.txt`. `:b2` still works.
- `bind-key!` raises for a key sequence in `'insert` mode, and `bind-key-extend!` outside `'normal`. Neither binding ever ran, so delete it.
- Command names ignore case, and `define-command!` and `#:commands` raise for an uppercase name: rename such commands to lowercase.
- `s` matches each selection's text on its own, without a line break it ends on, so `^`, `$` and `\b` see the selection's edges: add `(?m)` for `^` and `$` to match at each line inside it.
- `spawn-async!` passes `#f` as the exit code when the command could not start or was killed by a signal, as `run-capture!` does: compare it with `equal?`, not `=`.

### Editing
- `|` pipes each selection through a shell command in the background, `Ctrl-c` stops it, and `Ctrl-|` in Insert mode inserts a command's output at each cursor. The new `shell` option sets the shell, and `Tab` on the prompt completes program names and file paths.
- `Ctrl-r` in Insert mode and on the command line inserts a register or the current file's path.
- `m i x` and `m a x` select inside or around a tag pair, and `g x` and `g X` jump to the next or previous one, with or without a markup grammar.
- `Ctrl-a` and `Ctrl-s` add to or subtract from the number, date or time at each selection, and `G A` and `G S` count up or down across selections.

### Panes & interface
- `Shift-Tab` (kitty) and `Ctrl-p P` focus the previous pane.
- `core:git-diff`'s new `"branch-style"` config can show the statusline branch as `⎇ main`.
- The bottom drawer has a border, open at the bottom, which `popup-border` turns off.
- New bundled themes: Catppuccin, Tokyo Night, One Dark, Dracula, GitHub, Kanagawa, Rosé Pine, Solarized and Nord, with their variants.
- Typing, scrolling and redrawing are faster.
- The statusline's search match count hides once you type in Insert mode and comes back when you leave it.

### Plugins & scripting
- A `live-picker!` source gets a token per query, so a slow source can't overwrite newer rows.
- New `on-before-buffer-save` hook, which can hold a write until its handler is done.
- `core:git-diff` has key-bindable `git-diff/toggle-inline-diff` and `git-diff/toggle-git-signs`.
- New `paste-text!` inserts text as a paste would, and `edit-text!` replaces char ranges of a buffer in one edit.
- `bind-key!` takes `'command` to bind keys in the command mode prompt and in plugin prompts.
- Key bindings can apply to some buffers only: bind them into a named keymap with `#:keymap`, then turn it on in a buffer with `attach-keymap!`.
- `buffer-display-path` takes `#:relative #t` for the path relative to the working directory.
- New `register-name?` tells whether a string names a register.
- New `register-shell-dialect!` teaches `Tab` on the `|` prompt another shell's quoting.
- `completion-emit!` takes `#:span` and `#:prefix` for a `'minibuf` source.
- New `path-entries`, `path-programs`, `home-dir` and `string-common-prefix`.
- `load-plugin!` takes `#:ref` to pin a plugin to a tag, branch or commit, and `declared-plugin-ref` reads it back.

### Language servers
- `z s` and `z S` pick a symbol of the current file or of the whole project and jump to it.
- New `lsp/format-document` formats the buffer from a save hook. The manual's earlier format-on-save recipe never formatted the saved file: replace it with the new one.

### Fixes
- A `'minibuf` completion source gets the cursor counted in characters, not bytes.
- Tab completion of a file path treats a link to a directory as a directory, and ignores letter case on Windows.
- On Windows, closing a picker or stopping a language server also stops the programs it started.
- Git run by a language server fails at once on a private repository or expired credentials, instead of waiting for a password nobody can type.
- A language server's log keeps going after a line that is not valid UTF-8.
- A command or language server that cannot start because its working directory is missing says so, instead of reporting the program as missing.
- A failed `:plum-install-theme` or `:plum-install-grammar` over an installed theme or grammar keeps the installed one working.
- `:plum-install-grammar` installs the YAML, Vue, Astro, Lean, Org and haskell-persistent grammars, which are partly written in C++, and no longer needs the tree-sitter CLI.
- In Insert mode, `End`, `↑` and `↓` reach the end of a line, after its last character.
- The command line's completion menu opens right above the command line, over an open drawer or docked popup, instead of above them.
- Headless `--keys` runs print warnings and errors on stderr and exit with status 1 when an error is logged, lay the file out on an 80×25 screen, and honour a `FILE:LINE[:COL]` input.
- Centring, scrolling, `:split`, `viewport-range`, and popup and picker paging count a drawer, docked popup or tab bar that opened or closed since the last screen update.
- In a macro, `Ctrl-d` and `Ctrl-u` scroll a popup opened by the previous key, and the completion documentation box after a selection move, instead of closing the popup or paging the wrong text.
- `set-extra-highlights!`, `set-inlay-hints!` and `set-virtual-lines!` segments accept a char offset inside a character such as an emoji sequence.
- `json-ref-or` returns its default for a field the server sent as `null`, so a code action whose `disabled` is `null` is offered.
- `apply-text-edits!` and `apply-workspace-edit!` still apply after the pane switches buffers, and `apply-text-edits!` and `lsp-document-params` work without a pane.
- `#` on a closing tag finds its opening tag past a commented-out tag.
- `[` after pasting from the clipboard or a register skips the kill-ring entry holding the same text instead of pasting it again.
- A key bound with `bind-key-extend!` extends the selection, not only when the binding starts with `Ctrl`.
- `p` still pastes what you last deleted, changed or yanked after a language server, a background `|`, or a plugin's timer or hook changes some text.

## [0.15.0] - 2026-10-15

### Breaking changes
- `lsp.inlay-hints` is removed: `:toggle-inlay-hints` toggles them per buffer, and `core:lsp`'s `inlay-hints` config turns them on everywhere.
- `lsp.format-max-ranges` moves to `core:lsp`'s `#:config`, and `lsp.viewport-debounce-ms` is `viewport-debounce-ms`.
- `bind-wait-char!`, `request-wait-char!` and `pending-char` are removed: commands read keys with `on-next-char!`, so `bind-key!` binds any command.
- `repeat-last-action` runs only from a key: `call!` of it raises an error.
- `word-selects-whitespace` takes `off`, `before` (the default) or `after` instead of `true`/`false`, and `maw`/`maW` follow it. `after` makes `w`/`b`, `mm` and `maw` take the whitespace after the word.
- The bottom drawer moves one row with `Down`/`Up` instead of `Shift-Down`/`Shift-Up`; `j`/`k` still move the cursor in the buffer.
- `define-language!` no longer takes `#:roots` or `#:language-id`: set them with the new `configure-language!`.
- `:plum-install-grammar`, `:plum-list-grammars` and `plum-ensure-grammars` take and list language names. A grammar installed under another name, such as `proto`, is an orphan, which `:plum-list-grammars` lists: remove it with `:plum-cleanup-grammars`.

### Editing
- `G G` comments or uncomments the lines of each selection, in the language at the cursor. `prefer-line-comments` uses line comments instead of a block comment for several lines.
- `.` reuses your answers to a `#:repeatable` command's character read, picker, prompt or menu instead of asking again.
- With the kitty keyboard protocol, `Ctrl-?` searches backward and extends the selection.
- Enter, `o` and `O` indent new lines from the language's syntax tree, and a closing token such as `}` typed on a new line moves that line back. Run `:plum-install-grammar <language>` again for each grammar installed before this release.
- `:set buffer line-ending=lf|crlf` converts a buffer's line endings.

### Panes & interface
- `Ctrl-p r` and `Ctrl-p R` rotate the panes in the focused pane's row or column forward and back.
- Hover, signature help and the completion documentation box grow up to 120 columns wide, up from 60.

### Theming
- With no `theme` set, HUME loads `sand` from `themes/`, so edits to `sand.toml` apply.

### Plugins & scripting
- New `configure-language!` sets a language's project root markers, language server identifier and comments. A value from `init.scm` wins over a plugin's.
- New `bundled-language-grammar` and `bundled-grammar-languages` read the grammars HUME ships for each language.
- New `show-read-only-buffer!` shows text in a read-only buffer, reused by name.
- New `read-data-file`, `data-file` and `data-file-names` read a plugin's Scheme data files.
- New `safe-path-segment?` checks that a name is safe to use as one path component.
- New `stdlib/report-section` and `stdlib/show-report!` in `core:stdlib` show a list as titled sections in a read-only buffer.
- `call!` can pass a pattern to `search-forward`, `search-backward` and `sift-within`, and a character to `find-forward`, `find-backward`, `till-forward`, `till-backward`, `replace` and `surround-add`.
- A completion item's `documentation` and `detail` show in the completion documentation box.
- `register-grammar!` takes an `#:indents` query that indents new lines from the syntax tree.

### Language servers
- A box beside the completion menu shows the highlighted completion's documentation, asking the server for it when missing. `completion-docs` turns it off.

### Fixes
- The search and sift prompts report an invalid or unmatched pattern the same way scripts do. An invalid search pattern is not stored.
- After `q`, `Q` or `"`, a click or paste cancels the register wait. Esc cancels only the wait.
- `.` after an Insert session in which you pressed an arrow key leaves a cursor, as Esc did, instead of selecting the typed text.
- `.` after an Insert session that ended at a key bound to a search, sift or `:` prompt repeats the text typed before that key, without an error or a prompt.
- `.` inside a macro repeats the last change, including one made before the macro ran. After a macro, `.` repeats the last change the macro made.
- `n`, `N` and `m/` with no search pattern report it. `.` of a change whose selection step finds no pattern or no match leaves the text alone.
- `i`, `a`, `I`, `A` and the other Insert entries leave the selection where it was when they are refused: on a read-only buffer, or while another pane is in Insert mode on the same buffer.
- A paste with nothing to paste and `[`/`]` with nothing to cycle say why in the status line, and stop `.` or a macro there.
- `:e!` restores a file's line ending even when its text is unchanged.
- With soft wrap, `select-word-nearest-on-line` on a word that fills its display line no longer selects the space that starts the next display line.
- In Extend mode, `w` after a backward extend moves the selection's start forward by one word instead of doing nothing.
- In Extend mode, `w` onto the first word of an indented line takes the space after that word, as `w` does in Normal mode.
- `:plum-list-plugins`, `:plum-list-grammars` and `:plum-list-themes` show their whole lists in a read-only buffer.
- `:lsp-catalog` shows the server list in a read-only buffer, sorted by name, instead of printing it over the editor.
- `core:lsp-install` reports a server whose install receipt is damaged as an interrupted install, instead of failing to load.
- A position an `lsp-request!` holds through `#:tracked` is released when the code that sent the request raises an error afterwards, instead of staying tracked.
- Text objects and highlighting are faster in Markdown files with many code blocks.

## [0.14.1] - 2026-10-08

### Fixes
- A server that is slow to start no longer gets a burst of inlay-hint requests and `lsp inlay hints: timed out` messages.
- On Windows, `buffer-path` and the `'root` of a `lsp-server-status` entry no longer start with `\\?\`.
- HUME reports its working directory to the terminal (OSC 7), so a new WezTerm split or tab opens in HUME's directory, including after `:cd`.

## [0.14.0] - 2026-10-08

### Breaking changes
- Every scripting function with an effect ends in `!`: `declare-plugin!`, `load-plugin!`, `lsp-request!`, `lsp-notify!`, `after!`, `stdlib/run!`, `stdlib/write-file!`, `stdlib/delete-file!`, `stdlib/delete-dir!`.
- `buffer-cursor-line` and `offset->line` return 0-based lines.
- `on-mode-change` passes the old and new modes as symbols (`'insert`).
- `spawn-async!`, `run-capture!` and `stdlib/run!` take the working directory as `#:cwd`. `run-capture!` and `stdlib/run!` start their child in its own process group, so Ctrl-c during an inline-output command no longer reaches it.
- `buffer-selections` entries are opaque: read them with `stdlib/selection-anchor`, `-head`, `-start`, `-end` and `-primary?`. `stdlib/selection-start` and `-end` are new.
- `show-popup!` and `show-menu!` return a token, which `close-popup!`, `close-menu!` and `picker-close!` take.
- The `on-select` of `show-drawer-list!` and `update-drawer-list!` takes a second argument, the token of the drawer that called it.
- Decoration entries are symbol-keyed hashmaps: `set-signs!`, `set-eol-text!`, `set-extra-highlights!`, `set-virtual-lines!`, `set-line-backgrounds!` and `set-inlay-hints!` take `(hash 'line l 'text t 'scope s)` and the like.
- Builtins that returned a positional list or pair return a symbol-keyed hashmap: `diagnostic-counts`, `run-capture!`, `stdlib/run!`, `viewport-range`, `lsp-range->offsets`, `diff-lines`, `diff-buffer-lines`, `diff-words`, `lsp-locations->display-parts`. `goto-location!` and the `#:range` of `diagnostics-for-buffer` take one.
- `diagnostics-for-buffer` and `lsp-server-status` entries use symbol keys, and a diagnostic's `'severity` and a server's `'state` are symbols.
- Options renamed: `scrolloff` → `scroll-margin`, `autoread` → `auto-read`, `mouse-enabled` → `mouse`, `auto-pairs-enabled` → `auto-pairs`.
- Options that take one of a fixed set of names return a symbol from `get-option`/`get-buffer-option` and the `on-option-change` hook, and `set-option!` accepts a symbol or a string.
- `configure-statusline!` rejects an empty element name.
- `register-trigger-chars!` is `set-hook-triggers!`, and `completion-set-trigger-chars!` is `set-completion-triggers!`.
- `language` is a buffer option: use `set-buffer-option!`/`get-buffer-option`. `set-buffer-language!` and `buffer-language` are gone. `""` clears the language, and `on-language-set` passes `""` instead of `#f`.
- A typed command whose lambda declares no `arg` parameter rejects a typed argument, and one that declares no `force` parameter rejects `!`, instead of dropping them. A lambda with a rest parameter still receives both.
- `on-lsp-notification` is a hook, called with `(server method params)`. `register-lsp-notification-hook!` handles only chosen methods.
- `:lsp-install`, `:lsp-uninstall` and `:lsp-servers` moved to the new `core:lsp-install` plugin: add `(load-plugin! "core:lsp-install")` to keep them. `:lsp-servers` is `:lsp-catalog`, and `:lsp-rescan-servers` is removed.
- `:ls` lists buffers in the bottom drawer instead of a `[buffers]` buffer, so `(buffers)` and `on-text-changed` see no `[buffers]` buffer.
- The bare `(declare-plugin! "core:x")` form is removed, and `declare-plugin!` no longer takes `#:config`: use `(load-plugin! "core:x" #:config …)`. A plugin that ships a `manifest.scm` loads lazily.
- `hume-target`, `sha256-file`, `unpack-gz`, `unpack-zip`, `unpack-tar`, `mark-executable!`, `acquire-install-lock!` and `release-install-lock!` are removed.
- `register-lsp-server!` takes a server name and only says how to start it: `(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")`. A language's servers come from `set-language-servers!`. `unregister-lsp-server!`, `:lsp-stop` and `:lsp-restart` take that name, and `lsp-registered-for-language?` is replaced by `lsp-server-registered?`.
- `on-lsp-attach`, `on-lsp-detach` and `on-lsp-notification` pass a server instead of a language, and attach and detach fire once per server.
- `lsp-server-status` entries have `'name` and `'languages` instead of `'language`. `lsp-server-for-buffer` is replaced by `lsp-servers` and `lsp-server-name`.
- `lsp-request!` and `lsp-notify!` pick among a buffer's servers by method, `#:feature` or `#:to`, and a request no server can take calls back with the reason. A request callback's `err` is a hash with `'kind` and `'message`.
- `lsp-capabilities` takes a server, not a pane.
- A server stopped with `:lsp-stop` stays stopped until `:lsp-restart` or `:reload-config`.
- `completion-emit!` takes a list of entries: pass an `lsp-request!` answer as `(list res)`.
- `diagnostics-for-buffer` entries carry `'server`, and `lsp-locations->display-parts` rows carry `'location`.
- `lsp-position-params` and its siblings give each server position values in its own column units, not line and character numbers.

### Plugins
- `:plugin-status` also lists plugins that are not installed or whose `manifest.scm` failed.
- `declare-plugin!` takes `#:entry` to load a plugin in pieces, and can name a local `./file.scm` beside `init.scm` to load it lazily.
- `core:plum` loads its plugin, grammar and theme commands separately, each on first use.
- New `(plugin-dir)` returns the calling plugin's directory.
- New `(hume-version)`, `(hume-version>=? major minor patch)` and `(command-exists? name)` let a plugin adapt to the running editor.
- `set-eol-text!` takes `#:hide-on-insert-line`.
- `register-completion-source!` takes `#:token-chars` for extra token characters in a `'buffer` source.
- New `lsp-capability` reads what a server advertises for a feature or a method.
- `define-language!` takes `#:roots`, the file names that mark a project root for the language. Language servers start in the root found for the buffer's language.
- New `track-position!`, `tracked-position-params`, `untrack-position!` and `lsp-request!`'s `#:tracked` keep a position through edits.
- New `lsp-request-all!` sends a request to every server of a buffer that can answer and calls back once.
- `set-attachment-hook-triggers!` and `set-attachment-completion-triggers!` set trigger characters for one buffer's attachment to one server, for one feature.
- `lsp-request!` and `lsp-request-all!` take `#:unavailable 'empty`, which answers a request no server can take with an empty result instead of an error.
- `run-inline-output!` takes `#:env`.
- `core:buffer-words` indexes 100 lines on each side of the cursor per step, down from 200.
- New `core:undotree`: `:undotree` shows the buffer's undo tree in the bottom drawer, and `Enter` jumps to the highlighted revision. With `core:git-diff` loaded, it also shows inline what the current revision changed.
- New `buffer-undo-tree` and `goto-revision!` read the undo tree and jump to a revision, and the `on-undo-history-changed` hook fires when it changes.
- New `buffer-revision-diff` returns the line hunks, with word spans, between a revision of the undo history and the buffer's current content.
- New `git-diff/render-diff` and `git-diff/release-diff` in `core:git-diff` let a plugin draw its own hunks inline in place of the buffer's git diff.
- `show-drawer-list!` takes `#:selected` to open on a given row, and the drawer keeps the selected row visible.
- `show-drawer-list!` and `update-drawer-list!` take `#:render`, which draws only the visible rows and lets the items be any values.

### Editing
- Tab on the `:` line completes the text all candidates share; the next Tab picks the first. `command-completion=first-candidate` fills in the first candidate at once.
- Selections never split a character: text objects, motions, edits, search, `:sort`, join, paste and undo keep a letter with its combining marks, emoji sequences and flags whole.
- `f`/`t`/`F`/`T` match composed and decomposed forms alike, and `:sort` treats them as equal.
- `G C` capitalizes with title case.
- `#` matches tags with non-ASCII names.
- No-break and ideographic spaces count as indentation.
- A yank remembers whether it took whole lines, and pastes as lines or inline to match. Clipboard text counts as lines when it ends with a newline.
- `d` and `y` leave the file's last line break out of the register unless the selection covers whole lines. `d` or `c` that removes nothing leaves the register and kill ring alone.
- With several cursors, `d`, Backspace, `Ctrl-w` and pair deletion remove the union of overlapping ranges, and `J` joins a line once.
- Pasting lines over a selection adds a line break in front only when the selection does not start its line.
- `Tab` with several cursors moves each to its own tab stop, and copying a selection to the next or previous line keeps whole lines whole.
- `t`/`T` with a count stop against the counted match, and `=`/`-` after them move on to the next one.
- `X` and `x` select the cursor's own line.
- `mii` selects the last insertion after other edits.
- Search highlights follow a paste while it can still be cycled.
- Jumps from Insert mode record the right jump-list position and end the typed-text selection.
- A mouse drag extends from the selection's current anchor.
- Closing a pane that has a search or sift open ends it instead of crashing.
- Cancelling a search after a language server edited the buffer restores the original selection.
- A selection ending on a line break no longer covers virtual text at the end of that line.
- An inline diff's deleted lines wrap like the buffer line, so a word removed past the pane edge is visible.
- Typing a quote right after the same quote no longer opens a pair, so ```` ``` ```` and `"""` come out as typed.

### Files & buffers
- `:e path:line[:col]` jumps to a position on open.
- If HUME crashes or loses its terminal, it saves unsaved changes to a dump file and offers to restore them when the file is next opened.
- Text typed in Insert mode counts as unsaved at once: `[+]`, the tab marker, `:ls` and `buffer-dirty?` show it, and `:q`, `:qa`, `:bd` and `:e` refuse to drop it.

### Terminal & compatibility
- A crash in a background thread leaves the terminal intact. It shows in `:messages` and is printed after you quit.
- Yanks reach the clipboard over SSH through OSC 52 when the terminal allows it.

### Language servers
- A buffer uses every server in its language's list at once, such as ESLint beside typescript-language-server. New `set-language-servers!` chooses a language's servers, their order and their features, `set-default-language-servers!` gives a plugin's list, and `lsp-language-servers` reads the result.
- `:lsp-install` also takes a server name, such as ruff or tombi. With `core:lsp-install` loaded, every language starts with Helix's servers for it.
- A language server stops once no open buffer uses it.
- `:lsp-install` supports many more servers, including terraform-ls, jdtls and servers installed through `go`, `pip`, `gem` or `dotnet`, and says so when a server doesn't support your platform.
- The `:diagnostics` drawer names the tool that reported each diagnostic: its `source`, or the server's name when it sets none.
- Diagnostics from a server that only reports on request, such as ESLint, show. `lsp.diagnostics-pull-debounce-ms` sets how long HUME waits after typing to ask.
- Diagnostics are hidden on the line you are typing on in Insert mode. `lsp.diagnostics-on-insert-line` keeps them.
- `on-viewport-change` fires when the lines a pane shows change, not on a resize or scroll that shows the same lines.
- Completion items are filtered against the text their own edit range covers, a list's default edit range is honored, and accepting replaces that range.
- The completion menu stays open when text elsewhere changes, and closes once a character outside the completed word is typed.
- The `z r` list follows your edits, so `Enter` lands on the right line.

### Theming
- `gruvbox_light`'s picker cursor matches upstream's reversed style.
- A reversed cursor stays visible on an indent guide.

## [0.13.0] - 2026-09-28

### Breaking changes
- `--keys` (headless mode) loads `init.scm` and plugins like interactive mode: pass `--no-config` to skip them.
- `show-drawer-list!` returns a token that `close-drawer!` takes as its first argument, errors on an empty list, and calls the replaced drawer's callback with `#f`.
- Completion sources are registered with `register-completion-source!` and answer with `completion-emit!`: `completion-begin!`, `completion-add-items!`, `completion-update-filter!` and `on-completion-refilter` are gone.
- `Ctrl-Space` is the built-in `completion-trigger`, and `core:lsp`'s `lsp-completion-trigger` is gone: remove it from a `core:lsp` declaration's `#:commands`.
- LSP responses and capabilities, a diagnostic's `"raw"` field, notification params, an accepted completion item and `json-parse`'s result are JSON handles: read them with the new `json-ref`/`json-ref-or`/`json-contains?`/`json-list`/`json-array?`/`json-object?` instead of `hash-ref` and friends.
- Scripting targets an explicit pane instead of the focused buffer: a command's lambda receives the invoking pane as its first argument, every builtin that took a buffer id takes a pane, and `buffers` returns panes. `current-buffer` and `focused-buffer` are gone (use `focused-pane` where no pane is at hand), and so are `buffer-id?`, `pane-id?`, `buffer-id=?` and `pane-id=?` (compare with `equal?`).
- `current-selections`, `current-line-number` and `char-index->line` are `buffer-selections`, `buffer-cursor-line` and `offset->line`, each taking a pane.
- `switch-to-buffer!`, `show-popup!`, `show-menu!`, `show-drawer-list!`, `picker!`, `live-picker!`, `prompt!`, `lsp-show-status!`, `goto-location!`, `apply-workspace-edit!` and `stdlib/resolve-lang-arg` take a leading pane, and `lsp-request`, `lsp-notify`, `lsp-capabilities`, `lsp-stop!` and `lsp-restart!` a required one (`lsp-stop!`/`lsp-restart!` also accept a language name).
- A builtin given a closed pane, or one no longer showing its buffer, raises an error, and a closed buffer raises the same "invalid buffer id" error from every builtin.
- `get-option` reads only the global value: the new `get-buffer-option` reads a buffer's effective value.
- `apply-text-edits!` and the other builtins reading LSP positions accept only values from a server response, and `lsp-locations->display-parts` and `lsp-label-offsets->text` drop their buffer argument.
- Native commands run through `call!` act on the given pane instead of requiring it to be focused, except those tied to focus.

### CLI
- New `--no-config` flag skips `init.scm` and plugins, and `--config` also works with `--keys`.

### Editing
- `/`, `?`, and `s` accept leading flags: `m/` moves every selection to its own next match, `v/` matches literally.
- New `:earlier`/`:later` step through undo history by count (`:earlier 3`) or by age (`:earlier 5m`).
- `.` repeats what you did during an Insert session: an Insert-mode key bound to a command, such as `Ctrl-w`, an accepted completion (its text only, not an import it added elsewhere), a picker pick, and cursor movement such as an arrow key.

### Panes & interface
- New tab pages: `:tabnew`/`:tabclose`/`:tabnext`/`:tabprev`, the `tab-new`/`goto-next-tab`/`goto-prev-tab` commands (`Ctrl-p t`/`Ctrl-p T`), and a tab bar controlled by the new `tabline` setting.
- `Ctrl-d`/`Ctrl-u`/`PageDown`/`PageUp` scroll the view, not just the cursor.
- The bottom drawer's `Ctrl-d`/`Ctrl-u` page by half its height; `Shift-Down`/`Shift-Up` move one row.
- The file, buffer, and modified-files pickers open a selection in a new tab or split with `Ctrl-t`/`Ctrl-v`/`Ctrl-s`.
- Pasting while a picker is open goes to its query.
- Clicking or scrolling while a picker is open no longer affects the buffer beneath it.
- A click or scroll dismisses the "file changed on disk" prompt, like a stray key, and closes an open code-action menu.
- A `:` command that enters Insert mode stays in Insert.
- `:reload-config` returns to Normal mode.

### Language servers
- Hover, signature help, code actions, and references no longer pop up if you've moved on before the server answers.
- Signature help closes when you leave Insert mode.
- Completion triggered mid-word filters the list by what's already typed, and no longer offers a candidate identical to it.
- The completion menu shows each candidate's detail (e.g. its type) in its own column, and stays up while the server refreshes an incomplete list.
- Backspacing within a completed word keeps the completion menu open; backspacing past its start closes it.

### Plugins & scripting
- A plugin that fails to load no longer aborts the rest of `init.scm`, and its error points at the plugin's own file and line.
- New `core:buffer-words` plugin completes words already in the buffer, merged with language-server candidates without duplicates.
- New `register-completion-source!`/`completion-emit!` let a plugin provide Insert-mode or `:` command-line completions, and `completion-set-trigger-chars!` sets a source's trigger characters per language.
- `define-typed-command!` takes `#:complete` to complete its argument on `Tab`.
- `picker!`/`live-picker!` take `#:actions` for extra key bindings, and the new `stdlib/buffer-actions` provides the open-in-tab/split ones.
- New `update-drawer-list!` and `drawer-selected-index` refresh an open drawer in place.
- `lsp-request` takes `#:require-focus` to drop a response once you've moved to another pane.
- `close-menu!` and `close-drawer!` close their widget even when something else is open on top of it.
- `on-mode-change` no longer fires during a `.` repeat.
- New `buffer-panes`, `buffer-key`, `pane?`, `buffer-line-count`, and `split-words` builtins, and `debounce-by` takes `#:key`.
- New `insert-key!` lets a command bound to an Insert-mode key fall back to that key's normal behaviour.
- `show-menu!`/`show-drawer-list!` called from a mode that can't accept them log a warning naming the mode, instead of doing nothing.

### Fixes
- A plugin pasting while you type in Insert mode no longer drops your typing from undo.
- Mouse-wheel scrolling no longer gets stuck on inline virtual lines, such as an inline diff's deleted hunk.
- Scrolling to the end of a file no longer makes the view jump on the next cursor move.
- `j`/`k` at the top or bottom of a file collapse the selection.
- `o`/`O` keep the current line's indent.
- A key that dismisses the "file changed on disk" prompt also runs its own binding, and pasting while the prompt is open no longer edits the buffer.
- The `:diagnostics` drawer emptying while you type no longer drops you out of Insert mode.
- The bottom drawer no longer leaves blank rows after its list shrinks.
- Selecting a `:diagnostics` row after switching buffers jumps to the listed buffer.
- `:plum-install-plugins`/`:plum-update-plugins` fail instead of hanging when git asks for credentials.
- Counted undo/redo (`5u`, `3 Ctrl-r`) is much faster over a long history.
- A server capability set to `null` is treated as unsupported.
- A macro that finishes while still in Insert mode no longer discards the Insert session it started when you finish it and press `.`.

## [0.12.0] - 2026-09-08

### Breaking changes
- `}`/`{` select the whole paragraph, plus its trailing blank line, instead of just moving the cursor. New `mip`/`map` text objects select just the paragraph.
- Case transforms moved from `gu`/`gU`/`gC` to `GL`/`GU`/`GC`.
- Several default keys moved to keep `g` for goto motions: fuzzy finders move from `gf`/`gb`/`gm` to `zf`/`zb`/`zm`, rename moves from `gr` to `GR`, hover moves to bare `K`, and the viewport keys become `zk`/`zz`/`zj`.
- The vim-keybind plugin no longer binds `G`, which overrode HUME's own case-transform keys.
- `goto-alternate-file` is renamed `goto-alternate-buffer`.
- A handful of commands (LSP install/status, PLUM plugin/grammar management, git-diff sign toggles, `:lsp-fmt` → `:format-source`) can no longer be typed at the `:` prompt by their old bindable name: bind them to a key instead, or use their new typed command name.
- Gutter sign columns are reserved per buffer: a sign source declares its column with `(register-sign-source! name bid priority)`, and `set-signs!` entries drop the trailing priority.
- `selection-spans-full-line?` is `selections-linewise?` and checks every selection, with a new `selections-charwise?`. `lsp-range-params` is `lsp-primary-range-params`, with a new `lsp-linewise-ranges-params`.
- Select mode (the `s` regex prompt) is renamed **Sift mode**: the status bar shows `SIF` instead of `SEL`, and `select-within` is renamed `sift-within`.
- `select-changed-text` is renamed `select-inserted-text`.

### Editing
- Leaving Insert mode selects the text you just typed however you entered it, not only after `c`.
- New `#` jumps between a bracket or tag and its matching partner.
- Word motions, `miw`/`maw`, `Ctrl-w`, `*`, and quote auto-pairing honor the new `word-chars` buffer option, settable per language, so `foo-bar` can be one word instead of three.
- New `>`/`<` indent/unindent every selected line by one level (`3>` for three levels).
- `:format-source` formats a multi-line selection, and several selected line ranges at once when the server supports it, instead of the whole buffer.
- Count prefixes (`3w`, `12j`) are capped at 10,000.
- Pasting or typing unusual line-break characters no longer splits the buffer into an extra line.
- Files with old-style line endings (`\r\n`, bare `\r`) are normalized to `\n` everywhere text enters the buffer, not just on load.
- New tree-sitter text objects for functions, types, comments, unit tests, and array/struct values (`mif`/`maf`, `mit`/`mat`, `mic`/`mac`, `miu`/`mau`, `miv`/`mav`), for any language with textobject support.
- `mia`/`maa` (argument) handles nested lists, tuples, or structs inside a call.
- New `goto-next-`/`goto-prev-` navigation for each text-object kind above, bound under `g` on the same letter (e.g. `gf`/`gF` for functions).
- `}` and forward structural navigation re-center the view on the selected object instead of leaving it at the bottom of the screen (new `object-jump-align` setting).

### Files & buffers
- New `goto-next-buffer`/`goto-prev-buffer` commands, for binding to a key.

### Appearance
- New `cursor-shape-insert` setting (`block`/`bar`/`underline`, default `bar`) picks the terminal cursor's shape in Insert mode.
- Curly, dotted, dashed, and double-line underlines render correctly on terminals that support them.
- Whitespace indicator glyphs (spaces, tabs, newlines) are themable.
- Inline diagnostic messages use their own theme color instead of always matching the underlined squiggle.
- The bundled `dark`/`light`/`gruvbox` themes are replaced by ports of Helix's `gruvbox` and `gruvbox_light`: the bundled themes are `sand`, `gruvbox` and `gruvbox_light`.
- Themes can use TOML section headers as well as HUME's flat dotted-key format.
- The theme loader follows Helix more closely (search-match highlighting, statusline mode colors, underline/modifier names, git-diff's gutter and line colors).
- The pane seam divider takes the theme's base text color when `ui.window` sets only a background.

### Configuration & options
- New `--config <FILE>` flag loads a config file other than the default `init.scm`; `:reload-config` re-evaluates the same file.
- File arguments can include a `:LINE` or `:LINE:COLUMN` suffix to open at that position (e.g. `hume src/main.rs:42:5`).

### Plugins
- New scripting support for structural text objects, live/streaming pickers, git helpers, and a per-buffer statusline element plugins can write to.
- `core:git-diff` can show the current git branch in the statusline.
- Clearer error messages when a key or `:` command targets a command that isn't reachable that way.
- New `:plum-install-theme`, `:plum-update-themes`, `:plum-list-themes`, and `:plum-remove-theme` manage third-party themes from GitHub, as PLUM does plugins and grammars.

### Fixes
- "Nothing to do here" refusals (no search match, unknown buffer name, mistyped setting, …) flash a message instead of being logged as errors.
- `Ctrl-o`/`Ctrl-i` jumps land correctly even after the buffer was edited, undone, or reloaded.
- `p` on an empty line pastes onto that line instead of the next one.
- `:reload-config` no longer leaves plugin-driven statusline elements (e.g. the git branch) blank until the next buffer switch or save.
- The sign column no longer shifts or resizes as gutter signs come and go.
- Diagnostic gutter markers no longer render underlined.
- `.` replays the selection step, not just the edit, for `m/`, `ms`, `C`, `,`, `S`, `_`, `(`/`)`, `Ctrl-,`, `Ctrl-e` and `select-word-nearest-on-line`.
- The bracket-match highlight no longer treats `<`/`>` as a pair, so it no longer misfires on generics or comparisons.
- The alternate buffer (`Ctrl-6`, `#`, `:b#`) follows visit order, so it keeps toggling with the buffer you actually came from.
- `:split`/`:vsplit` resize every pane on that axis equally instead of just halving the one being split, and closing a pane redistributes its space evenly.

## [0.11.0] - 2026-08-25

### Breaking changes
- `core:pickers`, `core:vim-keybind` and `core:lsp` require `core:stdlib` declared or loaded first.
- `core:plum`'s plugin commands are renamed `:plum-install-plugins`, `:plum-cleanup-plugins`, `:plum-update-plugins`, `:plum-list-plugins` (were `:plum-install`, `:plum-cleanup`, `:plum-update`, `:plum-list`).
- `set-inline-diagnostics!` is renamed `set-eol-text!` and takes a `source` argument first: `(set-eol-text! source bid entries)`.
- `set-inlay-hints!` takes a `source` argument first, `(set-inlay-hints! source bid hints)`, and each hint's position is a buffer char offset instead of an LSP wire position.
- `set-virtual-lines!`'s entries are hashmaps (`(hash 'line ... 'text ... 'scope ... 'anchor ... 'segments ...)`) instead of positional `(line text scope)` lists, and `'segments` are char offsets, not byte offsets.
- `declare-plugin`'s `#:events` entries must be symbols (e.g. `'(on-buffer-save)`), not strings.
- `(viewport-range bid)` returns `(first-line . end-line)` with `end-line` exclusive: drop any `(+ 1 (cdr vr))`. The `on-viewport-change` hook's third argument is renamed `end-line`.

### Editing
- `C` honours a count prefix: `3C` duplicates each selection onto the 3 lines below.
- `.` no longer replays `C`/`copy-selection-on-{next,prev}-line` as part of a selection.
- `core:vim-keybind`'s `C` (default `'smart` config) takes `copy-selection-on-next-line` with any count prefix, not just when a selection is already active.
- The kill ring dedupes its entries.
- `p`/`P` run the new `smart-paste-after`/`smart-paste-before`, which paste from the kill ring until something is edited after your last delete, change or yank, and from the clipboard after that. `paste-after`/`paste-before` are unbound.
- New `:sort` command sorts selected rows by their selected text.

### Files & buffers
- HUME notices when an open file changes on disk and offers to reload it, controlled by the `autoread` option. `:w`/`:wa` refuse to overwrite a changed file unless forced with `!`.

### Panes & interface
- The buffer picker (`g b`) shows each buffer's full display path instead of a `:pwd`-relative one.
- New `core:pickers` picker, `g m`, lists files with staged or unstaged git changes.
- `picker!` takes `#:pending`, which shows a loading indicator until the first results arrive.
- Scrollable popups and menus show a scrollbar.
- The whole statusline tints with the current mode's color, not just the mode indicator. Opt out with the new `statusline.mode-colors` option.
- `:messages` entries are colored by severity.

### Configuration & options
- `:reload-config` is a full reset: keymaps, options, hooks, commands, and plugins all go back to their defaults before `init.scm` re-runs. Buffers, undo history, and running language servers are untouched.
- `mouse-enabled`/`mouse-select` and `jump-list-capacity` apply immediately when changed with `:set`, instead of only at startup.
- `wrap-mode` is a buffer option, and `:set global wrap-mode=…` applies to buffers already open. A pane's `:wrap` pin is kept per buffer.

### Plugins & scripting
- New `core:git-diff` plugin: `:toggle-git-signs` shows git changes as gutter signs and `:toggle-inline-diff` inline. Requires `core:stdlib` declared or loaded first.
- New `buffer-text`/`buffer-lines` builtins return a buffer's live, unsaved content.
- New `diff-words`, `diff-lines` and `diff-buffer-lines` builtins compute word- and line-level diffs between two texts, or between a text and a buffer's live content.
- New `set-line-backgrounds!` builtin sets a full-line background tint.
- New `lsp-position->offset`/`lsp-range->offsets` builtins convert an LSP wire position or range into buffer char offsets.
- New `on-option-change`, `on-text-changed`, `on-buffer-enter` and `on-focus-gained` hooks.
- New `spawn-async!`/`cancel-async!` builtins run a subprocess in the background without blocking the editor.
- `core:steel-server` no longer flags HUME's own commands and configuration functions as unknown identifiers while you edit `init.scm` or a plugin file.
- Syntax highlighting no longer requires the `core:plum` plugin to be loaded: installed grammars register automatically at startup.
- `set-option!` can be called from a hook or command body, not just `init.scm`.

### Terminal & compatibility
- Quitting with Ctrl-C/SIGTERM/SIGHUP/SIGQUIT shuts down language servers gracefully and exits with the conventional `128 + signal` code.
- Tagged release builds show a clean `--version` string, with no commit-hash suffix.

### Fixes
- `:plum-list-plugins`/`:plum-install-plugins`/`:plum-update-plugins` no longer fail on a stray file such as `.DS_Store` among installed plugins.
- `C`/`copy-selection-on-{next,prev}-line` no longer lands a copy one column off after a tab or wide character, or merges a multi-line selection's copy into the original.
- The fuzzy picker's Ctrl-u/Ctrl-d move the selection by half a page.
- A closed terminal with no controlling process no longer leaves a HUME process spinning at 100% CPU.
- Opening a `.tsx`/`.jsx` file no longer makes the language server log an "Invalid languageId" warning.
- Quitting with an attached language server no longer leaves the screen frozen in the alternate screen while it shuts down: the terminal is restored first.
- `d`/`c`/`p` on a read-only buffer no longer overwrite the kill ring or a named register before refusing the edit.
- A hover or diagnostic popup closes on a mouse scroll or click.
- `.` after `[`/`]` (paste-ring cycling) repeats it.
- `.` keeps the last repeatable action when its replay is refused on a read-only buffer.

## [0.10.0] - 2026-07-24

### Language servers
- Full LSP support: hover, goto definition, references, rename, code actions, formatting, signature help.
- In-buffer autocompletion.
- Diagnostics: underlines, gutter signs, inline messages, statusline counts, `gn`/`gp` navigation, `:diagnostics` list.
- Inlay hints.
- One-command server installation (`:lsp-install`) from a bundled catalog.
- Manual server registration from config via `register-lsp-server!`.
- `core:steel-server` plugin: a Steel language server for editing your own config and plugins.
- Hover docs and other overflow popup/drawer content are syntax-highlighted, with scroll affordance arrows shown when there's more to see.

### Editing
- After changing text with `c`, leaving Insert mode now selects the text you just typed. Controlled by the new `select-changed-text` option (default on).
- `mii` selects the last insertion.
- Case transforms: `gu`, `gU`, `gC`.
- Word motions and `mm`/`MM` now also select adjacent whitespace. Controlled by the new `word-selects-whitespace` option (default on).
- New `undo-levels` option caps the number of undo states kept per buffer (default `0`, unlimited).
- Character prompts (`r`, `t`, `f`, …) accept Enter and Tab.
- Extend-mode `o` (flip selection) moved into the `core:vim-keybind` plugin.

### Panes & interface
- Fuzzy file and buffer finders (`core:pickers` plugin): `g f` / `g b`. Plugins can build their own with `picker!`.
- Configurable sign column in the gutter (`signcolumn` option).
- `Diagnostics` statusline element, in the default statusline.
- Indentation guides can now be hidden via the new `indent-guides` option (default on).

### Plugins & scripting
- `manifest.scm`: a bare `(declare-plugin "name")` is enough for lazy loading.
- Expanded plugin API: LSP requests, timers, new hooks, decorations, popups, menus, drawer lists, minibuffer prompts.
- `set-buffer-option!` builtin for per-buffer setting overrides from hooks and commands.

### Terminal & compatibility
- Bracketed paste — pastes land in one step, without auto-pairing.
- Event-driven main loop: HUME sleeps when idle instead of polling.
- Kitty keyboard protocol now works on Windows (Windows Terminal ≥ 1.25).

### Theming
- Theme editor rebuilt as a proper web app.
- `sand` theme refinements.

### Fixes
- `:wq` quit the whole editor instead of closing the focused pane.
- Crash when joining lines with a cursor on the last line.
- Syntax highlighting precedence for overlapping captures.
- `(set-option! ...)` from a lazily-activated plugin now takes effect immediately.
- `:set global theme=<name>` matches `:theme <name>`'s existing behavior.
- Minibuffer history recall (Up/Down in `:`, `/`, `?`) now filters to entries starting with the text typed before recalling.
- Statusline shows `*scratch*` and other synthetic buffer names instead of leaving the file-path element blank.
- Windows: statusline file paths and `:e`'s duplicate-buffer detection no longer choke on the `\\?\` canonical-path prefix.

## [0.9.0] - 2026-07-15

First tagged release. HUME has been under active development for a while; this is the point where it's considered stable enough to hand out prebuilt binaries rather than requiring a build from source.

### Editing model
- Multiple selections and multi-cursor editing.
- Full Unicode correctness.
- Registers and kill ring, with system-clipboard integration.
- Keyboard macros, count prefixes, dot-repeat, undo/redo tree.
- Incremental search, search-based multi-cursor selection, jump list.

### Syntax & language awareness
- Tree-sitter-powered syntax highlighting.
- Multi-layer language injections.
- Grammar installation and management built in.

### Plugins & scripting
- Steel (Scheme) scripting for configuration and plugins.
- Built-in plugin manager (PLUM) for installing and managing plugins.
- Lazy plugin loading.

### Panes & interface
- Split panes with directional focus movement, seam dividers, and focus dimming.
- Tab completion in the command line.
- Multi-buffer workflow.
- Configurable status line.

### Theming
- Hierarchical, scope-based theming compatible with Helix themes.

### Terminal & compatibility
- True color and synchronized output by default.
- Kitty keyboard protocol support with automatic fallback to legacy key encoding.
- Runs on macOS, Linux, and Windows.
