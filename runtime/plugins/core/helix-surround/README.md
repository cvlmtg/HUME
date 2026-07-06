# core:helix-surround

Helix-compat `ms`/`md`/`mr` surround shortcuts — reroutes HUME's default surround keys to
the Helix layout.

## Usage

```scheme
(load-plugin "core:helix-surround")
```

## Keys

| Key                     | Command                | Effect                                              |
|-------------------------|-------------------------|------------------------------------------------------|
| `m s` + char             | surround-add            | Wrap each selection with `char` (and its pair-close)  |
| `m d` + char             | helix-delete-surround   | Delete the surrounding delimiter pair                 |
| `m r` + char + new_char  | helix-replace-surround  | Replace the surrounding pair with `new_char`          |

This overwrites HUME's default `ms` keybind (`select-surround`) and unbinds `mw`
(`surround-add`'s native binding) while the plugin is loaded. The underlying
`select-surround` / `surround-*` commands (`surround-paren`, `surround-bracket`, etc.) stay
registered and reachable via the typed-command interface (`:surround-paren`, ...) — only the
keybindings move.

## How it works

`surround-cmd-for` maps a delimiter char to its `surround-*` command name, returning `#f` for
anything unrecognised so callers can skip gracefully instead of erroring on a stray keypress.

`helix-replace-surround` doesn't implement replacement itself: it selects the existing pair
via the matching `surround-*` command, then calls `request-wait-char! "replace"` so the *next*
key becomes the pending-char argument to HUME's built-in `replace`. `replace` already does
smart open/close substitution — e.g. pressing `(` on a `(`-delimited selection yields `[`,
`)` yields `]` — so `helix-replace-surround` gets that behavior for free rather than
re-implementing delimiter-pair logic.
