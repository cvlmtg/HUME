# core:markdown — Links

## Link paste

The `on-paste` handler turns a paste into `[previous](destination)` when `pasted` is a URL
or a path, and `previous`, the text it replaces:

- exists and is on one line (a paste at a cursor replaces nothing);
- holds no `[` or `]` and does not end with `\`, any of which would keep the result from
  reading as a link. A whole link holds both, so pasting over one replaces it;
- is not a URL, so pasting over one replaces it. A file name is not checked, since it is
  common link text.

Otherwise the paste goes in unchanged.

A **URL** is a scheme, `://` and no whitespace. A **path** is a one-line `pasted` whose
part before its first `#` names an existing file or folder, read from the buffer's file's
folder (`path-join` keeps an absolute path as it is). A `pasted` starting with `#` names a
heading, not a path, so pasting a tag over text makes no link. A buffer without a file
links no path: a relative one would be read from the working directory, and telling an
absolute one apart takes `stdlib/path-absolute?`, whose first call loads a core:stdlib
entry, which no plugin may do inside an `on-paste` handler.

The link stays selected, so a second `p` replaces it with the bare text: smart paste's
repeat check compares the selection with the pasted text, and the link differs from it.

## Destinations

`destinations.scm` writes every destination the plugin produces, splits one into its path,
the part before the first `#`, and its fragment, the part after, and reads a path from the
buffer's file's folder, the way a renderer does. A path is read as given, so a caller
decoding one splits it first and a `%23` in it stays part of a file's name. A
destination is written bare when CommonMark reads it bare: not empty, no whitespace, not
starting with `<`, and parentheses paired. Otherwise it goes between `<` and `>`, which
hold anything but a line break, `<` or `>`; a text holding one of those gets no
destination, being too rare a name to escape. A destination written or completed is never
backslash-escaped; one followed is read with its escapes, as [Following
links](#following-links) describes.

## Link paths

The `markdown:paths` source finds the last `](` before the cursor on its line. What
follows is the destination typed so far: after a `<`, the angle form, holding no `<` or
`>`; otherwise the bare form, holding no whitespace and no unpaired `)`, which would end
the link. The source answers an empty list, leaving the menu to the other sources, in a
buffer without a file, outside both forms, and for a destination starting with `/` (joined
to the buffer's folder it would name the filesystem root) or holding `#` (a fragment names
no file).

The typed destination splits at its last `/`. The part before it names a folder relative
to the buffer's file, which `path-entries` lists, or lists its hidden entries when the
part after the `/` starts with `.`. Each item:

- is a `textEdit` from the destination's start, matched as its path, since a space or a
  parenthesis in the destination would end the menu's token early;
- in the bare form, is written as [Destinations](#destinations) describes; in the angle
  form, gets the closing `>`, covering a `>` already after the cursor;
- for a folder, shows its `/` but inserts the path without it, so typing the `/` lists the
  folder through its trigger. A folder the bare form cannot hold gets the opening `<`
  alone, so the listing continues in the angle form.

An entry neither form can hold is left out.

The menu's fuzzy matcher does the filtering, so typing does not read the folder on every
keystroke:

- `#:token-chars` hold every ASCII punctuation character but `(`, `)`, `<`, `>`, `#` and
  `\`, so the answer stays while a path is typed. A `(` would carry the token start back
  past `](`, and a `)` ends the link. A space or a parenthesis drops the answer until
  `Ctrl-Space` or a trigger asks again.
- The answer is incomplete only while the part after the last `/` is empty, so the first
  character typed asks again, which a leading `.` needs, and later ones only narrow.
  Erasing back over a `/` keeps the folder's answer, whose paths still match.
- The empty answer outside a destination is complete, so typing elsewhere does not ask
  again.
- `(`, `<` and `/` are Markdown trigger characters for the source, so `](`, `](<` and a
  folder's `/` open the menu.

## Following links

`markdown/follow-link` follows the innermost link holding the primary cursor's head. It
follows an `inline_link`, `image`, `uri_autolink` or `email_autolink` itself, the last as a
`mailto:` URL. On a reference link (`full_reference_link`, `collapsed_reference_link` or
`shortcut_link`) it runs `lsp/goto-definition`, what `g d` runs without this plugin: the
server resolves the label against every file. The grammar has no wiki link node and reads
`[[b]]` as a shortcut link `[b]` between two brackets, so a reference link wrapped in one
more pair of brackets also covers them, and a wiki link takes the same path. The grammar
also reads any bracketed text as a shortcut link, so that text asks the server too.
Anywhere else, or without the `markdown.inline` grammar, the command does nothing and
returns `#f`, as it does on a reference link when `lsp/goto-definition` does not exist, so
a binding can try another command after it.

An `inline_link` and an `image` go by their destination, its `<` `>` dropped, a
`uri_autolink` by its text. Each reads as the preview shows it: the conceals `inline.scm`
makes for its `markdown.inline` captures applied, so an autolink loses its `<` `>`, and a
destination its escapes and the entities and numeric references `entities.scm` knows. An
unknown entity stays as written. A `\#` or `&#35;` then splits like a `#`, as in a
renderer. The destination is then a URL when the part before its
first `:` is a scheme as CommonMark defines one for autolinks: a letter, then 1 to 31
letters, digits, `+`, `.` or `-`, so a drive letter is not one. It is a fragment when it
starts with `#`, else a path read as [Destinations](#destinations) describes. The paste's
URL pattern is narrower: it also needs `://`. The path and the fragment are percent-decoded
apart, after the split at the first `#`, so `%20` names a space. A `%` without two hex
digits stays as written, and an escape that decodes to malformed UTF-8 raises.

A URL, and an image's path, open through `open`, `xdg-open` or
`rundll32 url.dll,FileProtocolHandler`. None of them reads the target as shell syntax,
which `cmd /c start` does, running a URL's `&` as a command separator. Each does run a
program it is handed, a `file:` URL, an `.exe` or a `.command` file alike, so only the
`http`, `https` and `mailto` schemes reach them and any other raises, and only an image's
path ending in a raster or SVG image extension does. A path must name an existing file,
which opens through `open-buffer!` and `goto-location!`, an image's other paths included.

A fragment names the first heading whose name equals it, ignoring case. The headings come
from the file's syntax tree: ATX headings, their closing `#`s dropped, and setext headings,
a heading in a quote or a list item included. A heading's text is what the preview shows of
it: the conceals `inline.scm` makes for its `markdown.inline` captures applied, so a link
gives its text and an entity its character, and the raw source without that grammar. Its
slug is that text lowercased, with only letters, digits, `_`, `-` and spaces kept and each
space made a `-`. Its name is the slug, with `-1`, `-2` and so on added to each later
heading of the same slug, in the file's order, as GitHub names them.

An empty fragment, a bare `#`, names the file's start, as in a browser.

A file just opened has no tree until its first parse lands, and no language until the
command that opened it returns. No query tells a parse still to come from a file without
the heading, so a heading not found in a file whose language is `""` or `markdown` is
waited for, unless the file's last `on-syntax-changed` came at its current generation: its
tree is then current, and the miss warns at once. The pane shows the file's start, and the file's next `on-syntax-changed` moves
it to the heading, or warns that no heading matches. It moves only while the pane still
shows the file, its text is unchanged and its primary cursor is still at the start, so a
move or an edit made before the parse lands keeps its place. The second move records a
second jump, so `Ctrl-o` stops at the file's start on its way back. One follow waits at a
time, and the next follow drops it.

A wait that no parse ends within a second warns that no heading matches and goes on
waiting, so a parse that lands later still moves to the heading. That covers a file that
turns out not to be Markdown, whose hooks never reach this plugin. A file in another language, or any file while
the `markdown` grammar is missing, warns at once.

## Implementation notes

- The URL pattern is all lowercase, so `regex` matches it in either case, and it writes
  `[^\s]` for `\S` for that reason.
