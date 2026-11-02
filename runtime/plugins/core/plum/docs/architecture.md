# core:plum — Architecture

PLUM has three subsystems and a shared module. Each subsystem is its own manifest entry,
so a `:plum-list-plugins` never reads the grammar pin or registers the theme completion
source.

## File layout

| File | Owns |
|---|---|
| `plugin.scm` | Entry point of the plugin commands; requires `lib/plugins.scm` |
| `lib/plugins.scm` | Third-party plugin install, update, list and cleanup (see [`repos.md`](repos.md)) |
| `grammars.scm` | `#:entry` of the grammar commands: the tree-sitter grammar install pipeline, built on `bundled-languages` and the grammar layout functions core provides (see [`grammars.md`](grammars.md)) |
| `themes.scm` | `#:entry` of the theme commands: third-party theme install, update, list and remove (see [`repos.md`](repos.md#theme-install)) |
| `lib/helpers.scm` | The shared module (see below) |
| `data/treesitter-hume-pin.scm` | The grammar pin (see [`grammars.md`](grammars.md#grammar-pin)) |

## Shared module

`lib/helpers.scm` provides:

| Function | Does |
|---|---|
| `plum/msg` | Every line PLUM logs or prints, prefixed `PLUM: `; a raised error carries no prefix and gets this one where it is reported, with `error-text` |
| `plum/report-failure!` | Logs an error a command raises as `PLUM: <action> failed: <message>` |
| `plum/staging-dir` | `<data>/plum-staging/`, where every install builds its new copy |
| `plum/staged-install!` | The stage-then-replace step every install shares: fills a fresh stage directory, then moves it into place with `replace-dir!`, deleting the stage on failure |
| `plum/github-url` | The one GitHub URL shape |
| `plum/install-repo!` | The plugin and theme install (see [`repos.md`](repos.md#pinned-plugins)) |
| `plum/git-sync!` | The checkout behind both install and update (see [`repos.md`](repos.md#pinned-plugins)) |
| `plum/batch-run!` | Runs a command's items one by one, collecting failures rather than aborting (see below) |
| `plum/two-level-repos` | The `<root>/<user>/<repo>/` walk behind plugin and theme-repo discovery |

`plum/batch-run!` returns how many items succeeded. It then logs a summary built with its
`verb` argument and each failure, or the caller's `#:none` message when there are no items.
Its keyword arguments:

- `#:skip`, given an item and the items that failed so far, returns why to skip it, or `#f`.
  A skipped item counts as failed.
- `#:progress` names the line printed as each item starts (see [Output model](#output-model)).

Directory listing, filesystem cleanup, list search, downloads, digests and archive
extraction come from `core:stdlib` through `call!`.

## Path safety

PLUM validates with `safe-path-segment?` every name that reaches `path-join` or a
subprocess argument without coming from a fixed catalog: either half of a user-typed
GitHub `user/repo` slug. A grammar name is always one of `bundled-languages`, so it needs
no check.

## Output model

Every command that reaches the network (`plum-install-plugins`, `plum-update-plugins`,
`plum-install-grammar`, `plum/ensure-grammars`, `plum-install-theme`,
`plum-update-themes`) is `#:inline-output`: it leaves the alt-screen on its first write,
so `git`'s progress and the `curl` and `tar` errors print as they happen, and returns to
the editor on a keypress once the run finishes.

While one of these commands runs, the user is watching the terminal screen it switched to,
not HUME's. `log!` reports to HUME's statusline and `:messages`, which the user sees only
once back in the editor, so a line meant for the run's own screen goes through `print!`:

- `plum/batch-run!` prints `PLUM: <progress> <item>` as each item starts, when its caller
  passes `#:progress`, as the commands above do. It logs its summary and each failure for the
  user to read back in the editor.
- A grammar download and unpack print nothing else, so fetching the release index prints its
  own line, and `plum-install-grammar` prints an "installed" line only when its whole plan
  installed.
- `plum-install-theme` prints its own "installing" and "installed" lines.

`print!` writes only inside an `#:inline-output` command. `plum-cleanup-plugins` and
`plum-cleanup-grammars` finish at once and never leave the editor, so they pass no
`#:progress` and report only through their summary `log!` line.

The list commands (`plum-list-plugins`, `plum-list-grammars`, `plum-list-themes`) build
their report with `core:stdlib`'s [report commands](../../stdlib/README.md#reports), in a
read-only buffer named after the command (`[plum-plugins]`, `[plum-grammars]`,
`[plum-themes]`). `:plum-list-grammars` leaves out the languages with no installed
grammar: that list is nearly the whole bundled catalog, which `:plum-install-grammar`'s
Tab completion already offers.

## Design decisions

- **No state files.** Every "what's installed" question is answered by walking disk when it
  is asked: the plugin and theme clones, and the grammar install folders, are their own
  record.
- **Install commands build the new copy beside the old and swap it in last.** A reinstall
  is the repair path for grammars and themes, and a failed one leaves the installed
  copy as it was.
