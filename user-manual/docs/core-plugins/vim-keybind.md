# core:vim-keybind

Vim keys: `$`, `^`, `0`, `C` and `D` (change/delete to end of line), `Ctrl-6` (alternate buffer), and `o` in Extend mode to swap the selection's ends. It doesn't bind `G`, which is HUME's own prefix (`G L`/`G U`/`G C`/`G G`/`G A`/`G S`, plus `G R` with `core:lsp`); `g e` already goes to the last line.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind")
```

Requires `core:stdlib` loaded first.

By default (`'smart`), `C` is context-sensitive. On a bare cursor with no count it changes to end of line, as in Vim. With a real selection or any count prefix (e.g. `3C`), it runs HUME's own `copy-selection-on-next-line`, so that command stays reachable. Change this with `#:config`:

```scheme
(load-plugin! "core:vim-keybind" #:config (hash 'change-to-eol 'on))
```

`'on` always changes to end of line; `'off` leaves `C` alone. Every value needs `core:stdlib`.
