# core:plum — Grammars

`grammars.scm` installs prebuilt tree-sitter grammars into `<data>/grammars/`, in the
layout core's `runtime/scheme/grammars.scm` defines and reads (see
[`runtime/scheme/README.md`](../../../../scheme/README.md#grammarsscm)).

## Grammar source

Grammars are prebuilt by [treesitter.hume](https://github.com/cvlmtg/treesitter.hume), which
publishes one bundle per platform in each release. `<platform>` is `grammar-platform`'s
`<os>-<arch>`.

- `grammars-<platform>.tar`: an uncompressed tar of one `<language>.tar.gz` per language.
- `grammars-<platform>.index`: a first line `treesitter.hume <tag> <platform>`, then one
  `<language> <offset> <size> <sha256>` line per language, giving the byte range of that
  language's archive in the `.tar`.

Grammars come from two sources, checked per language in this order:

1. **The folder `#:config 'grammar-source` names**, if any: a `hume-grammar-build build --out
   <dir>` folder, holding one `<language>.tar.gz` per grammar it built, the same archive a
   release bundles. It has no index: a language is in the folder when its archive is, and the
   archive's own manifest is checked as any other, platform included (step 4 of the
   [Grammar install pipeline](#grammar-install-pipeline)).
2. **The release `data/treesitter-hume-pin.scm` names**, read once when `grammars.scm` loads.
   Its index is downloaded with `curl` into `<data>/plum-staging/grammars/`, and deleted once
   read, only when the folder holds no archive for a language asked for, and at most once per
   command. When a folder is configured, a failure to read the index names the folder too,
   since the language was looked for there first.

The folder must be an absolute path (`stdlib/path-absolute?`). `grammars.scm` loads on the
first grammar command, so a relative path would depend on where `:cd` had moved by then;
`grammars.scm` fails to load instead, naming the value.

An install command checks the folder when it builds its plan (`plum/served-plan`), so
`plum/ensure-grammars` does not check it when every grammar it is given is installed. The
check fails the command, rather than falling through to the release, when the folder looks
like a wrong path rather than one that lacks only the languages asked for:

- it is not a directory;
- it holds no `<language>.tar.gz` for any language HUME knows, such as the parent of the
  `--out` folder;
- it is inside `<data>/plum-staging/` or `<data>/grammars/`, whose contents installs and
  `:plum-cleanup-grammars` delete. The test is `first-path-outside`, so a symlink into either
  counts as inside.

`plum/source-lookup` makes the choice: it returns a function from a language to the source
that serves it, or `#f` when neither does. A source calls a procedure on the archive to
unpack, and one install path serves both sources from there:

| Source | Archive the procedure gets | Deleted |
|---|---|---|
| Folder | its own `<language>.tar.gz`, read in place | never |
| Release | `<data>/plum-staging/grammars/<language>.tar.gz`, written by one ranged `stdlib/download!` of the release's `.tar`; its digest must match | by the release source, once the procedure returns or fails |

`plum/served-plan` resolves every language of a command's plan once, before anything
installs, and each install uses the source its plan resolved. A folder that changes during a
batch cannot move a language to the other source. A language the folder holds comes from the
folder even when its archive there turns out to be broken or gone: the install fails rather
than downloading something else.

## Grammar pin

`data/treesitter-hume-pin.scm` holds one string, a treesitter.hume release tag, and is HUME's
only grammar-related pin. HUME's language data is generated from that release's language
settings, so the data and the installed queries come from one release. To bump it:

1. Release treesitter.hume.
2. Change the tag here.
3. Run the sync scripts (`scripts/README.md`).

## Grammar dependencies

A grammar can have injection dependencies. Markdown's `(inline)` injection resolves only
if the `markdown.inline` grammar is also installed. `*plum-grammar-deps*` declares them, and
`plum/served-plan` turns the names a command is given into its install plan: each name after
the dependencies it needs that are not installed, each once. So `:plum-install-grammar` on a
Markdown buffer also installs `markdown.inline`.

## Grammar install pipeline

`:plum-install-grammar` installs one grammar and `plum/ensure-grammars` a batch, each from
the plan `plum/served-plan` resolves before anything installs, so a grammar no source serves
installs nothing (see [Grammar source](#grammar-source)). Both install the plan through
`plum/install-plan!`, which goes on past a grammar that fails and skips each grammar a
dependency of which failed or was skipped, so Markdown never installs without
`markdown.inline`. Each failed or skipped grammar is logged as `PLUM: <language>: <reason>`.

Each install of a grammar gets its own folder, `<data>/grammars/<language>/<install>/`, and the
newest one marked `checked` is the grammar in use. An install never renames or deletes a
folder whose library may be loaded, because:

- a loaded library stays loaded until HUME exits;
- Unix's loader hands back the loaded library for a path it has opened before;
- Windows cannot move or delete a folder whose library is loaded.

A new folder per install means a new library path, so a reinstall takes effect in the session
that runs it, and the installed grammar is replaced only once the new one has loaded.

`plum/install-one` installs one language:

1. Find the source that serves the language (see [Grammar source](#grammar-source)). The
   release index's header must name this platform. A language neither source holds raises
   `no <language> grammar in <folder> or treesitter.hume <tag> for <platform>`, without the
   folder part when none is configured.
2. From the release, write the language's archive to
   `<data>/plum-staging/grammars/<language>.tar.gz`, and check it against the index line's
   SHA-256 with `stdlib/verify-sha256!`. An archive in the folder is used where it is.
3. Extract the archive into `<data>/plum-staging/grammars/<language>/` with
   `stdlib/extract-archive!`.
4. Check its `manifest.scm`: `name` is the language, `symbol` is a non-empty string,
   `platform` is this one, and the library and `queries/highlights.scm` are there. A `tag`
   other than the pin is a `'warning`, not a failure, so a folder built by another release of
   the tool still installs. The library's ABI is checked once, by loading it in step 6.
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
8. Delete the language's older installs, checked or not. The new install is already in
   place, so a delete that fails is reported and the install goes on: as an error on Linux
   and macOS, where it is a real failure, and as a warning on Windows, which refuses to
   delete a library HUME has loaded and gives the same access-denied error for both. The
   Windows warning says to restart HUME and run `:plum-cleanup-grammars`.

When an install fails or stops:

- A failure in steps 1 to 5 deletes the staging files. A failure in step 6 deletes the new
  install folder, and the language's folder when nothing else is left in it. Either way the
  failure's own error is raised, with the installed grammar as it was.
- On Windows, an install folder whose check opened its library cannot be deleted. It is
  reported as in step 8, and it has no marker, so it is never the grammar.
- A script budget that stops the install at any point leaves the old or the new install as
  the newest checked one, so the language's grammar is never one that failed its check.
- Older installs are deleted only once the new one is registered, so the session never names
  a library that is gone.

Install folders left behind are [leftovers](#leftovers).

The archives carry treesitter.hume's query files, taken from Helix and
nvim-treesitter-context, with their `; inherits:` directives already resolved, so PLUM
installs them unchanged.

## Leftovers

`plum/grammar-leftovers` is what `:plum-list-grammars` and `:plum-cleanup-grammars` share:
every entry of `<data>/grammars/` that is not an installed grammar of a language HUME knows,
and every entry of such a grammar's folder but its current install. That covers:

- an orphan: a grammar installed under a name that is not a language;
- an older install a reinstall could not delete;
- an install folder that was never checked, because its load check failed and it could not be
  deleted, or its install was stopped first;
- any other entry, such as `sources/` clones, `<name>.<ext>` libraries at the top level or in
  a language's folder, and the `.replaced` backups an interrupted install leaves.

Each is listed by its path under `<data>/grammars/` and deleted whole.

`plum/staging-leftovers` lists every entry of `<data>/plum-staging/grammars/`, which the two
commands show and clear too: stage directories, archives and the index download. Grammar
commands run one at a time, so nothing there belongs to an install in progress. On Windows,
an install folder whose library this session loaded cannot be deleted until HUME exits, so
cleanup reports it as a failure and clears it the next time.

## Startup registration

Registering installed grammars at startup is core's job, not PLUM's.
`%register-installed-grammars!` (`runtime/scheme/grammars.scm`) runs once at editor startup,
whether or not PLUM is declared, and registers the current install of every installed
grammar of a language HUME knows.

It reads each install's `manifest.scm` for the grammar's `symbol` and loads nothing. A
grammar loads the first time a buffer or an embedded block needs it, so a missing library or
`highlights.scm` is reported then, not at startup. It starts no subprocess and uses no
network.

- An orphan is skipped without a message.
- A grammar whose manifest is missing, unreadable or has no `symbol` logs a `'warning` naming
  `:plum-install-grammar`, and the other grammars still register.
- Nothing installs at startup: a language whose grammar is not installed stays without one
  until `:plum-install-grammar` runs or `init.scm` calls `plum/ensure-grammars`.

## Design decisions

- **Startup registration lives in core, not PLUM.** Highlighting must survive PLUM being
  absent from `init.scm`.
- **Grammars are downloaded, never compiled.** A release install needs `curl`, `tar` and the
  system's SHA-256 tool (`shasum`, `sha256sum` or `certutil`), not a C toolchain, and every
  platform gets the same build.

## Known limitations

- **One platform per data directory.** Nothing that reads `<data>/grammars/` looks at which
  platform an install was built for: registration, listing and cleanup take a language's
  newest `checked` install as its grammar whatever the platform. In a data directory shared
  with another platform, a language whose newest checked install that platform made has it
  registered here, where it fails when it loads; it is listed as installed, kept by cleanup
  and skipped by `plum/ensure-grammars`. `:plum-install-grammar` replaces it with this
  platform's build.
