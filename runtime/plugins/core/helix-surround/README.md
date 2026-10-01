# core:helix-surround

Rebinds HUME's `m`-prefixed surround keys to Helix's layout: `ms` adds a surround, `md`
deletes one, `mr` replaces one.

## Usage

```scheme
(load-plugin! "core:helix-surround")
```

- **Depends on:** nothing.
- **Activates on:** its own key bindings only. It has no `manifest.scm`, so it must be
  loaded eagerly (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-helix-surround).

## Key bindings

Each key waits for a character before it dispatches.

| Keys | Command | Effect |
|---|---|---|
| `m s` + char | `surround-add` (built in) | Surround the selection with the pair for char |
| `m d` + char | `helix-delete-surround` | Delete the surrounding pair for char |
| `m r` + old + new | `helix-replace-surround` | Replace the surrounding pair for old with the pair for new |
| `m w` | none | Unbound; HUME binds it to `surround-add` by default |

By default HUME binds `m s` + char to select the surrounding delimiters and `m w` + char
to add a surround. The plugin moves `surround-add` to `m s`, so selecting surrounding
delimiters with `m s` is not available while it is loaded. HUME's native `surround-*`
selection commands stay registered and are reachable from the command line.

## How it works

`surround-cmd-for` maps a delimiter character, opening or closing, to the name of the
native `surround-*` command for its pair. It answers `#f` for any other character, and both
helix commands do nothing in that case.

`helix-delete-surround` calls that command to select the pair as two cursor selections,
then calls `delete`. `helix-replace-surround` calls it, then `request-wait-char! "replace"`,
so the next key becomes the argument to the built-in `replace`.

## Design decisions

`helix-replace-surround` delegates the substitution to `replace`, which maps the new
character onto each end of the pair: typing `[` over a `(`-delimited selection yields `[`
and `]`.
