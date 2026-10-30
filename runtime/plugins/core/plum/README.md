# core:plum

**PLUM**: the HUME **PLU**gin **M**anager. It installs and updates third-party Steel
plugins and themes from GitHub, and installs the prebuilt tree-sitter grammars that power
syntax highlighting.

PLUM keeps no state files. Every "what's installed" question is answered by walking disk
when it is asked (see [Plugin discovery](#plugin-discovery)).

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

- **Depends on:** `core:stdlib`, when its commands run.
- **Activates on:** the first `:plum-*` command typed, or the first call to
  `plum/ensure-grammars`. Its `manifest.scm` has three entries, each loading on its own
  commands: `plugin.scm` (the four plugin commands), `grammars.scm` (`#:entry`;
  `plum/ensure-grammars` in `#:commands`, the grammar commands in `#:typed-commands`) and
  `themes.scm` (`#:entry`; the theme commands).
- **Config:** `'grammar-source`, a folder of grammar archives to install from before the
  pinned release (see [Grammar source](#grammar-source)).
- **Not required for what is already installed.** PLUM is a plugin like any other, so
  leaving it out removes only the management commands below. Installed plugins, grammars
  and themes keep working, including syntax highlighting: registering installed
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
| `:plum-update-plugins` | Check every installed third-party plugin out at its `#:ref`, or its default branch's latest commit |
| `:plum-list-plugins` | Show the declared, installed, orphan and missing plugin lists in `[plum-plugins]` |
| `:plum-install-grammar [lang]` | Install or repair a grammar from the grammar source, replacing the installed one only once the new one has loaded. Defaults to the current buffer's language. Tab-completes the language names |
| `:plum-list-grammars` | Show the installed grammars, what else `<data>/grammars/` holds, and what stopped installs left in `<data>/plum-staging/grammars/`, in `[plum-grammars]` |
| `:plum-cleanup-grammars` | Delete everything in `<data>/grammars/` but the current install of each language HUME knows, and everything in `<data>/plum-staging/grammars/` |
| `:plum-install-theme <user/repo>` | Install or reinstall a theme repo's `themes/*.toml` from a GitHub slug |
| `:plum-update-themes` | Check every installed theme repo out at its default branch's latest commit and re-sync its `.toml` copies |
| `:plum-list-themes` | Show installed theme repos, the theme names each provides, and any unmanaged `.toml` in `[plum-themes]` |
| `:plum-remove-theme <user/repo>` | Remove a theme repo's `.toml` copies and its clone. Tab-completes installed slugs |

`plum/ensure-grammars` installs the grammars of any of the given languages that are not yet
installed. It raises before installing anything if a name is not a language, or if no
source serves one of the grammars it would install, dependencies included.
It is a plain editor command, not a `:` command, and takes a list argument, so it belongs
in `init.scm` (`(call! "plum/ensure-grammars" '("rust" "json"))`), not at the command
prompt.

`plum/grammar-leftovers` is what the two grammar list and cleanup commands share: every entry
of `<data>/grammars/` that is not an installed grammar of a language HUME knows, and every
entry of such a grammar's folder but its current install. That is an orphan (a grammar
installed under a name that is not a language), an older install a reinstall could not delete,
an install folder that was never checked (its load check failed and it could not be deleted,
or its install was stopped first), and any other entry, such as `sources/` clones, `<name>.<ext>` libraries at the top level or in
a language's folder, and the `.replaced` backups an interrupted install leaves. Every
such entry is listed by its path under `<data>/grammars/` and deleted whole.
`plum/staging-leftovers` lists every entry of `<data>/plum-staging/grammars/`, which the two
commands show and clear too: stage directories, archives and the index download. Grammar
commands run one at a time, so nothing there belongs to an install in progress. On Windows, an
install folder whose library this session loaded cannot be deleted until HUME exits, so cleanup
reports it as a failure and clears it the next time.

## How it works

PLUM has three subsystems and a shared module. Each subsystem is its own manifest entry,
so a `:plum-list-plugins` never reads the grammar pin or registers the theme completion
source:

- **`lib/plugins.scm`** (required by `plugin.scm`): third-party plugin install, update and
  cleanup.
- **`grammars.scm`** (`#:entry`): the tree-sitter grammar install pipeline. It builds on
  `bundled-languages` and the layout functions core provides (see
  [Grammar source](#grammar-source)).
- **`themes.scm`** (`#:entry`): third-party theme install, update, list and remove (see
  [Theme install](#theme-install)).
- **`lib/helpers.scm`**: `plum/staged-install!` (the stage-then-replace step every
  install shares), `plum/github-url`, `plum/install-repo!` and `plum/git-sync!` (the one
  GitHub URL shape, the plugin and theme install, and the checkout behind both install and
  update; see [Pinned plugins](#pinned-plugins)), `plum/batch-run!`, and
  `plum/two-level-repos` (the `<root>/<user>/<repo>/`
  walk behind plugin and theme-repo discovery). Directory listing, filesystem cleanup, list
  search come from `core:stdlib` through `call!`.

### Path safety

PLUM validates with `safe-path-segment?` every name that reaches `path-join` or a
subprocess argument without coming from a fixed catalog: either half of a user-typed
GitHub `user/repo` slug. A grammar name is always one of `bundled-languages`, so it needs
no check.

### Plugin discovery

Declared plugins live in `init.scm`. Installed plugins are found by walking
`<data>/plugins/<user>/<repo>/` and checking each leaf directory for a `plugin.scm`.
"Missing" and "orphan" are set differences between the declared list and that walk, with
`core:*` plugins excluded since they are bundled. Nothing is cached, so
`:plum-list-plugins` reflects the current disk state.

### Pinned plugins

`(load-plugin! "user/repo" #:ref "v1.2.0")` pins a plugin to a tag, a branch or a
commit id, full or abbreviated; `(declared-plugin-ref name)` reads the pin back.

`plum/git-sync!` puts a clone on a ref. Without a pin it fetches `HEAD`, the remote's
default branch, and for a full 40-digit commit id it fetches
that commit alone, whether or not a branch or tag reaches it. Any other pin is a tag, a
branch or an abbreviated commit id, and `git fetch` cannot resolve an abbreviated id on
the remote, so it fetches every branch and tag (`git fetch --force --prune --tags
origin`) and resolves the ref locally, taking the first of `refs/tags/<ref>`,
`refs/remotes/origin/<ref>` and, for 4 or more hex digits, the commit id itself that
`git rev-parse --verify` accepts. A ref that resolves to nothing (missing upstream, an
ambiguous short id, or a short id of a commit no branch or tag reaches, which the fetch
never brings in) raises an error naming it. A tag or branch named with 40 hex digits is
read as a commit id. Either way it ends with `git checkout --detach`, which moves a
clone between any two states: onto a tag, from one tag to another, from a tag to a
branch, and off a tag once the pin is removed, where `git pull` would refuse a detached
HEAD. So every clone is on a detached HEAD, and a local edit that conflicts with the
new checkout fails the update for that plugin. An update also leaves behind commits
made in the clone and any branch checked out there by hand.

`plum/install-repo!`, behind every plugin and theme install, runs `plum/git-sync!` on a fresh
repo: `git init` in `<data>/plum-staging/<parent>/<name>`, `git remote add origin <url>`,
origin made a promisor remote with `partialclonefilter blob:none` (a blobless clone,
which downloads only the files of the commits it checks out), `plum/git-sync!`, then
the caller's `#:build` on the staging clone, if any. Only then does `plum/staged-install!`
move the new clone into the old one's place with `replace-dir!`, in one step the script budget cannot stop
half way. A failed install deletes its staging
clone and leaves the old clone untouched, then raises the failure's own error.
`:plum-install-plugins` installs only missing plugins, so a changed pin reaches an
installed plugin through `:plum-update-plugins`.
`load-plugin!` raises for a ref `git check-ref-format` would refuse, for one starting
with `-`, which no tag or branch name can, and for a ref on a `core:` plugin.

### Grammar source

Grammars are prebuilt by [treesitter.hume](https://github.com/cvlmtg/treesitter.hume), which
publishes one bundle per platform in each release: `grammars-<platform>.tar`, an uncompressed
tar of one `<language>.tar.gz` per language, and `grammars-<platform>.index`, whose first line
is `treesitter.hume <tag> <platform>` and whose other lines are `<language> <offset> <size>
<sha256>`, the byte range of that language's archive in the `.tar`. `<platform>` is
`grammar-platform`'s `<os>-<arch>`.

Grammars come from two sources, checked per language in this order:

1. The folder `#:config 'grammar-source` names, if any: a `hume-grammar-build build --out
   <dir>` folder, which holds one `<language>.tar.gz` per grammar it built, the same archive
   a release bundles. It has no index: a language is in the folder when its archive is, and
   the archive's own manifest is checked as any other, platform included (step 4 of the
   [Grammar install pipeline](#grammar-install-pipeline)).
   It must be an absolute path (`stdlib/path-absolute?`): `grammars.scm` loads on the first
   grammar command, so a relative one would depend on where `:cd` had moved by then, and
   `grammars.scm` fails to load naming the value instead. It is checked at the start of
   every install command, and fails the command rather than falling through to the release
   when it is a wrong path rather than an empty one: when it is not a directory, when it
   holds no `<language>.tar.gz` for any language HUME knows (such as the parent of the
   `--out` folder), or when it is inside `<data>/plum-staging/` or `<data>/grammars/`, whose
   contents installs and `:plum-cleanup-grammars` delete (`first-path-outside`, so a symlink
   into either counts as inside).
2. The release `data/treesitter-hume-pin.scm` names, read once when `grammars.scm` loads.
   Its index is downloaded with `curl` into `<data>/plum-staging/grammars/`, and deleted once
   read, only when the folder holds no archive for a language asked for, and at most once per
   command. When a folder is configured, a failure to read it names the folder too, since
   the language was looked for there first.

`plum/source-lookup` makes that choice: a function from a language to the source that
serves it, or `#f` when neither does. A source calls a procedure on the archive to unpack:
the folder's own `<language>.tar.gz`, read where it is and never deleted, or
`<data>/plum-staging/grammars/<language>.tar.gz`, which one ranged `stdlib/download!` of the
release's `.tar` writes and whose digest must match. The release source deletes that file
once the procedure returns or fails, so only it owns a staging archive. One install path
serves both sources from there. `plum/served-plan` resolves every language of a command's
plan once, before anything installs, and each install uses the source its plan resolved, so
a folder that changes during a batch cannot move a language to the other source. A language
the folder holds comes from the folder even when its archive there turns out to be broken or
gone: the install fails rather than quietly downloading something else.

### Grammar pin

`data/treesitter-hume-pin.scm` holds one string, a treesitter.hume release tag, and is HUME's
only grammar-related pin. That release's `grammars.toml` also names the Helix commit HUME's
language data is generated from, so the data and the installed queries agree. Bumping it:
release treesitter.hume, change the tag here, then run the sync scripts (`scripts/README.md`).

### Grammar dependencies

A grammar can have injection dependencies. Markdown's `(inline)` injection resolves only
if the `markdown.inline` grammar is also installed. Both install commands first install the
not-yet-installed dependencies declared in `*plum-grammar-deps*`, so `:plum-install-grammar`
on a Markdown buffer also installs `markdown.inline`. `plum/served-plan` turns the names a
command is given into its install plan: each name after the dependencies it needs that are
not installed, each once.

### Grammar install pipeline

`:plum-install-grammar` installs the grammar, its dependencies first, and
`plum/ensure-grammars` installs a batch. Both first check that every grammar of the plan has a
source, so a grammar no source serves installs nothing; each reads the release index at most
once.

Each install of a grammar gets its own folder, `<data>/grammars/<language>/<install>/`, and the
newest one marked `checked` is the grammar in use (see core's `runtime/scheme/README.md`). An install never
renames or deletes a folder whose library may be loaded: a loaded library stays loaded until
HUME exits, Unix's loader hands back the loaded library for a path it has opened before, and
Windows cannot move or delete a folder whose library is loaded. A new folder per install means
a new library path, so a reinstall takes effect in the session that runs it, and the installed
grammar is replaced only once the new one has loaded:

1. Find the source that serves the language (see [Grammar source](#grammar-source)). The
   release index's header must name this platform. A language neither source holds raises
   `no <language> grammar in <folder> or treesitter.hume <tag> for <platform>`, without the
   folder part when none is configured.
2. From the release, write the language's archive to
   `<data>/plum-staging/grammars/<language>.tar.gz`, and check it against the index line's
   SHA-256 with `stdlib/verify-sha256!`. An archive in the folder is used where it is.
3. Extract the archive into `<data>/plum-staging/grammars/<language>/` with
   `stdlib/extract-archive!`.
4. Check its `manifest.scm`: `name` is the language, `platform` is this one, and the library
   and `queries/highlights.scm` are there. A `tag` other than the pin is a `'warning`, not a
   failure, so a folder built by another release of the tool still installs. The library's
   ABI is checked once, by loading it in step 6.
5. Move the stage directory to `<data>/grammars/<language>/<install>/` with
   `replace-dir!`. Nothing has loaded from it yet, so it can move on every platform.
   `<install>` is the current time in milliseconds, or one past the newest install when that is
   later, so it sorts after every install before it.
6. Load it there with core's `check-grammar!`, which opens the library, checks its ABI and
   compiles its queries without registering anything, so the check is the same from a
   command, from `init.scm` (where registration only queues) and whatever registered the
   language's grammar first. Once it loads, `mark-grammar-checked!` writes the folder's
   `checked` marker, which makes it the language's current install.
7. Register the new install for this session with core's `register-installed-grammar!`. It
   opens the library at the path the check opened, so the library loads once.
8. Delete the language's older installs, checked or not. The new install is already in place, so a delete
   that fails is reported and the install goes on: as an error on Linux and macOS, where it
   is a real failure, and as a warning on Windows, which refuses to delete a library HUME has
   loaded and gives the same access-denied error for both. The Windows warning says to
   restart HUME and run `:plum-cleanup-grammars`.

A failure in steps 1 to 5 deletes the staging files, and a failure in step 6 deletes the new
install folder, and the language's folder when nothing else is left in it. Either way the
failure's own error is raised, with the installed grammar as it was. On Windows, an install
folder whose check opened its library cannot be deleted; it is reported as in step 8, and it
has no marker, so it is never the grammar. A script budget that stops the install at any point
leaves the old or the new install as the newest checked one, so the language's grammar is
never one that failed its check. Older installs are deleted only once the new one is
registered, so the session never names a library that is gone. Install folders left behind
are leftovers (see [Commands](#commands)).
The archives carry Helix's query files with their `; inherits:` directives already resolved,
so PLUM installs them unchanged.

### Startup registration is core's job, and passive

`%register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor startup,
whether or not PLUM is declared, and registers the current install of every grammar in
`<data>/grammars/`. Registering opens nothing: each grammar loads the first time a buffer or
an embedded block needs it. It lists that directory rather than asking every language, so a
setup with no grammars installed pays one `is-dir?`. It starts no subprocess and uses no
network. An orphan is skipped. An installed grammar of a language with no `highlights.scm` or
no readable manifest logs a `'warning` pointing at `:plum-install-grammar` and is skipped. A
language whose grammar is not installed stays without one until `:plum-install-grammar` runs
or `init.scm` calls `plum/ensure-grammars`; nothing installs at startup.

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
`safe-path-segment?` rejects would otherwise reach `path-join` and `git`, so
it raises and stays in `:messages`.

#### Layout and discovery

Each clone is kept at `<data>/themes/sources/<user>/<repo>/`. A `sources/` directory has
no extension, so it never matches the `*.toml` glob. The clone is the record of which repo a given
`<data>/themes/<name>.toml` came from, so `:plum-update-themes`, `:plum-list-themes` and
`:plum-remove-theme` need no state file.

Theme repos are discovered by walking `<data>/themes/sources/<user>/<repo>/` for a `.git`
marker rather than a `themes/` directory. A repo stays discoverable and removable after
upstream drops its `themes/` directory and `:plum-update-themes` starts failing its sync.

#### Name collisions

Two installed repos that provide the same theme name write the same
`<data>/themes/<name>.toml`. The sync step logs a `'warning` naming the other repos whenever
it is about to write a name another installed repo also provides.

#### Reinstall

`:plum-install-theme` always installs from a fresh clone through `plum/install-repo!`,
which also repairs a broken install. Its `#:build` raises
when the new clone has no `themes/*.toml`, so a repo without themes fails before it
replaces an installed clone, and the installed copies stay as they were.

### Output model

Every command that reaches the network (`plum-install-plugins`, `plum-update-plugins`,
`plum-install-grammar`, `plum/ensure-grammars`, `plum-install-theme`,
`plum-update-themes`) is `#:inline-output`: it leaves the alt-screen on its first write,
so `git`'s progress and the `curl` and `tar` errors print as they happen, and returns to the editor on a keypress once
the run finishes.

`plum/batch-run!` paints each item's progress line with the `#:progress` reporter its
caller passes, `print!` for the commands above, because `log!` buffers until the command
returns and then collapses into one status-line slot, which would lose every per-item line
but the last while the batch runs. `print!` writes only inside an `#:inline-output`
command, so `plum-cleanup-plugins` and `plum-cleanup-grammars`, which finish at once, pass
none and keep their summary `log!` line.

The list commands (`plum-list-plugins`, `plum-list-grammars`, `plum-list-themes`) build
their report with `core:stdlib`'s [report commands](../stdlib/README.md#reports), in a
read-only buffer named after the command (`[plum-plugins]`, `[plum-grammars]`,
`[plum-themes]`).
`:plum-list-grammars` leaves out the languages with no installed grammar: that list is
nearly the whole bundled catalog, which `:plum-install-grammar`'s Tab completion already
offers.

## Design decisions

- **Startup registration lives in core, not PLUM.** Highlighting must survive PLUM being
  absent from `init.scm`.
- **Theme repos are found by `.git`, not `themes/`.** See
  [Layout and discovery](#layout-and-discovery).
- **Grammars are downloaded, never compiled.** A user needs `curl` and `tar`, not a C
  toolchain, and every platform gets the same build.
- **Install commands build the new copy beside the old and swap it in last.** A reinstall
  is the repair path for grammars and themes, and a failed one leaves the installed copy as
  it was.

## Known limitations

- **One platform per data directory.** `<data>/grammars/<language>/` holds one platform's
  build. A grammar directory holding another platform's library, as in a data directory
  shared with a Linux setup, is invisible to registration, listing and cleanup on macOS;
  installing the grammar there replaces it with this platform's build.
- **Removing the active theme does not unload it.** `:plum-remove-theme` deletes files on
  disk only; the theme stays loaded until the next `:theme <name>` or a restart.
