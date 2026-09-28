# core:plum

**PLUM**: the HUME **PLU**gin **M**anager. Installs and updates third-party Steel
plugins and themes from GitHub, and installs the tree-sitter grammars that power syntax
highlighting.

PLUM keeps no state files of its own. Every "what's installed" question is answered by
walking disk at the moment it's asked (see [Plugin discovery](#plugin-discovery) below).

## Usage

```scheme
(declare-plugin! "core:stdlib")
(declare-plugin! "core:plum")
```

- **Depends on:** `core:stdlib`: plugin/grammar/theme install and cleanup call
  `stdlib/find`, `stdlib/write-file`, `stdlib/delete-dir`, `stdlib/delete-file`,
  `stdlib/list-subdirs`, `stdlib/run`, `stdlib/resolve-lang-arg` via `call!`.
- **Activates on:** the first `:plum-*` command typed. `(load-plugin! "core:plum")` also
  works, loading it eagerly instead.
- PLUM is not privileged. It's a plugin like any other, so disabling it only removes the
  management commands below. Anything already installed keeps working without it,
  *including* syntax highlighting: registering already-compiled grammars at startup is
  core's job (see [Startup registration](#startup-registration-is-cores-job-and-passive)
  below), not PLUM's. PLUM is only needed to *install* a plugin or grammar in the first
  place.
- LSP language servers are `core:lsp`'s own responsibility (`:lsp-install`,
  `:lsp-uninstall`, `:lsp-servers`); see that plugin's README and `docs/LSP-INSTALL.md`
  in the repository. PLUM never touches `<data>/servers/` or the LSP catalogs.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-plum)
  and [Syntax Highlighting](https://cvlmtg.github.io/HUME/syntax-highlighting.html) for
  the grammar workflow.

## Commands

| Command | Effect |
|---|---|
| `:plum-install-plugins` | Install all plugins declared in `init.scm` not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins no longer declared |
| `:plum-update-plugins` | Run `git pull` in every installed third-party plugin |
| `:plum-list-plugins` | Log declared / installed / orphan / missing plugin lists |
| `:plum-install-grammar` | Install (or repair) a named grammar: purges old source, re-clones, recompiles (default: current buffer's language). Tab-completes declared grammar names |
| `:plum-list-grammars` | Log declared / installed / orphan / missing grammar lists |
| `:plum-cleanup-grammars` | Delete compiled grammar files no longer declared |
| `:plum-install-theme` | Install (or reinstall) a theme repo's `themes/*.toml` by `user/repo` GitHub slug |
| `:plum-update-themes` | Run `git pull` in every installed theme repo and re-sync its `.toml` copies |
| `:plum-list-themes` | Log installed theme repos, the theme names each provides, and any unmanaged `.toml` |
| `:plum-remove-theme` | Remove an installed theme repo's `.toml` copies and its clone, by `user/repo` slug. Tab-completes installed slugs |

`plum-ensure-grammars`, which installs any of the given (list of) grammar names not yet
compiled, isn't in the table above: it's a plain editor command, not a `:` command, and
takes a list argument, so it's for `init.scm` (`(call! "plum-ensure-grammars" '("rust"
"json"))`), not the command-mode prompt.

## How it works

PLUM bundles three independent subsystems over shared plumbing:

- **`plugins.scm`**: third-party **plugin** install/update/cleanup.
- **`grammars.scm`**: tree-sitter **grammar** install pipeline; builds on the source
  catalog and path helpers core registers at startup (see
  [Grammar sources and the Helix pin](#grammar-sources-and-the-helix-pin)).
- **`themes.scm`**: third-party **theme** install/update/list/remove (see
  [Theme install](#theme-install)).
- **`lib.scm`**: shared utilities: `plum/clone-github!`/`plum/git-pull!` (the one
  GitHub-clone URL shape and its update-side counterpart every install command shares),
  `plum/batch-run` (see [Output model](#output-model) below), and `plum/two-level-repos`
  (the `<root>/<user>/<repo>/` discovery walk shared by plugin and theme-repo discovery).
  Directory listing, filesystem cleanup, list search, and path-segment validation live in
  `core:stdlib` instead, reached via `call!`.

### Path safety

PLUM validates every name that reaches `path-join` or a subprocess argument without
coming from a fixed catalog (either half of a GitHub `user/repo` slug typed by the user,
and a dependency name parsed out of downloaded content) with `core:stdlib`'s
`stdlib/safe-path-segment?`. See that plugin's README for the rejected set, why the `:`
rejection matters on Windows, and why every call site tests `(eq? #t …)` rather than bare
truthiness.

### Plugin discovery

Declared plugins live in `init.scm`; installed plugins are discovered by walking
`<data>/plugins/<user>/<repo>/` and checking for a `plugin.scm` in each leaf directory.
"Missing" and "orphan" are set differences between the declared list and that walk;
nothing is cached, so `:plum-list-plugins` always reflects the current disk state.

### Grammar sources and the Helix pin

Grammar source metadata (repo URL, pinned revision, tree-sitter symbol, subpath) and the
path helpers built on it are core, not PLUM: `runtime/scheme/grammars.scm` owns them
and, at startup, registers any already-compiled grammar it finds, before PLUM (or any
other plugin) ever loads. It finds them by listing `<data>/grammars/` rather than probing
every catalog entry, and reads `runtime/scheme/grammar-sources.scm` only on first use, so
a setup with no grammars installed pays a single `path-exists?` for the whole subsystem.
PLUM's `grammars.scm` calls those same bindings for its install pipeline; it doesn't
declare its own copy.

Syntax-highlighting and structural-text-object queries (`highlights.scm`,
`injections.scm`, `textobjects.scm`) aren't authored in HUME. They're fetched from the
Helix project's `runtime/queries/` at a pinned commit (`runtime/scheme/helix-pin.scm`,
read once at PLUM's own load), so HUME rides Helix's query-file curation without
vendoring it.

`plum/fetch-raw-query` (the `curl` call behind every query fetch) deliberately does *not*
wrap that call in a `with-handler`; only the temp-file cleanup after it succeeds is
guarded. Re-raising a native-builtin error through a nested `with-handler` corrupts Steel
0.8.3's VM continuation stack when it runs somewhere an outer handler also sits (see the
[core plugins index](../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)).
This was hit for real once already, in this same plugin's theme-install command (see
[Theme install](#theme-install) below), which is why the pattern is avoided here too, even
though `curl`'s own raise is never itself caught.

A query file can declare `; inherits: dep,dep,...` instead of writing out its own
patterns: a directive naming other query sources whose patterns should be spliced in
(the JS-family grammars `js`, `jsx`, `ts`, and `tsx` share most of their patterns this
way). tree-sitter has no notion of this, so `plum/resolve-query` resolves the chain
itself: it recursively fetches each named dependency's copy of the same file and splices
the results together before anything is written to disk. There's no deduplication. This
mirrors Helix's own resolver, which concatenates without deduping. A grammar reachable by
two `inherits` paths (a "diamond") would be spliced twice, harmless since tree-sitter
tolerates duplicate patterns. The JS/TS family (HUME's only multi-dependency `inherits`
case) is a flat one-level star at the pinned Helix commit (`tsx`'s bases, `ecma`,
`_typescript`, `_jsx`, are all leaves with no `inherits` line of their own), so no diamond
arises today, but the resolver doesn't rely on that staying true.

`plum/try-fetch-injections!`/`plum/try-fetch-textobjects!` each tolerate a missing query
file (most grammars don't have every kind) instead of letting a 404 abort the whole
install. This is an unusual case where failing silently, rather than fast, is correct: no query
file just means no injection highlighting, or no structural text objects, for that
grammar, not a broken install.

### Grammar install pipeline

`:plum-install-grammar` always installs from a clean slate, which doubles as the repair
path for a grammar left in a failed state (e.g. a source tree cloned but never compiled):

1. Install any not-yet-compiled dependency grammars first (see
   [Grammar dependencies](#grammar-dependencies) below).
2. Purge any existing source tree.
3. Blobless clone (skip file-history blobs) at the pinned revision, then check out that
   exact revision.
4. Download the highlights query, resolving any `; inherits:` chain.
5. Compile: tree-sitter build → shared lib (preceded by a status line, since the C compiler
   itself is silent, which on a slow grammar would otherwise read as a hang).
6. Download the Helix injections query, if any (best-effort; most grammars have none).
7. Download the Helix textobjects query, if any (best-effort; most grammars have none).
8. Register the grammar for its language in this session.

### Grammar dependencies

A grammar can have injection dependencies. For example, Markdown's `(inline)` injection only
resolves if the `markdown.inline` grammar is also compiled and attached, even though
nothing about a plain Markdown install signals that. `plum/install-grammar-deps!`
installs declared dependencies (`*plum-grammar-deps*`) before the grammar itself, so
`:plum-install-grammar` on a Markdown buffer transparently pulls in `markdown.inline`
too, so the user never needs to discover the dependency exists.

### Startup registration is core's job, and passive

`register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor
startup (whether or not PLUM is declared in `init.scm`) and registers every
already-compiled grammar in `<data>/grammars/`: no subprocess, no network. Grammars
declared but not yet compiled stay missing until the user explicitly runs
`:plum-install-grammar`, or `init.scm` calls `plum-ensure-grammars`; nothing
auto-installs on startup, since a first run with many declared languages could otherwise
mean a long, surprising stall before the editor is usable.

### Theme install

A theme repo is a GitHub `user/repo` with a `themes/*.toml` directory at its root (e.g.
[everforest.hume](https://github.com/cvlmtg/everforest.hume)). `:plum-install-theme
<user/repo>` clones it and copies that directory's `.toml` files flat into
`<data>/themes/`, the search tier `hume-editor`'s theme loader and `:theme <Tab>`
completer already read (both only ever glob `*.toml` files there, non-recursively).
`:theme <name>` picks up an installed theme immediately, with no `:reload-config`, unlike a
plugin, since theme lookup hits the filesystem at load time and the completer re-scans
on every `Tab`.

A user-typed slug that isn't shaped like `"user/repo"` at all is an ordinary usage
mistake: a status-line-only `log! 'info`, not a `:messages` entry. A slug that *is*
shaped right but has a segment `stdlib/safe-path-segment?` rejects is different in kind:
it would otherwise reach `path-join` and `git clone`, so it raises and stays in
`:messages` instead.

Theme repos are discovered by walking `<data>/themes/sources/<user>/<repo>/` for a
`.git` marker, not a `themes/` directory. A repo must stay discoverable (and so
removable) even after upstream drops its `themes/` directory and `:plum-update-themes`
starts failing its sync on every run. There's no state file recording which repo "owns"
a given `<data>/themes/<name>.toml` copy, so two installed repos providing the same theme
name would otherwise overwrite each other's file silently; the sync step instead logs a
`'warn` naming both repos whenever it's about to write a name another installed repo also
provides.

The clone itself is kept, at `<data>/themes/sources/<user>/<repo>/`, the direct analog
of `<data>/grammars/sources/<name>/`. A `sources/` *directory* has no extension, so it
never matches the `*.toml` glob either consumer runs, exactly like a grammar's source
tree is invisible to the compiled-grammar scan. Keeping it is what lets
`:plum-update-themes`/`:plum-list-themes`/`:plum-remove-theme` work with no separate
state file: the clone itself is the provenance record for which repo a given
`<data>/themes/<name>.toml` came from.

`:plum-install-theme` always installs from a clean slate, the same discipline
`:plum-install-grammar` uses: it purges any existing clone for that slug before cloning,
which doubles as the repair path. If the clone succeeds but the repo turns out to have no
`themes/*.toml`, the sync step raises and the stale clone is left on disk, a harmless
leftover, overwritten by the same purge on the next `:plum-install-theme` attempt for
that slug. This is deliberate, not an oversight: catching that failure to clean up
immediately would need wrapping a call that raises via a native-backed builtin
(`plum/clone-github!`) in a `with-handler` that catches and re-raises (see the Steel
pitfall in the [core plugins index](../README.md#steel-pitfalls-worth-knowing-before-you-hit-them)).
Letting the purge-on-next-attempt do the cleanup avoids the footgun entirely.

Removing the theme currently active in `:theme` leaves it loaded in memory until the next
`:theme <name>` or restart; `:plum-remove-theme` only touches files on disk.

## Output model

Every command that reaches the network (`plum-install-plugins`, `plum-update-plugins`,
`plum-install-grammar`, `plum-install-theme`, `plum-update-themes`) is `#:inline-output`:
it leaves the alt-screen on its first real write, so `git`'s own progress prints live
instead of vanishing behind a frozen-looking screen, and returns to the editor on a
keypress once the run finishes.

A batch command's per-item progress line (`plum/batch-run`) is painted with `displayln`,
not `log!`: `log!` only buffers until the whole command returns, then collapses into one
status-line slot, so every per-item line but the last would be silently lost while the
batch is still running. `displayln` is gated shut for a non-`#:inline-output` caller
(`plum-cleanup-plugins`, `plum-cleanup-grammars`), which is fine: those finish instantly
and keep their summary `log!` line.

## Known limitations

- **Foreign-platform compiled files are invisible.** `installed-grammars` (and so
  `plum/orphan-grammars`/`:plum-cleanup-grammars`) only ever sees files matching *this
  platform's* shared-library extension. A `.so` left behind in a `<data>/grammars/`
  shared with a Linux setup, for instance, is invisible to both registration and cleanup
  on macOS, not reported as an orphan and deleted. Remove it by hand if that ever comes
  up; it's inert either way, never `dlopen`'d on the platform it doesn't match.
- **Removing the active theme doesn't unload it** (see
  [Theme install](#theme-install) above).
