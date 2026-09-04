# core:git-diff

Live, VSCode-style inline git diff — compares the buffer against a git ref (default `HEAD`)
as it's edited, rendering gutter `+`/`-`/`~` signs, deleted lines as virtual rows,
added/changed lines with a background tint, and word-level highlights inside changed lines.
Also keeps a `"steel:git-branch"` statusline element fresh for the focused buffer — place it
yourself, no config needed.

## Usage

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:git-diff"
  #:config (hash "signs" #t "inline" #f "ref" "HEAD"))
```

Requires `core:stdlib` declared or loaded first — config validation calls
`stdlib/config-boolean`/`stdlib/config-string` via `call!` while this plugin's body
evaluates, and `call!`'s lazy-miss retry inline-activates a merely declared `core:stdlib`
before the read runs (see ["Depending on another
plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)). The bare
`declare-plugin` above resolves `manifest.scm`, activating on the first buffer opened or the
first `:toggle-git-signs`/`:toggle-inline-diff` typed; an explicit
`#:commands`/`#:events`/`#:languages` bypasses it. `"signs"` defaults on (cheap, no
line-shifting side effects); `"inline"` defaults off (it moves virtual rows into the
buffer's visual flow). See
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-git-diff) for value
semantics and key-binding examples — no default key bindings ship with this plugin.

Branch tracking has no config flag — placement is the switch. It doesn't fetch until
`"steel:git-branch"` appears in your own `configure-statusline!` call, and starts the moment
it does (see [Statusline → Custom
elements](https://cvlmtg.github.io/HUME/configuration.html#custom-elements)).

## Commands

| Command | Effect |
|---|---|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer |

## Documentation

Design and implementation notes, for contributors reading this plugin's source:

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | File layout, the signs+inline design choice, the per-buffer state model, ref handling |
| [`docs/pipeline.md`](docs/pipeline.md) | The fetch/diff pipeline (cache states, severity tiers, debounce) and branch tracking |
| [`docs/rendering.md`](docs/rendering.md) | Signs, virtual deleted lines + word highlights, row background tint, the flag→renderer dispatch |
