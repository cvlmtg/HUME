# core:classic-paste

Opt-in "GUI-style" copy/paste split: a predictable clipboard-vs-kill-ring binding
scheme, as an alternative to HUME's default smart-`p` heuristic.

## Usage

```scheme
(load-plugin "core:classic-paste")
```

- **Depends on:** nothing.
- **Activates on:** its own key bindings only (`p`/`P`/`Ctrl-v`/`Ctrl-Shift-v`). It has
  no `manifest.scm`, so it must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-classic-paste)
  for the binding scheme.

## Commands

| Command | Effect |
|---|---|
| `classic-ring-after` | Paste the kill-ring head after the selection |
| `classic-ring-before` | Paste the kill-ring head before the selection |
| `classic-clipboard-after` | Paste the OS clipboard after the selection |
| `classic-clipboard-before` | Paste the OS clipboard before the selection |

## How it works

Each wrapper command calls `set-register-prefix!` (`"k"` for the kill-ring head, `"c"`
for the OS clipboard) immediately before dispatching to the built-in `paste-after`/
`paste-before`. `set-register-prefix!` arms a *sticky* register for exactly the next
`call!` (the built-in paste commands read it, then it's consumed), so there's no
persistent state to reset between invocations. This is the same mechanism the raw
register-prefixed commands (`"kp`, `"cP`, etc.) use; these wrappers just pre-arm the
prefix so one keypress does what would otherwise take two.

## Known limitations

`Ctrl-Shift-v` is only delivered as a distinct event under the kitty keyboard protocol.
On legacy terminals it's typically encoded identically to `Ctrl-v`, or intercepted by the
terminal emulator as its own paste shortcut, so it may never reach HUME. `Ctrl-v` itself
is delivered reliably under both kitty and legacy encodings.
