# core:plum

**PLUM** (the HUME **PLU**gin **M**anager) installs and updates third-party plugins and themes from GitHub, and installs the tree-sitter grammars that power syntax highlighting. Every PLUM command needs `core:stdlib`, so load it first:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

PLUM installs nothing on its own; the commands below do the work when you run them.

| Command | Effect |
|---------|--------|
| `:plum-install-plugins` | Install every plugin named in `init.scm` that is not on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins that `init.scm` does not name |
| `:plum-update-plugins` | Bring every installed third-party plugin to its pinned tag, branch or commit, or its latest version (see [Pinning a version](../plugins.md#pinning-a-version)) |
| `:plum-list-plugins` | Show named / installed / orphan / missing plugins |
| `:plum-install-grammar [lang]` | Download and install one language's grammar, the current buffer's language by default; Tab-completes language names |
| `:plum-list-grammars` | Show the installed grammars, and anything else the grammars folder holds |
| `:plum-cleanup-grammars` | Remove everything in the grammars folder except the installed grammars |
| `:plum-install-theme <user/repo>` | Install (or reinstall) a theme repo's themes |
| `:plum-update-themes` | Pull the latest version of every installed theme repo |
| `:plum-list-themes` | Show installed theme repos and the themes each provides |
| `:plum-remove-theme <user/repo>` | Remove an installed theme repo; Tab-completes installed slugs |

After `:plum-install-plugins` or `:plum-update-plugins`, run `:reload-config` to load what changed.

`plum/ensure-grammars` installs the grammars in a list that are not installed. It takes a list argument, so it is for `init.scm`, not the `:` prompt:

```scheme
(call! "plum/ensure-grammars" '("rust" "toml"))
```

`#:config (hash 'grammar-source "/path/to/folder")` makes PLUM install grammars from a folder you built them into, and download only the ones the folder lacks; see [Install from a local folder](../syntax-highlighting.md#install-from-a-local-folder).

Leaving PLUM out only removes these commands. Installed plugins, grammars and themes keep working without it; PLUM is only needed to install new ones. See [Syntax Highlighting](../syntax-highlighting.md) for the grammar workflow and [Themes](../themes.md#installing-themes) for the theme workflow.
