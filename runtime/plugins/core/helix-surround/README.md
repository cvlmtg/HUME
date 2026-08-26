# core:helix-surround

Helix-compat `ms`/`md`/`mr` surround shortcuts — reroutes HUME's default surround keys to
the Helix layout.

## Usage

```scheme
(load-plugin "core:helix-surround")
```

Loads eagerly: `m s`/`m d`/`m r` are the only way to reach this plugin's commands, so a lazy
`declare-plugin` would have no trigger to ever activate it. See
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-helix-surround) for what
it rebinds.

## Commands

| Command | Effect |
|---|---|
| `helix-delete-surround` | Delete the surrounding delimiter pair (`m d` + char) |
| `helix-replace-surround` | Replace the surrounding pair with a new char (`m r` + char + char) |

## How it works

Native `select-surround`/`surround-*` commands stay registered — only their keybindings
move — so they're still reachable via the typed-command interface (`:surround-paren`, …)
while this plugin is loaded.

`surround-cmd-for` maps a delimiter char to its `surround-*` command name, returning `#f` for
anything unrecognised so callers can skip gracefully instead of erroring on a stray keypress.

`helix-replace-surround` doesn't implement replacement itself: it selects the existing pair
via the matching `surround-*` command, then calls `request-wait-char! "replace"` so the *next*
key becomes the pending-char argument to HUME's built-in `replace` — which already does smart
open/close substitution (`(` on a `(`-delimited selection yields `[`), so this plugin gets
that behavior for free rather than re-implementing delimiter-pair logic.
