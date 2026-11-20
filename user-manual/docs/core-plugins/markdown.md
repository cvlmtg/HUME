# core:markdown

<img src="/core_markdown_preview.png" alt="core:markdown in action" />

Draws Markdown without its syntax, in the style of a rendered preview, right in the buffer you edit:

- A heading's `#` becomes its level as a glyph of one to six dots and a bar (`⠘▌` for a second-level heading) over a tinted band.
- Bullets become dots, task boxes become checkboxes, quotes get a bar, and horizontal rules become a line.
- Code fences give way to a background band, and tables are drawn as box grids.
- Emphasis, strikethrough and code spans lose their markers, and links show only their text.
- Escapes and entities (`\*`, `&amp;`, `&#65;`) show the character they stand for.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:markdown")
```

Requires `core:stdlib` loaded first. It starts when you open a Markdown file, or when a Markdown popup opens with the `markdown` grammar installed.

It also draws Markdown popups, such as a language server's hover, and Markdown documentation in the completion menu's documentation box. A popup shows no source, since no selection is in it, and a table wider than the popup breaks its grid.

It needs the `markdown` and `markdown.inline` grammars: install them with `:plum-install-grammar` if HUME warns that one is missing. Without `markdown.inline`, the headings, lists, quotes, rules, code blocks and tables are drawn and the emphasis, links and escapes stay as written.

Every line the cursor or a selection touches shows its source, in every mode. While a selection is inside a code block or a table, the whole block shows its source.

Pasting a URL over selected text makes a link: select `foo`, paste `https://example.com`, and get `[foo](https://example.com)`. It works with `p`, `P` and your terminal's paste shortcut, and one `u` brings the selected text back.

Pasting the path of a file or folder does the same: over `foo`, `notes/todo.md` gives `[foo](notes/todo.md)`. It makes a link when the file or folder exists, a relative path being read from the Markdown file's folder. A path to a heading works too: `notes/todo.md#today` makes a link when `notes/todo.md` exists. A URL or a path with a space, or with a parenthesis that has no partner, goes between `<` and `>`, as Markdown needs. In a buffer not yet saved to a file, only a URL makes a link.

Pasting a URL or a path over a URL or a link replaces it, as does pasting at a cursor, over text that spans lines, or over text that holds a `[` or `]` or ends with `\`.

In a link's destination, right after `](` or `](<`, `Ctrl-Space` lists the files and folders next to the Markdown file. After a folder you have typed, such as `](notes/`, it lists the ones in that folder. Typing `(` right after `]`, `<` right after `](`, or `/` after a folder's name, opens the list too.

- The list narrows as you type, and the letters need only appear in order, so `rdm` finds `README.md`.
- Start the name with `.` to list hidden files.
- A name with a space, or with a parenthesis that has no partner, goes between `<` and `>`. A folder's name gets only the `<`, so you can go on to a file inside it, which adds the `>`.
- Typing a space inside `<` and `>` closes the list; `Ctrl-Space` opens it again.

In a list, `Enter` starts the next item: a bullet gets the same bullet, a numbered item the next number, and a task item an unchecked box. A nested item continues at its own indent, and a quote line gets the next line's `>`. `o` and `O` open a line below or above in the same way. Adding an item to a numbered list renumbers the items after it, so a new item after `2.` turns the `3.` below it into `4.`.

`Enter` on an item with nothing after its marker ends the list instead: the marker goes, and the items after it are renumbered to close the gap. On an empty quote line, it ends the quote.

`G X` checks the task boxes on the lines your selections touch, or unchecks them all when the first box on the main selection's lines is checked. When the main selection's lines have no box, the first box of all decides. Lines without a box don't change.

With a Markdown [language server](../lsp.md#install-a-server) installed (`:lsp-install markdown` installs marksman), the symbol keys jump between sections. `z s` lists the file's headings to pick one, and `z S` searches the headings of every Markdown file in the project as you type. marksman reads the files you haven't opened only when the project's folder is a git repository or holds a `.marksman.toml` file.

The preview draws ordered list markers (`1.`), bracketed text such as `[TODO]` or `[!NOTE]`, entity names other than common ones such as `&amp;` and `&copy;`, references to control characters, and Markdown inside a fenced code block as written.

The colors and bands come from the [theme](../themes.md#scopes-hume-adds); to change them without editing it, [style the scopes from your config](../themes.md#styling-scopes-from-your-config).

Only the lines on screen are drawn. After you scroll, the lines that came into view appear as written until `hook-viewport-debounce-ms` after you stop; lower it in [Settings](../settings.md#global-options) to shorten the wait.

| Command | Effect |
|---------|--------|
| `:toggle-markdown-preview` | Turn the preview off for the current buffer, or back on |
| `markdown/toggle-preview` | The same, as an editor command you can bind to a key |
| `markdown/enter` | What `Enter` does in a Markdown buffer |
| `markdown/open-below`, `markdown/open-above` | What `o` and `O` do in a Markdown buffer |
| `markdown/toggle-task` | What `G X` does in a Markdown buffer |

Configure with `#:config`:

```scheme
(load-plugin! "core:markdown"
  #:config (hash 'preview #f 'rule-width 60))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'preview` | boolean | `#t` | Whether the preview starts on in each buffer. Popups and the completion documentation box are drawn either way |
| `'rule-width` | integer (≥ 1) | `40` | How many `─` a horizontal rule draws |
| `'continue-lists` | boolean | `#t` | Whether `Enter`, `o` and `O` continue a list or quote |

A table wider than the pane draws a broken grid when soft wrap is on. A table's top and bottom border appear only on an empty line next to it. A tab inside a table cell throws its column out of line.
