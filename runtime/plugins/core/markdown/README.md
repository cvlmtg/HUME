# core:markdown

Draws Markdown without its syntax, in the style of Obsidian's live preview. Every line a selection touches shows its source, and a code block or a table shows its source as a whole while a selection is inside it. It also turns a URL pasted over selected text into a link, completes the path in a link's destination, and continues a list or quote on Enter, `o` and `O`.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown" #:config (hash 'preview #t 'rule-width 40))
```

- **Depends on:** `core:stdlib`, for config validation.
- **Activates on:** the first `markdown` buffer, or any of its six commands
  (`manifest.scm`), the first Markdown popup's `format-text/markdown` call included. The
  manifest lists no events: a `#:languages` manifest may list no buffer event.
- **Grammars:** `markdown` and `markdown.inline`. The first buffer opened without one
  logs a warning per missing grammar, once a session, naming `:plum-install-grammar`.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-markdown).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'preview` | bool | `#t` | Whether the preview starts on for each buffer. Popups are drawn either way |
| `'rule-width` | integer ≥ 1 | `40` | How many `─` a horizontal rule draws. Plugins cannot read the pane width |
| `'continue-lists` | bool | `#t` | Whether Enter, `o` and `O` continue a list or quote in each Markdown buffer |

## Commands

| Command | Effect |
|---|---|
| `:toggle-markdown-preview` | Turn the preview off for the current buffer, clearing everything it drew, or back on, rendering at once |
| `markdown/toggle-preview` | The same function, bindable |
| `format-text/markdown` | `(format-text/markdown text parsed)`: what the preview draws for a text and its parsed tree, as the hash a text formatter returns. The editor calls it for every Markdown popup and the completion documentation box, and `call!` reaches it |
| `markdown/enter` | Enter in a Markdown buffer: start the next list item or quote line, renumbering an ordered list, or end the list on an empty item; elsewhere, Enter's normal behaviour |
| `markdown/open-below`, `markdown/open-above` | `o` and `O` in a Markdown buffer: open a line, continuing the list item or quote and renumbering an ordered list |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Decisions, limitations, file layout, rendering, popups, widening by query roots, overlapping conceals, fenced blocks |
| [`docs/elements.md`](docs/elements.md) | Block elements, inline elements, tables, node shapes |
| [`docs/links.md`](docs/links.md) | Link paste and link-path completion |
| [`docs/lists.md`](docs/lists.md) | List and quote continuation, ending a list, renumbering |
