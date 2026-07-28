# core:plum

**PLUM** — the HUME **PLU**gin **M**anager. Installs and updates third-party Steel plugins
from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

## Usage

```scheme
(declare-plugin "core:plum")
```

PLUM is not privileged — it's a plugin like any other, so it must be brought in explicitly
too. Disabling it only removes the management commands below; anything already installed
keeps working without it, *including* syntax highlighting — registering already-compiled
grammars at startup is core's job (see [Syntax Highlighting](../../../../user-manual/docs/syntax-highlighting.md)),
not PLUM's. PLUM is only needed to *install* a plugin or grammar in the first place.

With no explicit `#:commands`/`#:events`/`#:languages`, this reads `core:plum`'s own
`manifest.scm`, which declares every `:plum-*` command — so it activates the first time you
type one.

`(load-plugin "core:plum")` also works, loading it eagerly instead.

## Commands

Plugin management:

| Command         | Effect                                                   |
|------------------|-----------------------------------------------------------|
| `:plum-install`  | Install all plugins declared in `init.scm` not yet on disk |
| `:plum-cleanup`  | Remove on-disk plugins no longer declared                  |
| `:plum-update`   | Run `git pull` in every installed third-party plugin       |
| `:plum-list`     | Log declared / installed / orphan / missing plugin lists   |

Grammar management:

| Command                   | Effect                                                          |
|-----------------------------|--------------------------------------------------------------------|
| `:plum-install-grammar`    | Install (or repair) a named grammar: purges old source, re-clones, recompiles (default: current buffer's language) |
| `:plum-ensure-grammars`    | Install any of the given (list of) grammar names not yet compiled  |
| `:plum-list-grammars`      | Log declared / installed / orphan / missing grammar lists           |
| `:plum-cleanup-grammars`   | Delete compiled grammar files no longer declared                    |

LSP language servers are `core:lsp`'s own responsibility (`:lsp-install`, `:lsp-uninstall`,
`:lsp-servers`) — see that plugin's README and `docs/LSP-INSTALL.md`. PLUM never touches
`<data>/servers/` or the LSP catalogs.

## How it works

### File layout

PLUM bundles two independent subsystems:

- `plugin.scm` — entry point; `require`s the two subsystems below.
- `plugins.scm` — third-party **plugin** install/update/cleanup (`:plum-install` etc).
- `grammars.scm` — tree-sitter **grammar** install pipeline (`:plum-install-grammar` etc);
  builds on the source catalog and path helpers core registers at startup (see "Grammar
  sources and the Helix pin" below).
- `lib.scm` — shared utilities: `plum/valid-dir-entry?` and `plum/batch-run` (batch
  installs), used by both `plugins.scm` and `grammars.scm`.

### Plugin discovery

Declared plugins live in `init.scm`; installed plugins are discovered by walking
`<data>/plugins/<user>/<repo>/` and checking for a `plugin.scm` in each leaf directory
(`plum/installed-plugins`). "Missing" and "orphan" are just set differences between the
declared list and that walk — nothing is cached, so `:plum-list` always reflects the current
disk state.

### Grammar sources and the Helix pin

Grammar source metadata (repo URL, pinned revision, tree-sitter symbol, subpath) and the
path helpers built on it (`grammar-output-path`, `grammar-highlights-path`, …) are core, not
PLUM — `runtime/scheme/grammars.scm` declares them from `runtime/scheme/grammar-sources.scm`
unconditionally at startup, before PLUM (or any other plugin) ever loads, and registers any
already-compiled grammar it finds. PLUM's `grammars.scm` calls those same bindings for its
install pipeline; it doesn't declare its own copy. Syntax-highlighting queries
(`highlights.scm`, `injections.scm`) aren't authored in HUME — they're fetched from the Helix
project's `runtime/queries/` at a pinned commit (`runtime/scheme/helix-pin.scm`, read once at
PLUM's own load), so HUME rides Helix's query-file curation without vendoring it.

`plum/try-fetch-injections!` tolerates a missing `injections.scm` (most grammars don't have
one) instead of letting `curl-fetch`'s 404 abort the whole install — an unusual case where
letting the failure fail silently, rather than fail fast, is correct: no query file just
means no injection highlighting for that grammar, not a broken install.

A query file can declare `; inherits: dep,dep,...` instead of writing out its own patterns —
a directive naming other query sources whose patterns should be spliced in (the JS-family
grammars — `js`, `jsx`, `ts`, `tsx` — share most of their patterns this way). tree-sitter has
no notion of this, so `plum/resolve-query` resolves the chain itself: it recursively fetches
each named dependency's copy of the same file and splices the results together before
anything is written to disk. `plum/fetch-query!` is the drop-in replacement for a plain
`curl-fetch` of a query file that also resolves this.

### Grammar dependencies

A grammar can have injection dependencies — e.g. Markdown's `(inline)` injection only
resolves if the `markdown.inline` grammar is also compiled and attached, even though nothing
about a plain Markdown install signals that. `plum/install-grammar-deps!` installs declared
dependencies (`*plum-grammar-deps*`) before the grammar itself, so `:plum-install-grammar` on
a Markdown buffer transparently pulls in `markdown.inline` too — the user never needs to
discover the dependency exists.

### Startup registration is core's job, and passive

`register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor startup —
whether or not PLUM is declared in `init.scm` — and registers any already-compiled grammar
found on disk: no subprocess, no network. Grammars declared but not yet compiled stay missing
until the user explicitly runs `:plum-install-grammar` or `:plum-ensure-grammars`; nothing
auto-installs on startup, since a first run with many declared languages could otherwise mean
a long, surprising stall before the editor is usable.
