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
| `plum/staging-dir` | `<data>/plum-staging/`, where every install builds its new copy |
| `plum/staged-install!` | The stage-then-replace step every install shares: fills a fresh stage directory, then moves it into place with `replace-dir!`, deleting the stage on failure |
| `plum/github-url` | The one GitHub URL shape |
| `plum/install-repo!` | The plugin and theme install (see [`repos.md`](repos.md#pinned-plugins)) |
| `plum/git-sync!` | The checkout behind both install and update (see [`repos.md`](repos.md#pinned-plugins)) |
| `plum/batch-run!` | Runs a command's items one by one, collecting failures rather than aborting, then logs a summary and each failure (see [Output model](#output-model)) |
| `plum/two-level-repos` | The `<root>/<user>/<repo>/` walk behind plugin and theme-repo discovery |

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

`plum/batch-run!` paints each item's progress line with the `#:progress` reporter its
caller passes, `print!` for the commands above, because `log!` buffers until the command
returns and then collapses into one status-line slot, which would lose every per-item line
but the last while the batch runs. `plum-install-grammar` and `plum-install-theme` print
their own "installing" and "installed" lines with `print!` for the same reason; a grammar
download and unpack print nothing else, so without them its screen stays empty. `print!`
writes only inside an `#:inline-output` command, so `plum-cleanup-plugins` and
`plum-cleanup-grammars`, which finish at once, pass none and keep their summary `log!` line.

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
