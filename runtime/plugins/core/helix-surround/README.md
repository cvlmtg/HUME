# core:helix-surround

Rebinds HUME's `m`-prefixed surround keys to Helix's layout: `ms` adds a surround, `md`
deletes one, `mr` replaces one.

## Usage

```scheme
(load-plugin! "core:helix-surround")
```

- **Depends on:** nothing.
- **Activates on:** `load-plugin!`. It exists to bind keys, so it has no `manifest.scm`
  (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-helix-surround).

## Key bindings

Each command reads the characters it needs from the keys typed after it.

| Key | Command | Effect |
|---|---|---|
| `m s` + char | `surround-add` (built in) | Surround the selection with the pair for char |
| `m d` + char | `helix-surround/delete` | Delete the surrounding pair for char |
| `m r` + old + new | `helix-surround/replace` | Replace the surrounding pair for old with the pair for new |
| `m w` | none | Unbound; HUME binds it to `surround-add` by default |

By default HUME binds `m s` + char to select the surrounding delimiters and `m w` + char
to add a surround. The plugin moves `surround-add` to `m s`, so `m s` does not select
surrounding delimiters while it is loaded. The native `surround-*` selection commands stay
registered, reachable from a key a config binds or from `call!`.

## How it works

Both helix commands read their delimiter with `on-next-char!`. `surround-cmd-for` maps a
delimiter character, opening or closing, to the name of the native `surround-*` command
for its pair. It answers `#f` for any other character, and for the `#f` a cancelled read
hands back, and both helix commands do nothing in that case.

`helix-surround/delete` calls that command to select the pair as two cursor selections,
then calls `delete`. `helix-surround/replace` calls it, then reads the next key with
`on-next-char!` and passes it to `replace`, which maps the new character onto each end of
the pair: typing `[` over a `(`-delimited selection yields `[` and `]`.
