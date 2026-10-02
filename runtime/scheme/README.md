# `runtime/scheme/`

Scheme evaluated at editor startup, before any plugin or `init.scm` runs. Two hand-written
files (`prelude.scm`, `grammars.scm`), one pin (`helix-pin.scm`), and the catalogs generated
from it by the scripts in `../../scripts/` (see that directory's `README.md` for the
regeneration run order, not repeated here). The LSP server catalogs live with their reader in
`../plugins/core/lsp-install/`.

| File | What | Loaded |
|---|---|---|
| `prelude.scm` | hand-written: `syntax-rules` sugar over the raw builtins | startup, see below |
| `languages.scm` | generated: default language identities | startup, see below |
| `grammars.scm` | hand-written: registers already-compiled grammars | startup, see below |
| `grammar-sources.scm` | generated: tree-sitter grammar catalog | lazily, by `grammars.scm` |
| `helix-pin.scm` | hand-written: pinned `helix-editor/helix` commit SHA | input to `sync-grammars.py` |

## Load order

`hume-editor/src/editor/scripting_setup.rs`'s `init_scripting` evaluates, in order:

1. `builtins/bootstrap.scm`: embedded in `hume-scripting` via `include_str!`, not part of
  this directory; defines `load-plugin!`/`declare-plugin!` and the inline-activation machinery.
2. `prelude.scm`
3. `languages.scm`
4. `grammars.scm`: reads `grammar-sources.scm` lazily, only once a grammar is actually
  installed or a `:plum-*-grammar` command runs, not at startup.
5. the user's `init.scm`
6. plugins named by `load-plugin!`/`declare-plugin!` in `init.scm`.

Each file is read via `host.runtime_dir()`; a missing runtime directory or a missing file is a
silent no-op, not an error. Only a file that exists but fails to parse is reported.

## Reading a generated catalog from a plugin

Both generated files here are single literal sexprs, read with the same R7RS idiom:

```scheme
(call-with-input-file
  (path-join (runtime-dir) "scheme" "<file>.scm")
  read)
```

`grammar-sources.scm` and `languages.scm`/`grammars.scm` are a client-agnostic pair: identities
in `languages.scm`, grammar metadata behind them read only on demand.

**`grammar-sources.scm`**: one `(name git-url rev symbol subpath)` 5-tuple per grammar. All
fields fully canonicalised at sync time; no defaults applied at read time.

## `languages.scm`

Identity only: extensions, globs, shebangs, and an optional `#:language-id` override for the
`languageId` sent to language servers (present only when it differs from the name, e.g. `tsx`
→ `typescriptreact`). No grammars are shipped here; installing one is `core:plum`'s job
(`:plum-install-grammar`). Override any entry in your own `init.scm`. `define-language!`
replaces the prior identity for that name and keeps any grammar already attached to it (see
`runtime/init.scm.example` for override examples).

## `grammars.scm`

Passive: registers already-compiled grammars found on disk, no subprocess, no network.
Installing *new* grammars is `core:plum`'s job (`runtime/plugins/core/plum/grammars.scm`);
this file only makes already-installed ones take effect, so highlighting survives PLUM being
absent from `init.scm`.

- **The catalog is lazy and boxed.** `grammar-sources.scm` holds 350+ 5-tuples; parsing it is
  a measurable slice of startup a fresh setup would pay for nothing, so it's read once on
  first use and cached in a `box` rather than `set!` on a plain global: `core:plum` reaches
  these bindings from inside a `require`d module, and `box` is the pattern already proven
  across that boundary (see `debounce` in `hume-scripting/src/builtins/bootstrap.scm`).
- **Registration is driven by the install directory, not the catalog.** `<data>/grammars/`
  holds one compiled file per installed grammar and doesn't exist until something is
  installed, so a fresh setup settles the whole question with one `path-exists?` instead of
  probing 350+ catalog entries that cannot match.
- **The platform-extension filter doubles as existence proof.** `read-dir` already proves a
  file exists; matching it against the platform's shared-library extension (`.dylib`/`.dll`/
  `.so`, from `(current-os!)`, defaulting to `.so` for anything unrecognized) does the whole
  job a second `stat` would, and drops a stale cross-platform file (a leftover `.so` on macOS)
  before it can reach `dlopen`.
- **Two distinct failure modes, two responses.** A compiled grammar with no catalog entry
  (dropped by a HUME update) is an orphan: silently skipped, expected. A catalog-known
  grammar missing its `highlights.scm` (e.g. the user cleared
  `<data>/grammars/sources/` to reclaim disk) is repairable, so it warns instead, pointing at
  `:plum-install-grammar`, so `:plum-list-grammars` doesn't keep reporting it "installed" with
  no highlighting and no explanation.
- **`injections.scm` and `textobjects.scm` are both optional, `highlights.scm` isn't.** Each is
  passed as `#f` when absent from the grammar's source directory; only a missing
  `highlights.scm` blocks registration.

## The prelude

See `prelude.md` in this directory.
