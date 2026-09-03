# core:plum

**PLUM** — the HUME **PLU**gin **M**anager. Installs and updates third-party Steel plugins
from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

Requires `core:stdlib` declared (or loaded) first — grammar/plugin install and cleanup call
`stdlib/find`, `stdlib/write-file`, `stdlib/delete-dir`, `stdlib/delete-file`,
`stdlib/list-subdirs`, `stdlib/run`, `stdlib/resolve-lang-arg` via `call!`.

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:plum")
```

PLUM is not privileged — it's a plugin like any other, so it must be brought in explicitly
too. Disabling it only removes the management commands below; anything already installed
keeps working without it, *including* syntax highlighting — registering already-compiled
grammars at startup is core's job (see [Syntax Highlighting](https://cvlmtg.github.io/HUME/syntax-highlighting.html)),
not PLUM's. PLUM is only needed to *install* a plugin or grammar in the first place.

With no explicit `#:commands`/`#:events`/`#:languages`, this reads `core:plum`'s own
`manifest.scm`, which declares every `:plum-*` command — so it activates the first time one
is typed. `(load-plugin "core:plum")` also works, loading it eagerly instead. See
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-plum) and
[Syntax Highlighting](https://cvlmtg.github.io/HUME/syntax-highlighting.html) for the
grammar workflow.

## Commands

| Command | Effect |
|---|---|
| `:plum-install-plugins` | Install all plugins declared in `init.scm` not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins no longer declared |
| `:plum-update-plugins` | Run `git pull` in every installed third-party plugin |
| `:plum-list-plugins` | Log declared / installed / orphan / missing plugin lists |
| `:plum-install-grammar` | Install (or repair) a named grammar: purges old source, re-clones, recompiles (default: current buffer's language) |
| `:plum-list-grammars` | Log declared / installed / orphan / missing grammar lists |
| `:plum-cleanup-grammars` | Delete compiled grammar files no longer declared |

`plum-ensure-grammars` — install any of the given (list of) grammar names not yet
compiled — is not in the table above: it's a plain editor command, not a `:` command, and
takes a list argument, so it's for `init.scm` (`(call! "plum-ensure-grammars" '("rust" "json"))`),
not the command mode prompt.

LSP language servers are `core:lsp`'s own responsibility (`:lsp-install`, `:lsp-uninstall`,
`:lsp-servers`) — see that plugin's README and `docs/LSP-INSTALL.md` in the repository. PLUM
never touches `<data>/servers/` or the LSP catalogs.

## How it works

### File layout

PLUM bundles two independent subsystems:

- `plugin.scm` — entry point; `require`s the two subsystems below.
- `plugins.scm` — third-party **plugin** install/update/cleanup (`:plum-install-plugins` etc).
- `grammars.scm` — tree-sitter **grammar** install pipeline (`:plum-install-grammar` etc);
  builds on the source catalog and path helpers core registers at startup (see "Grammar
  sources and the Helix pin" below).
- `lib.scm` — shared utilities: `plum/read-file`, `plum/run!` (a `core:stdlib`
  `stdlib/run` wrapper that raises instead of returning a status), and `plum/batch-run`
  (batch installs), used by `plugins.scm` and `grammars.scm` as needed. Directory listing,
  filesystem cleanup, and list search live in `core:stdlib` (`stdlib/list-subdirs`,
  `stdlib/find`, `stdlib/write-file`, `stdlib/delete-dir`, `stdlib/delete-file`) — reached
  via `call!`, not local wrappers.

### Plugin discovery

Declared plugins live in `init.scm`; installed plugins are discovered by walking
`<data>/plugins/<user>/<repo>/` and checking for a `plugin.scm` in each leaf directory
(`plum/installed-plugins`). "Missing" and "orphan" are just set differences between the
declared list and that walk — nothing is cached, so `:plum-list-plugins` always reflects the current
disk state.

### Grammar sources and the Helix pin

Grammar source metadata (repo URL, pinned revision, tree-sitter symbol, subpath) and the
path helpers built on it (`grammar-output-path`, `grammar-highlights-path`,
`installed-grammars`, …) are core, not PLUM — `runtime/scheme/grammars.scm` owns them and, at
startup, registers any already-compiled grammar it finds, before PLUM (or any other plugin)
ever loads. It finds them by listing `<data>/grammars/` rather than probing every catalog
entry, and reads `runtime/scheme/grammar-sources.scm` only on first use, so a setup with no
grammars installed pays a single `path-exists?` for the whole subsystem. PLUM's `grammars.scm`
calls those same bindings for its install pipeline; it doesn't declare its own copy.
Syntax-highlighting and structural-text-object queries
(`highlights.scm`, `injections.scm`, `textobjects.scm`) aren't authored in HUME — they're
fetched from the Helix project's `runtime/queries/` at a pinned commit
(`runtime/scheme/helix-pin.scm`, read once at PLUM's own load), so HUME rides Helix's
query-file curation without vendoring it.

`plum/try-fetch-injections!` and `plum/try-fetch-textobjects!` each tolerate a missing query
file (most grammars don't have every kind) instead of letting a 404 abort the whole install —
an unusual case where letting the failure fail silently, rather than fail fast, is correct: no
query file just means no injection highlighting, or no structural text objects, for that
grammar, not a broken install.

A query file can declare `; inherits: dep,dep,...` instead of writing out its own patterns —
a directive naming other query sources whose patterns should be spliced in (the JS-family
grammars — `js`, `jsx`, `ts`, `tsx` — share most of their patterns this way). tree-sitter has
no notion of this, so `plum/resolve-query` resolves the chain itself: it recursively fetches
each named dependency's copy of the same file and splices the results together before
anything is written to disk.

No deduplication: this mirrors Helix's own resolver, which concatenates without deduping. A
grammar reachable by two `inherits` paths (a "diamond") would be spliced twice — harmless
(tree-sitter tolerates duplicate patterns) and exactly what Helix produces. The JS/TS family
(HUME's only multi-dependency `inherits` case) is a flat one-level star at the pinned Helix
commit — `tsx`'s bases (`ecma`, `_typescript`, `_jsx`) are all leaves with no `inherits` line
of their own — so no diamond arises today, but the resolver doesn't rely on that staying true.

### Grammar install pipeline

`:plum-install-grammar` always installs from a clean slate, which doubles as the repair path
for a grammar left in a failed state (e.g. a source tree cloned but never compiled):

1. Install any not-yet-compiled dependency grammars first (see "Grammar dependencies" below).
2. Purge any existing source tree.
3. Blobless clone (skip file-history blobs) at the pinned revision, then check out that exact
   revision.
4. Download the highlights query, resolving any `; inherits:` chain.
5. Compile: tree-sitter build → shared lib (preceded by a status line — the C compiler itself
   is silent, which on a slow grammar would otherwise read as a hang).
6. Download the Helix injections query, if any (best-effort — most grammars have none).
7. Download the Helix textobjects query, if any (best-effort — most grammars have none).
8. Register the grammar for its language in this session.

### Grammar dependencies

A grammar can have injection dependencies — e.g. Markdown's `(inline)` injection only
resolves if the `markdown.inline` grammar is also compiled and attached, even though nothing
about a plain Markdown install signals that. `plum/install-grammar-deps!` installs declared
dependencies (`*plum-grammar-deps*`) before the grammar itself, so `:plum-install-grammar` on
a Markdown buffer transparently pulls in `markdown.inline` too — the user never needs to
discover the dependency exists.

### Startup registration is core's job, and passive

`register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor startup —
whether or not PLUM is declared in `init.scm` — and registers every already-compiled grammar
in `<data>/grammars/`: no subprocess, no network. Grammars declared but not yet compiled stay missing
until the user explicitly runs `:plum-install-grammar`, or `init.scm` calls `plum-ensure-grammars`;
nothing auto-installs on startup, since a first run with many declared languages could otherwise mean
a long, surprising stall before the editor is usable.

`installed-grammars` (and so `plum/orphan-grammars`/`:plum-cleanup-grammars`) only ever sees
files matching *this platform's* shared-library extension — a `.so` left behind in a
`<data>/grammars/` shared with a Linux setup, for instance, is invisible to both registration
and cleanup on macOS, not reported as an orphan and deleted. Remove it by hand if that ever
comes up; it's inert either way (never dlopen'd on the platform it doesn't match).
