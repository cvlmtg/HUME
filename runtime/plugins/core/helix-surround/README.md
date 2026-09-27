# core:helix-surround

Rebinds HUME's `m`-prefixed surround keys to Helix's own `ms`/`md`/`mr` layout.

## Usage

```scheme
(load-plugin "core:helix-surround")
```

- **Depends on:** nothing.
- **Activates on:** its own key bindings only. It has no `manifest.scm`, so it must be
  loaded eagerly (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-helix-surround).

## Commands

| Command | Effect |
|---|---|
| `helix-delete-surround` | Delete the surrounding delimiter pair (`m d` + char) |
| `helix-replace-surround` | Replace the surrounding pair with a new char (`m r` + char + char) |

## How it works

HUME's native `select-surround`/`surround-*` commands stay registered under this plugin
(only their keybindings move), so they're still reachable via the typed-command interface
(`:surround-paren`, …) while it's loaded. `surround-cmd-for` maps a delimiter char to its
`surround-*` command name, answering `#f` for anything unrecognized so both wrapper
commands can skip gracefully instead of erroring on a stray keypress.

## Design decisions

- **`helix-replace-surround` delegates to `replace`'s pending-char wait, rather than
  reimplementing delimiter substitution.** It selects the existing pair via the matching
  `surround-*` command, then calls `request-wait-char! "replace"` so the next key becomes
  the pending-char argument to HUME's built-in `replace`. `replace` already does smart
  open/close substitution (`(` on a `(`-delimited selection yields `[`), so this plugin
  gets that behavior for free.
