# core:stdlib

A toolkit of small helpers that other plugins build on, not something you use directly. `core:buffer-words`, `core:git-diff`, `core:lsp`, `core:lsp-install`, `core:pickers`, `core:plum` and `core:vim-keybind` all depend on it, so load it before them:

```scheme
(load-plugin! "core:stdlib")
```

::: warning Keep its own triggers
Load `core:stdlib` as shown above, without a `declare-plugin!` line of your own. The plugins that depend on it call every one of its helpers; with a narrower trigger list, a plugin fails with an error when it calls a helper you left out.
:::

If you're writing a plugin, see [Standard Library](../standard-library.md) for every command it offers.
