# `runtime/scheme/`

Scheme evaluated at editor startup, before any plugin or `init.scm` runs. Two hand-written
files (`prelude.scm`, `grammars.scm`), one pin (`helix-pin.scm`), and the language data
generated from it by the scripts in `../../scripts/` (see that directory's `README.md` for the
regeneration run order, not repeated here). The LSP server catalogs live with their reader in
`../plugins/core/lsp-install/`.

| File | What | Loaded |
|---|---|---|
| `prelude.scm` | hand-written: `syntax-rules` sugar over the raw builtins | startup, see below |
| `languages.scm` | generated: language detection data | startup, see below |
| `languages/<name>.scm` | generated: one language's data file | when the language is first used |
| `grammars.scm` | hand-written: registers already-compiled grammars | startup, see below |
| `helix-pin.scm` | hand-written: pinned `helix-editor/helix` commit SHA | input to `sync-grammars.py` |

## Load order

`hume-editor/src/editor/scripting_setup.rs`'s `init_scripting` evaluates, in order:

1. `builtins/bootstrap.scm`: embedded in `hume-scripting` via `include_str!`, not part of
  this directory; defines `load-plugin!`/`declare-plugin!` and the inline-activation machinery.
2. `prelude.scm`
3. `languages.scm`
4. `grammars.scm`: reads the data file of each language whose grammar is installed, and no other.
5. the user's `init.scm`
6. plugins named by `load-plugin!`/`declare-plugin!` in `init.scm`.

Each file is read via `host.runtime_dir()`; a missing runtime directory or a missing file is a
silent no-op, not an error. Only a file that exists but fails to parse is reported.

The files in `languages/` are never evaluated. They are parsed as data by one reader in
`builtins/bootstrap.scm`, which also checks the record format below. The editor calls it when a
buffer is first assigned a language, and the `bundled-language-grammar` and
`bundled-grammar-languages` functions are built on it, so a plugin reads the files through those
functions rather than opening them itself.

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

The editor reads a language's file the first time a buffer is assigned that language, and fills
each of `roots` and `language-id` that configuration has not set. A value from
`configure-language!` always wins, whether it arrives before or after the file is read; among
configured values the first wins, so `init.scm` beats a plugin, and the ignored write is
logged as a warning. `:reload-config` drops both and reads the files again on next use.

## `grammars.scm`

Passive: registers already-compiled grammars found on disk, no subprocess, no network.
Installing *new* grammars is `core:plum`'s job (`runtime/plugins/core/plum/grammars.scm`);
this file only makes already-installed ones take effect, so highlighting survives PLUM being
absent from `init.scm`.

- **Registration is driven by the install directory.** `<data>/grammars/` holds one compiled
  file per installed grammar, named after its language, and doesn't exist until something is
  installed, so a fresh setup settles the whole question with one `path-exists?`. Startup
  reads the data file of each installed language for its grammar's C symbol, and no other.
- **The platform-extension filter doubles as existence proof.** `read-dir` already proves a
  file exists; matching it against the platform's shared-library extension (`.dylib`/`.dll`/
  `.so`, from `(current-os!)`, defaulting to `.so` for anything unrecognized) does the whole
  job a second `stat` would, and drops a stale cross-platform file (a leftover `.so` on macOS)
  before it can reach `dlopen`.
- **Two distinct failure modes, two responses.** A compiled grammar whose language has no
  bundled grammar (dropped by a HUME update) is an orphan: silently skipped, expected. A
  language with a bundled grammar whose compiled file is missing its `highlights.scm` (e.g. the
  user cleared `<data>/grammars/sources/` to reclaim disk) is repairable, so it warns instead,
  pointing at `:plum-install-grammar`, so `:plum-list-grammars` doesn't keep reporting it
  "installed" with no highlighting and no explanation.
- **`injections.scm` and `textobjects.scm` are both optional, `highlights.scm` isn't.** Each is
  passed as `#f` when absent from the grammar's source directory; only a missing
  `highlights.scm` blocks registration.

## The prelude

See `prelude.md` in this directory.
