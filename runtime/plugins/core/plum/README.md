# core:plum

**PLUM**: the HUME **PLU**gin **M**anager. It installs and updates third-party Steel
plugins and themes from GitHub, and installs the tree-sitter grammars that power syntax
highlighting.

PLUM keeps no state files. Every "what's installed" question is answered by walking disk
when it is asked (see [Plugin discovery](#plugin-discovery)).

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

- **Depends on:** `core:stdlib`. PLUM calls `stdlib/find`, `stdlib/write-file!`,
  `stdlib/delete-dir!`, `stdlib/delete-file!`, `stdlib/list-subdirs`,
  `stdlib/safe-path-segment?` and `stdlib/resolve-lang-arg` through `call!`.
- **Activates on:** the first `:plum-*` command typed, or the first call to
  `plum-ensure-grammars`. Its `manifest.scm` has three entries, each loading on its own
  commands: `plugin.scm` (the four plugin commands), `grammars.scm` (`#:entry`;
  `plum-ensure-grammars` in `#:commands`, the grammar commands in `#:typed-commands`) and
  `themes.scm` (`#:entry`; the theme commands).
- **Not required for what is already installed.** PLUM is a plugin like any other, so
  leaving it out removes only the management commands below. Installed plugins, grammars
  and themes keep working, including syntax highlighting: registering already-compiled
  grammars at startup is core's job (see
  [Startup registration](#startup-registration-is-cores-job-and-passive)). PLUM is needed
  to install a plugin or grammar.
- **LSP servers are not PLUM's.** `core:lsp-install` owns them; see its
  [README](../lsp-install/README.md). PLUM never touches `<data>/servers/` or the LSP
  catalogs.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-plum)
  and [Syntax Highlighting](https://cvlmtg.github.io/HUME/syntax-highlighting.html) for
  the grammar workflow.

## Commands

| Command | Effect |
|---|---|
| `:plum-install-plugins` | Install all plugins declared in `init.scm` that are not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins that are no longer declared |
| `:plum-update-plugins` | Run `git pull` in every installed third-party plugin |
| `:plum-list-plugins` | Show the declared, installed, orphan and missing plugin lists in `[plum-plugins]` |
| `:plum-install-grammar [lang]` | Install or repair a grammar: purge its source, re-clone, recompile. Defaults to the current buffer's language. Tab-completes the languages with a bundled grammar |
| `:plum-list-grammars` | Show the installed and orphan grammars in `[plum-grammars]` |
| `:plum-cleanup-grammars` | Delete the orphan grammars: compiled files whose name is not a language with a bundled grammar |
| `:plum-install-theme <user/repo>` | Install or reinstall a theme repo's `themes/*.toml` from a GitHub slug |
| `:plum-update-themes` | Run `git pull` in every installed theme repo and re-sync its `.toml` copies |
| `:plum-list-themes` | Show installed theme repos, the theme names each provides, and any unmanaged `.toml` in `[plum-themes]` |
| `:plum-remove-theme <user/repo>` | Remove a theme repo's `.toml` copies and its clone. Tab-completes installed slugs |

`plum-ensure-grammars` installs the grammars of any of the given languages that are not yet compiled, and raises before installing anything if a name has no bundled grammar.
It is a plain editor command, not a `:` command, and takes a list argument, so it belongs
in `init.scm` (`(call! "plum-ensure-grammars" '("rust" "json"))`), not at the command
prompt.

## How it works

PLUM has three subsystems and a shared module. Each subsystem is its own manifest entry,
so a `:plum-list-plugins` never reads the grammar pin or registers the theme completion
source:

- **`lib/plugins.scm`** (required by `plugin.scm`): third-party plugin install, update and
  cleanup.
- **`grammars.scm`** (`#:entry`): the tree-sitter grammar install pipeline. It builds on
  `bundled-language-grammar`, `bundled-grammar-languages` and the path helpers core provides
  (see [Grammar sources and the Helix pin](#grammar-sources-and-the-helix-pin)).
- **`themes.scm`** (`#:entry`): third-party theme install, update, list and remove (see
  [Theme install](#theme-install)).
- **`lib/helpers.scm`**: `plum/require-stdlib!` (the `core:stdlib` check each entry runs),
  `plum/clone-github!` and `plum/git-pull!` (the one GitHub clone URL shape
  and its update counterpart, shared by every install command), `plum/batch-run!`, and
  `plum/two-level-repos` (the `<root>/<user>/<repo>/`
  walk behind plugin and theme-repo discovery). Directory listing, filesystem cleanup, list
  search and path-segment validation come from `core:stdlib` through `call!`.

### Path safety

PLUM validates with `stdlib/safe-path-segment?` every name that reaches `path-join` or a
subprocess argument without coming from a fixed catalog: either half of a user-typed
GitHub `user/repo` slug, and a dependency name parsed out of downloaded content. The
rejected set, the Windows reason for rejecting `:`, and the `(eq? #t …)` call-site rule are
in [`core:stdlib`'s README](../stdlib/README.md#path-safety).

### Plugin discovery

Declared plugins live in `init.scm`. Installed plugins are found by walking
`<data>/plugins/<user>/<repo>/` and checking each leaf directory for a `plugin.scm`.
"Missing" and "orphan" are set differences between the declared list and that walk, with
`core:*` plugins excluded since they are bundled. Nothing is cached, so
`:plum-list-plugins` reflects the current disk state.

### Grammar sources and the Helix pin

Grammars are keyed by language name: a compiled grammar is `<data>/grammars/<language>.<ext>`,
and every command takes a language name. A language's grammar source (repo URL, pinned
revision, tree-sitter symbol, subpath) lives in its bundled data file,
`runtime/scheme/languages/<language>.scm`, read through core's `bundled-language-grammar` and
`bundled-grammar-languages` functions; the field and path helpers live in core's
`runtime/scheme/grammars.scm`. PLUM declares no copy of either. A language that borrows
another's grammar (`jsx` uses javascript's) installs its own compiled copy under its own name.

Highlighting and structural-text-object queries (`highlights.scm`, `injections.scm`,
`textobjects.scm`) are not authored in HUME. They are fetched from the Helix project's
`runtime/queries/` at a pinned commit (`runtime/scheme/helix-pin.scm`, read once when PLUM
loads), so HUME uses Helix's query files without vendoring them.

### Query fetching

`plum/fetch-raw-query` downloads one query file with `curl` into a temp file under the
grammar sources directory, reads it, and deletes the temp file. A failed read still deletes
the temp file and then raises a fresh error; the native read error is not re-raised.

A query file can declare `; inherits: dep,dep,...` instead of writing out its own
patterns. The directive names other query sources whose patterns are spliced in; the
JS-family grammars `js`, `jsx`, `ts` and `tsx` share most of their patterns this way.
tree-sitter has no such notion, so `plum/resolve-query` resolves the chain itself: it
fetches each named dependency's copy of the same file recursively and splices the results
together before anything is written to disk. It does not deduplicate, the same as Helix's
own resolver. At the pinned Helix commit the JS/TS family is a flat one-level star (`tsx`'s
bases `ecma`, `_typescript` and `_jsx` are leaves), so a grammar reached by two `inherits`
paths does not occur; one would be spliced twice, and tree-sitter accepts duplicate
patterns.

`plum/try-fetch-query!` catches a failed fetch of an optional query, logs it at `'trace`, and
continues. Most grammars lack some optional files, and the absence means that feature is off
for the grammar, not a broken install. A missing `highlights.scm` is not tolerated and aborts
the install.

### Grammar dependencies

A grammar can have injection dependencies. Markdown's `(inline)` injection resolves only
if the `markdown.inline` grammar is also compiled and attached. `plum/install-grammar-deps!`
installs the dependencies declared in `*plum-grammar-deps*` before the grammar itself, so
`:plum-install-grammar` on a Markdown buffer also installs `markdown.inline`.

### Grammar install pipeline

`:plum-install-grammar` always installs from a clean slate, which also repairs a grammar
left in a failed state (a source tree cloned but never compiled):

1. Install any not-yet-compiled dependency grammars (see
   [Grammar dependencies](#grammar-dependencies)).
2. Purge any existing source tree.
3. Blobless clone (no file-history blobs) at the pinned revision, then force-checkout that
   revision.
4. Download the highlights query, resolving any `; inherits:` chain.
5. Compile with tree-sitter build into a shared library, after a status line, since the C
   compiler prints nothing and a slow grammar would look hung.
6. Download each optional Helix query the editor can compile (the kinds
   `grammar-optional-queries` lists), if any.
7. Register the grammar for its language in this session with
   `register-grammar-from-sources!`.

### Startup registration is core's job, and passive

`register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor startup,
whether or not PLUM is declared, and registers every already-compiled grammar in
`<data>/grammars/`. It lists that directory rather than asking every language for its
bundled grammar, so a setup with no grammars installed pays one `path-exists?`. It starts no
subprocess and uses no network. An orphan is skipped. A compiled grammar of a language with
a bundled grammar but no `highlights.scm` on disk logs a `'warn` pointing at
`:plum-install-grammar` and is skipped. A language whose grammar is not compiled stays
without one until `:plum-install-grammar` runs or `init.scm` calls `plum-ensure-grammars`;
nothing installs at startup, since a first run that compiled every bundled grammar would
stall before the editor is usable.

### Theme install

A theme repo is a GitHub `user/repo` with a `themes/*.toml` directory at its root (for
example [everforest.hume](https://github.com/cvlmtg/everforest.hume)).
`:plum-install-theme <user/repo>` clones it and copies that directory's `.toml` files flat
into `<data>/themes/`. That is the search tier `hume-editor`'s theme loader and
`:theme <Tab>` completer read; both glob `*.toml` files there without recursing.
`:theme <name>` picks up an installed theme with no `:reload-config`, since theme lookup
reads the filesystem at load time and the completer re-scans on every `Tab`.

#### Slug validation

A slug not shaped like `user/repo` is a usage mistake and logs `'info`, which reaches the
status line but not `:messages`. A slug of the right shape with a segment
`stdlib/safe-path-segment?` rejects would otherwise reach `path-join` and `git clone`, so
it raises and stays in `:messages`.

#### Layout and discovery

Each clone is kept at `<data>/themes/sources/<user>/<repo>/`, the analog of
`<data>/grammars/sources/<name>/`. A `sources/` directory has no extension, so it never
matches the `*.toml` glob, the same way a grammar's source tree is invisible to the
compiled-grammar scan. The clone is the record of which repo a given
`<data>/themes/<name>.toml` came from, so `:plum-update-themes`, `:plum-list-themes` and
`:plum-remove-theme` need no state file.

Theme repos are discovered by walking `<data>/themes/sources/<user>/<repo>/` for a `.git`
marker rather than a `themes/` directory. A repo stays discoverable and removable after
upstream drops its `themes/` directory and `:plum-update-themes` starts failing its sync.

#### Name collisions

Two installed repos that provide the same theme name write the same
`<data>/themes/<name>.toml`. The sync step logs a `'warn` naming the other repos whenever
it is about to write a name another installed repo also provides.

#### Reinstall

`:plum-install-theme` purges any existing clone for the slug before cloning, the same
clean-slate rule as `:plum-install-grammar`, which also repairs a broken install. If the
clone succeeds but the repo has no `themes/*.toml`, the sync step raises and the clone
stays on disk until the next install of that slug purges it.

### Output model

Every command that reaches the network (`plum-install-plugins`, `plum-update-plugins`,
`plum-install-grammar`, `plum-ensure-grammars`, `plum-install-theme`,
`plum-update-themes`) is `#:inline-output`: it leaves the alt-screen on its first write,
so `git`'s progress prints as it happens, and returns to the editor on a keypress once
the run finishes.

`plum/batch-run!` paints each item's progress line with `displayln`, because `log!`
buffers until the command returns and then collapses into one status-line slot, which would
lose every per-item line but the last while the batch runs. `displayln` is gated shut for a
command that is not `#:inline-output` (`plum-cleanup-plugins`, `plum-cleanup-grammars`);
those finish at once and keep their summary `log!` line.

The list commands (`plum-list-plugins`, `plum-list-grammars`, `plum-list-themes`) build
their report with `core:stdlib`'s [report commands](../stdlib/README.md#reports), in a
read-only buffer named after the command (`[plum-plugins]`, `[plum-grammars]`,
`[plum-themes]`).
`:plum-list-grammars` leaves out the languages with no compiled grammar: that list is
nearly the whole bundled catalog, which `:plum-install-grammar`'s Tab completion already
offers.

## Design decisions

- **Startup registration lives in core, not PLUM.** Highlighting must survive PLUM being
  absent from `init.scm`.
- **Theme repos are found by `.git`, not `themes/`.** See
  [Layout and discovery](#layout-and-discovery).
- **Optional query fetches are best-effort.** See
  [Query fetching](#query-fetching).
- **Install commands start from a clean slate.** Purging the existing tree first makes a
  reinstall the repair path for grammars and themes.

## Known limitations

- **Foreign-platform compiled files are invisible.** `installed-grammars`, and so
  `plum/orphan-grammars` and `:plum-cleanup-grammars`, see only files with this platform's
  shared-library extension. A `.so` left in a `<data>/grammars/` shared with a Linux
  setup is invisible to registration and cleanup on macOS, so it is neither registered nor
  reported as an orphan. It is inert on the platform it does not match and can be removed
  by hand.
- **Removing the active theme does not unload it.** `:plum-remove-theme` deletes files on
  disk only; the theme stays loaded until the next `:theme <name>` or a restart.
