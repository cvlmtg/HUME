# core:markdown

Draws Markdown without its syntax, in the style of Obsidian's live preview: the lines a
selection touches show their source, and a code block or table shows its source whole
while a selection is inside it. It also edits Markdown: a URL or path pasted over text
becomes a link, link destinations complete paths, Enter, `o` and `O` continue lists and
quotes, ordered lists renumber, `G X` toggles task boxes, `M B`, `M I`, `M C` and `M S`
toggle bold, italic, code and strikethrough, and table keys select cells and columns and
move rows and columns.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown" #:config (hash 'preview #t 'rule-width 40))
```

- **Depends on:** `core:stdlib`, for config validation, list concatenation and its
  selection queries.
- **Activates on:** the first `markdown` buffer, or any command in `manifest.scm`, a
  Markdown popup's `format-text/markdown` call included. A `#:languages` manifest may list
  no buffer event, so it lists none.
- **Grammars:** `markdown` and `markdown.inline`. A missing one logs a warning once a
  session, naming `:plum-install-grammar`.
- **User docs:**
  [core:markdown](https://cvlmtg.github.io/HUME/core-plugins/markdown.html).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'preview` | bool | `#t` | Whether the preview starts on in each buffer. Popups are drawn either way |
| `'rule-width` | integer ≥ 1 | `40` | How many `─` a rule draws, since a plugin cannot read the pane width |
| `'continue-lists` | bool | `#t` | Whether Enter, `o` and `O` continue lists and quotes, and an edit renumbers the ordered items after it |

## Commands

The keys are bound in the `"markdown"` buffer keymap, which `on-buffer-open` attaches.
Enter, `o` and `O` are bound only with `'continue-lists` on.

| Mode | Key | Command | Effect |
|---|---|---|---|
| | | `:toggle-markdown-preview`, `markdown/toggle-preview` | Turn the current buffer's preview off, clearing what it drew, or back on |
| | | `format-text/markdown` | `(format-text/markdown text parsed)`: a popup's conceals, line backgrounds and highlights |
| Insert | Enter | `markdown/enter` | Continue the list or quote, or end it on an empty item |
| Normal | `o`, `O` | `markdown/open-below`, `markdown/open-above` | Open a line continuing the list or quote |
| Normal | `g d` | `markdown/follow-link` | Follow the link under the primary cursor; a wiki or reference link runs `lsp/goto-definition` |
| Normal | `G X` | `markdown/toggle-task` | Check or uncheck task boxes |
| Normal | `M B`, `M I` | `markdown/bold`, `markdown/italic` | Toggle bold or italic |
| Normal | `M C`, `M S` | `markdown/code`, `markdown/strikethrough` | Toggle inline code or strikethrough |
| Normal | `m i \|`, `m a \|` | `markdown/select-cell`, `markdown/select-around-cell` | Select the cell's text, or the cell with its pipe |
| Normal | `m \|` | `markdown/select-column` | Select the primary selection's column |
| Normal | `G T k`, `G T j` (or ↑, ↓) | `markdown/move-row-up`, `markdown/move-row-down` | Move table rows |
| Normal | `G T h`, `G T l` (or ←, →) | `markdown/move-column-left`, `markdown/move-column-right` | Move table columns |

## Documentation

| Doc | Covers |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Decisions, limitations, file layout, rendering, popups, how the queries fetch and filter matches |
| [`docs/elements.md`](docs/elements.md) | What each element is drawn as, and the grammar's node shapes |
| [`docs/links.md`](docs/links.md) | Link paste, link-path completion and following links |
| [`docs/lists.md`](docs/lists.md) | List and quote continuation, renumbering, task boxes |
| [`docs/tables.md`](docs/tables.md) | Reading table rows, selecting cells and columns, moving rows and columns |
