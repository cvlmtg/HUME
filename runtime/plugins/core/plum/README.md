# core:plum

**PLUM**: the HUME **PLU**gin **M**anager. It installs and updates third-party Steel
plugins and themes from GitHub, and installs the prebuilt tree-sitter grammars that power
syntax highlighting.

PLUM is only the management commands. Installed plugins, grammars and themes keep working
without it, highlighting included, since core registers installed grammars at startup (see
[`docs/grammars.md`](docs/grammars.md#startup-registration)). Language servers are
`core:lsp-install`'s: PLUM never touches `<data>/servers/` or the LSP catalogs.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

- **Depends on:** `core:stdlib`, when its commands run.
- **Activates on:** the first `:plum-*` command typed, or the first call to
  `plum/ensure-grammars`. Its `manifest.scm` has three entries, each loading on its own
  commands: `plugin.scm` (the plugin commands), `grammars.scm` and `themes.scm`.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-plum)
  and [Syntax Highlighting](https://cvlmtg.github.io/HUME/syntax-highlighting.html) for
  the grammar workflow.

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'grammar-source` | string, an absolute path | none | A folder of grammar archives to install from before the pinned release; `grammars.scm` fails to load on a relative path. See [`docs/grammars.md`](docs/grammars.md#grammar-source) |

## Commands

| Command | Effect |
|---|---|
| `:plum-install-plugins` | Install all plugins declared in `init.scm` that are not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins that `init.scm` does not declare |
| `:plum-update-plugins` | Check every installed third-party plugin out at its `#:ref`, or its default branch's latest commit |
| `:plum-list-plugins` | Show the declared, installed, orphan and missing plugin lists in `[plum-plugins]` |
| `:plum-install-grammar [lang]` | Install or repair a grammar, and the dependencies it lacks, replacing the installed one only once the new one has loaded. Defaults to the current buffer's language. Tab-completes language names (completion source `"plum:grammars"`) |
| `(call! "plum/ensure-grammars" '("rust" "json"))` | Install the grammars of the listed languages that are not installed, dependencies included. Raises before installing anything if a name is not a language, or if no source serves a grammar it would install. Takes a list, so it belongs in `init.scm`, not at the command prompt |
| `:plum-list-grammars` | Show the installed grammars and the [leftovers](docs/grammars.md#leftovers) in `<data>/grammars/` and `<data>/plum-staging/grammars/`, in `[plum-grammars]` |
| `:plum-cleanup-grammars` | Delete the [leftovers](docs/grammars.md#leftovers): everything in `<data>/grammars/` but the current install of each language HUME knows, and everything in `<data>/plum-staging/grammars/` |
| `:plum-install-theme <user/repo>` | Install or reinstall a theme repo's `themes/*.toml` from a GitHub slug |
| `:plum-update-themes` | Check every installed theme repo out at its default branch's latest commit and re-sync its `.toml` copies |
| `:plum-list-themes` | Show installed theme repos, the theme names each provides, and any unmanaged `.toml` in `[plum-themes]` |
| `:plum-remove-theme <user/repo>` | Remove a theme repo's `.toml` copies and its clone. Tab-completes installed slugs (completion source `"plum:themes"`) |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | The three subsystems and the shared module, path safety, the output model, cross-cutting design decisions |
| [`docs/repos.md`](docs/repos.md) | Third-party plugin and theme repos: plugin discovery, pinned plugins and the clone/sync code both share, theme install |
| [`docs/grammars.md`](docs/grammars.md) | Grammar source and pin, dependencies, the install pipeline, leftovers, startup registration |
