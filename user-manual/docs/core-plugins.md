# Core Plugins

HUME ships these plugins under the `core:` namespace:

- [`core:plum`](core-plugins/plum.md): a plugin and grammar manager
- [`core:lsp`](core-plugins/lsp.md): language server support
- [`core:lsp-install`](core-plugins/lsp-install.md): an installer for the language servers
- [`core:steel-server`](core-plugins/steel-server.md): a language server for your Scheme config
- [`core:pickers`](core-plugins/pickers.md): fuzzy pickers
- [`core:git-diff`](core-plugins/git-diff.md): live git diff
- [`core:undotree`](core-plugins/undotree.md): an undo-history graph
- [`core:markdown`](core-plugins/markdown.md): Markdown drawn without its syntax
- [`core:colorizer`](core-plugins/colorizer.md): color literals painted with their color
- [`core:buffer-words`](core-plugins/buffer-words.md): completion from the words in a buffer
- [`core:vim-keybind`](core-plugins/vim-keybind.md) and [`core:classic-paste`](core-plugins/classic-paste.md): keymap alternatives
- [`core:stdlib`](core-plugins/stdlib.md): the standard library the others build on

None of them load automatically: nothing runs until you ask for it in your [`init.scm`](configuration.md).

Bring a plugin in with `load-plugin!`:

```scheme
(load-plugin! "core:plum")
```

Most core plugins wait until you first use them, such as when you run one of their commands or open a file in a language they handle. `core:buffer-words` and the ones that add keys load at startup, except `core:lsp`, which loads the first time a buffer gets a language. See [How plugins are loaded](plugins.md#how-plugins-are-loaded).
