# core:markdown

Draws Markdown without its syntax, in the style of Obsidian's live preview. Every line a selection touches shows its source, and a code block or a table shows its source as a whole while a selection is inside it. It also turns a URL pasted over selected text into a link, and completes the path in a link's destination.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown" #:config (hash 'preview #t 'rule-width 40))
```

- **Depends on:** `core:stdlib`, for config validation.
- **Activates on:** the first buffer whose language is `markdown`, or any of its three
  commands (`manifest.scm`), the first Markdown popup's `format-text/markdown` call
  included. The manifest lists no events: a `#:languages` manifest may list no buffer
  event, and the language is the trigger.
- **Grammars:** `markdown` and `markdown.inline`. The first buffer opened without one of
  them logs one warning per missing grammar per session, naming the
  `:plum-install-grammar` command that installs it.
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-markdown).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'preview` | bool | `#t` | Whether the preview starts on for each buffer. Popups are drawn either way |
| `'rule-width` | integer ≥ 1 | `40` | How many `─` a horizontal rule draws. Plugins cannot read the pane width |

## Commands

| Command | Effect |
|---|---|
| `:toggle-markdown-preview` | Turn the preview off for the current buffer, clearing everything it drew, or back on, rendering at once |
| `markdown/toggle-preview` | The same function, bindable |
| `format-text/markdown` | `(format-text/markdown text parsed)`: what the preview draws for a Markdown text, given the text and its parsed tree, as the hash a text formatter returns. The editor calls it for every Markdown popup and the completion documentation box, and `call!` reaches it |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Decisions, limitations, file layout, rendering, popups, widening by query roots, overlapping conceals, fenced blocks |
| [`docs/elements.md`](docs/elements.md) | Block elements, inline elements, tables, node shapes |
| [`docs/links.md`](docs/links.md) | Link paste and link-path completion |
