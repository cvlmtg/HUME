# core:plum

**PLUM** — the HUME **PLU**gin **M**anager. Installs and updates third-party Steel plugins
from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

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

## How it works

### File layout

PLUM is the one core plugin split across multiple files, since it bundles two independent
subsystems:

- `plugin.scm` — entry point; `require`s the two subsystems below and runs startup grammar
  registration.
- `plugins.scm` — third-party **plugin** install/update/cleanup (`:plum-install` etc).
- `grammars.scm` — tree-sitter **grammar** install pipeline (`:plum-install-grammar` etc).
- `lib.scm` — shared utilities (`plum/valid-dir-entry?`, `plum/batch-run`) used by both.

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
