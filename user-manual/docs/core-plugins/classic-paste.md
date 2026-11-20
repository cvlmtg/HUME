# core:classic-paste

GUI-style paste, if you'd rather keep the clipboard on its own keys:

- `p` / `P` paste the kill ring.
- `Ctrl-v` / `Ctrl-Shift-v` paste the system clipboard (`Ctrl-Shift-v` needs the kitty protocol).
- [`clipboard-joins-ring`](../settings.md) is set to `never`, so copied text stays off the kill ring. A `clipboard-joins-ring` you set yourself wins.

```scheme
(load-plugin! "core:classic-paste")
```
