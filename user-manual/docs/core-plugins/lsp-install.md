# core:lsp-install

Downloads language servers and sets them up for `core:lsp`, with `:lsp-install`, `:lsp-uninstall` and `:lsp-catalog`. Each language uses the same servers Helix picks for it. Without this plugin, `core:lsp` works with servers you [register yourself](../lsp.md#registering-a-language-server).

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp-install")
```

Requires `core:stdlib` loaded first. See [Language Servers](../lsp.md#installing-servers) for the prerequisites and the commands.
