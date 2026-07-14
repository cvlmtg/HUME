# core:plum

**PLUM** — the HUME **PLU**gin **M**anager. Installs and updates third-party Steel plugins
from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

## Usage

```scheme
(declare-plugin "core:plum")
```

PLUM is not privileged — it's a plugin like any other, so it must be brought in explicitly
too. Disabling it only removes the management commands below; anything already installed
keeps working without it.

With no explicit `#:commands`/`#:events`/`#:languages`, this reads `core:plum`'s own
`manifest.scm`, which declares `#:languages '("*")` (any buffer with a detected language)
plus every `:plum-*` command — so it activates on the first buffer with a language, or the
first `:plum-*` command you type, whichever comes first. The language trigger runs before a
buffer's tree-sitter highlighting is wired up, so an already-compiled grammar still
registers (`plum/register-installed-grammars!`) in time for that buffer, matching eager
`load-plugin` behavior.

`(load-plugin "core:plum")` also works, loading it eagerly instead.

**Caveat**: passing a custom `#:commands`/`#:events` override to `declare-plugin` without also
naming `#:languages` bypasses the manifest entirely (all-or-nothing) — PLUM then won't register
already-installed grammars until the first `:plum-*` command runs, so buffers opened before
that render with no syntax highlighting until you do. Keep `#:languages '("*")` (or a narrower
language list) in any custom override, or load `core:plum` eagerly instead.

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

- `plugin.scm` — entry point; `require`s the two subsystems below and runs startup
  grammar registration.
- `plugins.scm` — third-party **plugin** install/update/cleanup (`:plum-install` etc).
- `grammars.scm` — tree-sitter **grammar** install pipeline (`:plum-install-grammar` etc).
- `lib.scm` — shared utilities: `plum/valid-dir-entry?` and `plum/batch-run` (batch
  installs), used by both `plugins.scm` and `grammars.scm`.

### Plugin discovery

Declared plugins live in `init.scm`; installed plugins are discovered by walking
`<data>/plugins/<user>/<repo>/` and checking for a `plugin.scm` in each leaf directory
(`plum/installed-plugins`). "Missing" and "orphan" are just set differences between the
declared list and that walk — nothing is cached, so `:plum-list` always reflects the current
disk state.

### Grammar sources and the Helix pin

Grammar source metadata (repo URL, pinned revision, tree-sitter symbol, subpath) is declared
once via `plum/declare-grammar-source!`, loaded from `runtime/scheme/grammar-sources.scm` at
plugin load time. Syntax-highlighting queries (`highlights.scm`, `injections.scm`) aren't
authored in HUME — they're fetched from the Helix project's `runtime/queries/` at a pinned
commit (`runtime/scheme/helix-pin.scm`, read once at load), so HUME rides Helix's
query-file curation without vendoring it.

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

### Startup registration is passive

`plum/register-installed-grammars!` runs once at plugin load and registers any
already-compiled grammar found on disk — no subprocess, no network. Grammars declared but not
yet compiled stay missing until the user explicitly runs `:plum-install-grammar` or
`:plum-ensure-grammars`; PLUM never auto-installs on startup, since a first run with many
declared languages could otherwise mean a long, surprising stall before the editor is usable.
