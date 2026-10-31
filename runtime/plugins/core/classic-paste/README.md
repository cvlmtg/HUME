# core:classic-paste

Opt-in "GUI-style" copy/paste split: `p`/`P` paste from the kill-ring and `Ctrl-v`/
`Ctrl-Shift-v` paste from the OS clipboard, in place of HUME's default smart-`p`
heuristic, which picks between the two by whether anything was edited since the last copy
([Copy & Paste](https://cvlmtg.github.io/HUME/copy-and-paste.html)).

## Usage

```scheme
(load-plugin! "core:classic-paste")
```

- **Depends on:** nothing.
- **Activates on:** `load-plugin!`. It exists to bind keys, so it has no `manifest.scm`
  (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-classic-paste).

## Key bindings

All four are bound in Normal mode.

| Key | Command | Effect |
|---|---|---|
| `p` | `classic-paste/ring-after` | Paste the kill-ring head after the selection |
| `P` | `classic-paste/ring-before` | Paste the kill-ring head before the selection |
| `Ctrl-v` | `classic-paste/clipboard-after` | Paste the OS clipboard after the selection |
| `Ctrl-Shift-v` | `classic-paste/clipboard-before` | Paste the OS clipboard before the selection |

## How it works

Each command calls `set-register-prefix!` (`"k"` for the kill-ring head, `"c"` for the OS
clipboard) and then `call!`s the built-in `paste-after` or `paste-before`. The prefix
applies to every `call!` after it in the same command body and ends with that body, so no
state carries over to the next keypress. Each command is the same paste as the
register-prefixed form its docstring names (`"kp`, `"cP`, and so on).

## Known limitations

- **`Ctrl-Shift-v` needs the kitty keyboard protocol.** A legacy terminal encodes it the
  same as `Ctrl-v`, or the terminal emulator intercepts it as its own paste shortcut, so it
  may never reach HUME. `Ctrl-v` arrives under both encodings.
