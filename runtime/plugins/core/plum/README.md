# core:plum

**PLUM** — the HUME **PLU**gin **M**anager. Installs and updates third-party Steel plugins
from GitHub, installs the tree-sitter grammars that power syntax highlighting, and downloads
and manages LSP language servers.

## Usage

```scheme
(load-plugin "core:plum")
```

PLUM is not privileged — it's a plugin like any other, so it must be loaded explicitly too.
Disabling it only removes the management commands below; anything already installed keeps
working without it.

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

LSP server management:

| Command                | Effect                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| `:lsp-install [lang]`  | Download, verify, and unpack the server for a language (default: current buffer's language); registers it via `core:lsp` if that plugin is loaded, otherwise warns |
| `:lsp-uninstall <name>`| Shut down and unregister a server's clients, remove it from disk (by server name, not language) |
| `:lsp-servers`         | Catalog listing: every seeded server, its languages, and install status      |

PLUM never calls `register-lsp-server!` itself — see "Startup registration is passive"
below and `core:lsp`'s README for the `:lsp-rescan-servers` command it owns.

See `docs/LSP-INSTALL.md` for the design and `user-manual/docs/lsp.md#installing-servers` for
the user-facing workflow.

## How it works

### File layout

PLUM is the one core plugin split across multiple files, since it bundles three independent
subsystems:

- `plugin.scm` — entry point; `require`s the three subsystems below and runs startup
  grammar registration (server registration is `core:lsp`'s job — see below).
- `plugins.scm` — third-party **plugin** install/update/cleanup (`:plum-install` etc).
- `grammars.scm` — tree-sitter **grammar** install pipeline (`:plum-install-grammar` etc).
- `servers.scm` — LSP **server** install pipeline (`:lsp-install` etc) — download,
  verify, unpack, receipt, uninstall, catalog listing. Registration lives in
  `core:lsp/registration.scm`, not here; see `docs/LSP-INSTALL.md`.
- `lib.scm` — shared utilities: `plum/valid-dir-entry?` (used by all three) and
  `plum/batch-run` (used by `plugins.scm`/`grammars.scm` for batch installs — `servers.scm`'s
  install/uninstall are single-target, so it doesn't need it).

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

LSP servers are the exception, not a variant of this rule: PLUM installs them but never
registers one for use. `core:lsp`'s `lsp/register-installed-servers!` runs the equivalent
scan — same passive contract (readable `receipt.scm`, recorded absolute bin path, no
`$PATH` lookup, no subprocess) — at `core:lsp`'s own load or lazy activation. PLUM asks it
to rescan right after a successful (or already-up-to-date) `:lsp-install`, via `call!`; if
`core:lsp` isn't loaded, PLUM warns instead of silently leaving the server unusable. A
buffer whose language has a seeded, installable server but nothing registered gets a
one-line `on-language-set` hint — `:lsp-install` if it isn't installed yet, or "load
`core:lsp`" if it already is. See `docs/LSP-INSTALL.md` for the full rationale.
