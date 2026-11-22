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

`destinations.scm` writes every destination the plugin produces and reads a path, its part
before the first `#`, from the buffer's file's folder, the way a renderer does. A
destination is written bare when CommonMark reads it bare: not empty, no whitespace, not
starting with `<`, and parentheses paired. Otherwise it goes between `<` and `>`, which
hold anything but a line break, `<` or `>`; a text holding one of those gets no
destination, being too rare a name to escape. Backslash escapes are not read.

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

## Implementation notes

- The URL pattern is all lowercase, so `regex` matches it in either case, and it writes
  `[^\s]` for `\S` for that reason.
