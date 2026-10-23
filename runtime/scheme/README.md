# `runtime/scheme/`

Scheme evaluated at editor startup, before any plugin or `init.scm` runs. Three hand-written
files (`prelude.scm`, `shells.scm`, `grammars.scm`), one pin (`helix-pin.scm`), and the language data
generated from it by the scripts in `../../scripts/` (see that directory's `README.md` for the
regeneration run order, not repeated here). The LSP server catalogs live with their reader in
`../plugins/core/lsp-install/`.

| File | What | Loaded |
|---|---|---|
| `prelude.scm` | hand-written: `syntax-rules` sugar over the raw builtins, and the `insert-register` command with its default keys | startup, see below |
| `shells.scm` | hand-written: the shell dialects and the `\|` line's completion source | startup, see below |
| `languages.scm` | generated: language detection data | startup, see below |
| `languages/<name>.scm` | generated: one language's data file | when the language is first used or a grammar is registered for it |
| `grammars.scm` | hand-written: registers already-compiled grammars | startup, see below |
| `helix-pin.scm` | hand-written: pinned `helix-editor/helix` commit SHA | input to `sync-grammars.py` |

## Load order

`hume-editor/src/editor/scripting/init.rs`'s `init_scripting` evaluates, in order:

1. `builtins/bootstrap.scm`: embedded in `hume-scripting` via `include_str!`, not part of
  this directory; defines `load-plugin!`/`declare-plugin!` and the inline-activation machinery.
2. `prelude.scm`
3. `shells.scm`
4. `languages.scm`
5. `grammars.scm`: reads the data file of each language whose grammar is installed, and no other.
6. the user's `init.scm`
7. plugins named by `load-plugin!`/`declare-plugin!` in `init.scm`.

Each file is read via `host.runtime_dir()`; a missing runtime directory or a missing file is a
silent no-op, not an error. Only a file that exists but fails to parse is reported.

The files in `languages/` are never evaluated. They are parsed as data by one reader in
`builtins/bootstrap.scm`, which also checks the record format below. The editor calls it when a
buffer is first assigned a language or a grammar is registered for the language, and the
`bundled-language-grammar` and `bundled-grammar-languages` functions are built on it, so a plugin
reads the files through those functions rather than opening them itself.

## `shells.scm`

Tab on the `|` line completes by the rules of the shell the `shell` option runs. Those rules
live here so a config can add a dialect for any shell; the contract `register-shell-dialect!`
takes is documented for users in `user-manual/docs/writing-plugins.md` ("Shell dialects").

A word has two forms. Its *display* is the text on the line (`"my dir/a b"`, `my\ dir/a\ b`);
its *value* is what the shell reads for it (`my dir/a b`). Listing, prefix matching and the
shared prefix of several names work on values. A dialect converts between the two, once each
way: `parse` reads the line into a `shell-word`, `write` puts a value back on the line. Nothing
else reads shell syntax, the editor included.

The file holds, in order:

- **The word record** `shell-word`: `start` and `end` (the span, in characters), `command?`,
  `typed` (display: the word up to its last directory, plus a quote that opens right after it),
  `dir` (the value of `typed`), `open` (the quote open at the end of `typed`, or `#f`) and
  `prefix` (the value from the end of `typed` to the cursor). `typed` goes back on the line as
  the user wrote it, so `~/`, `$HOME/` and an escaped `\~/` survive completion. When the
  directory part ends on a variable that a following name would extend (`$TMPDIR`, whose value
  ends in `/`), `typed` also gets that separator, so a name is written as `$TMPDIR/foo`, never
  `$TMPDIRfoo`.
- **The registry**: `register-shell-dialect!` and the list it fills. A dialect is found by the
  file name of the `shell` program, its directory, case and `.exe` aside. The dialect registered
  last wins, so `init.scm` and plugins, which run after this file, replace a shipped one by
  registering the same name or claiming the same program.
- **Shared scanning**: `%shell-scan` walks the line once with a dialect's `%shell-rules` (its
  escape character and the quotes it works inside, its quotes, operators, the operators and
  keywords after which a program name follows, its directory separators, whether `~` names the
  home directory) and one hook, `(read open c rest)`, which decodes the units only that shell
  has (`$NAME`, `%NAME%`, PowerShell's doubled quotes, fish's `\n`) as a `%shell-unit`: its
  value, the chars it uses after `c`, and whether name characters after it would extend it
  (`$NAME` and `$env:NAME` yes, `${NAME}` and `%NAME%` no); or `#f` for a character that reads
  as itself. Each unit's value is kept only when the unit ends
  at or before the cursor: `my\‸ dir` completes `my`. A unit whose value ends with a separator
  ends the directory part, so an escaped `/` or a variable ending in one counts, and a separator
  inside quotes does too. Offsets are chars, the unit Steel indexes strings by and the one the
  completion source contract uses.
- **The dialects**: POSIX `sh` (also `bash`, `zsh`, `dash`, `ksh`, `ash`, `mksh`, `yash`, `posh`,
  `busybox`), fish, PowerShell and `cmd`,
  each a `%shell-rules` value, its `read` hook, and its writer. `shell/posix-parse` and
  `shell/posix-write` (and the same for `fish`, `pwsh`, `cmd`) are public so a new dialect can
  reuse them. `PATHEXT` is read once, at load.
- **The `|` completion source** `shell`, registered `'delegated`: it parses the word, checks
  the record and each text the dialect writes (a malformed one raises, naming the dialect),
  lists programs for a word in command position whose `dir` is empty (so `"gr` lists programs
  too) and directory entries otherwise (`path-programs`, `path-entries`, matched on values),
  writes each name finished, gives a directory's item the LSP Folder kind, which the line's
  Enter goes into, and with several names writes
  their shared value prefix unfinished as `#:prefix`, so the first `Tab` inserts it, or `#f`
  when that prefix adds nothing to what is typed, so no escaped text is cut. With no
  dialect for the shell it answers nothing, which closes the menu, and logs a warning the first
  time per `shell` program until the file is evaluated again: completing by another shell's
  rules could insert text this shell reads differently.

### The dialects' rules

| Dialect | Reads | Writes |
|---|---|---|
| POSIX | `\` escapes outside single quotes; inside double quotes only before `` $ ` " \ `` and a newline. `~/` at the word start, `$NAME`, `${NAME}`. Words end at `\| ; & ( ) < >`; a program follows `\| ; & (` | a backslash before each special character, a newline as `'\n'`; inside an open quote, only what that quote needs (`'\''` in single quotes) |
| fish | `\` escapes outside quotes (`\n`, `\t`, `\r` are control characters, any other character reads as itself); in single quotes only `\'` and `\\`, in double quotes `\"`, `\$`, `\\`. `~/`, `$NAME`. A program also follows `and`, `or`, `not` | a backslash before each special character, braces included; `\n` and `\t` for a newline and a tab |
| PowerShell | the backtick escapes outside single quotes (in double quotes `` `n `` and `` `t `` are control characters); `''` and `""` inside their quotes. `~` before a separator, `$env:NAME`, `${env:NAME}`, `$HOME`. Words also end at `{ } ,`, and a program follows `{`. `\` and `/` separate directories | a path that needs quoting in single quotes from the word start, `'` doubled; inside an open double quote, a backtick before `` ` " $ `` |
| `cmd` | `^` escapes outside quotes, `"` quotes, `%NAME%` anywhere. `\` and `/` separate directories, so `"C:\Program Files\"` ends its quote | a path that needs quoting in double quotes from the word start |

A finished name closes the quote the word is in, a directory's too, so a completed word stands
on its own; the next `Tab` parses it again, and its `typed` part keeps that quote open for the
name after it. An unfinished shared prefix leaves its quote open, since the user keeps typing
into it. PowerShell (on Windows) and `cmd` end a directory with `\`.

## `languages.scm`

Detection data only: each language's extensions, globs and shebangs. Override any entry in
your own `init.scm`. `define-language!` replaces the prior detection data for that name and
keeps any grammar and per-language configuration already attached to it. The user-facing
override examples are in `user-manual/docs/syntax-highlighting.md`.

## `languages/<name>.scm`

One file per language, named after the language (`markdown.inline.scm`), holding a single
literal alist. Every field is canonicalised at sync time; a key is absent when the language has
no value for it, so a language with none is `()`:

```scheme
((comment-tokens "//" "///" "//!")
 (block-comment-tokens ("/*" . "*/") ("/**" . "*/") ("/*!" . "*/"))
 (language-id . "typescriptreact")
 (roots "Cargo.toml" "Cargo.lock")
 (grammar "url" "rev" "tree_sitter_rust" ""))
```

- **`comment-tokens`, `block-comment-tokens`**: line comment tokens and `(start . end)` block
  comment pairs, normalised the way Helix's own deserializer reads its several accepted shapes.
- **`language-id`**: the `languageId` sent to language servers, present only when it differs
  from the name (`tsx` → `typescriptreact`).
- **`roots`**: file names that mark the language's project root.
- **`grammar`**: `(url rev symbol subpath)` of the tree-sitter grammar the language uses. A
  language that borrows another's grammar carries a copy of it, with the C symbol of the
  grammar it borrows (`jsx` gets `tree_sitter_javascript`). `subpath` is the directory of the
  grammar's `src/` within its clone, empty for the clone root.

The editor reads a language's file the first time a buffer is assigned that language, or a
syntax layer parses text in it (a fenced code block), and fills each of `roots`, `language-id`,
`comment-tokens` and `block-comment-tokens` that configuration has not set. It reaches the editor
as a hash keyed by field, without `grammar`, which only the plugin-facing functions read.
A value from `configure-language!` always wins, whether it arrives before or after the file is read. A value
from `init.scm` beats a plugin's in either order, and the plugin's is dropped quietly; between two
values from the same side the first wins and the ignored write is logged as a warning. `:reload-config` drops both and reads the files again on next use.

## `grammars.scm`

Passive: registers already-compiled grammars found on disk, no subprocess, no network.
Installing *new* grammars is `core:plum`'s job (`runtime/plugins/core/plum/grammars.scm`);
this file only makes already-installed ones take effect, so highlighting survives PLUM being
absent from `init.scm`.

- **Registration is driven by the install directory.** `<data>/grammars/` holds one compiled
  file per installed grammar, named after its language, and doesn't exist until something is
  installed, so a fresh setup settles the whole question with one `path-exists?`. Startup
  reads the data file of each installed language for its grammar's C symbol, and no other.
- **Registering opens nothing.** A grammar registered here has its library loaded and its
  queries compiled the first time a buffer of its language, or an embedded block naming it,
  needs it, so startup pays nothing for grammars a session never uses. A library or query file
  that fails to load is reported then, once, as a warning naming the grammar.
- **The platform-extension filter doubles as existence proof.** `read-dir` already proves a
  file exists; matching it against the platform's shared-library extension (`.dylib`/`.dll`/
  `.so`, from `(current-os!)`, defaulting to `.so` for anything unrecognized) does the whole
  job a second `stat` would, and drops a stale cross-platform file (a leftover `.so` on macOS)
  before it is registered.
- **Two distinct failure modes, two responses.** A compiled grammar whose language has no
  bundled grammar (dropped by a HUME update) is an orphan: silently skipped, expected. A
  language with a bundled grammar whose compiled file is missing its `highlights.scm` (e.g. the
  user cleared `<data>/grammars/sources/` to reclaim disk) is repairable, so it warns instead,
  pointing at `:plum-install-grammar`, so `:plum-list-grammars` doesn't keep reporting it
  "installed" with no highlighting and no explanation.
- **`highlights.scm` is required, every other query file is optional.** The optional kinds come
  from the `grammar-optional-queries` builtin, an association list of `(key . file-name)`
  pairs, one per kind the editor can compile. `register-grammar-from-sources!` registers
  `highlights.scm` plus each of those files present in the grammar's source directory, so a
  new kind needs no change here. Only a missing `highlights.scm` blocks registration.
- **`grammar-query-path` names a query file in a grammar's source directory.** Startup
  registration and PLUM's install both go through it.

## The prelude

See `prelude.md` in this directory.
