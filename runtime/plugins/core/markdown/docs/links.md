# core:markdown — Links

## Link paste

The `on-paste` handler returns `[previous](destination)` when:

- `previous` is text the paste replaces: not `#f` (a paste at a cursor replaces nothing)
  and on one line;
- `pasted` is a URL (a scheme, `://`, and no whitespace, the whole text matching) or a path
  (below);
- `previous` holds no `[` or `]` and does not end with `\`, any of which would keep the
  result from reading as a link. A whole link (`[text](destination)`) holds both, so
  pasting over one replaces it, the way a link's destination is updated;
- `previous` is not a URL: pasting over one replaces it, the way a pasted URL is updated.
  A file name is not checked, since it is common link text.

Otherwise it returns `#f` and the paste goes in unchanged. The patterns are all lowercase,
so `regex` matches them in either case, and they write `[^\s]` for `\S` for that reason.

A path is a one-line `pasted` whose part before its first `#` names a file or folder that
exists, read from the buffer's file's folder: `path-join` keeps an absolute `pasted` as it
is. A `pasted` starting with `#` names a heading of this file, not a path, and links nothing,
so pasting a tag over text never makes a link. A buffer without a file links no path. A
relative one would be read from the working directory, which is not where the file will be,
and telling an absolute one apart takes `stdlib/path-absolute?`, whose first call loads a
core:stdlib entry, which no plugin may do while an `on-paste` handler runs. `destination` is
`pasted`, URL or path, written as [Destinations](#destinations) describes.

A link paste leaves the whole link selected, so a second `p` replaces the link with the bare
text: smart paste's repeat check compares the selection with the pasted text, and the link
differs from it.

## Destinations

`destinations.scm` writes every destination the plugin produces, pasted or completed, and
reads a path, its part before the first `#`, from the folder of the buffer's file, the way
a renderer reads it. A destination is bare when CommonMark reads it bare: not empty,
holding no whitespace, not starting with `<`, and its parentheses paired, the depth never
below zero and back to zero at its end. Any other destination goes between `<` and `>`,
which hold anything but a line break, a `<` or a `>`; a text holding one of those has no
destination, being too rare a name to escape. Backslash escapes are not read.

## Link paths

The `markdown:paths` source reads the cursor's line up to the cursor and finds the last `](`
on it. What follows is the destination typed so far, in one of two forms: the angle form
after a `<`, holding no `<` or `>`, and otherwise the bare form, holding no whitespace and no
`)` without its `(`, since that `)` ends the link. The source answers only in a buffer with a
file and inside one of the forms: it emits an empty list otherwise, which leaves the menu to
the other sources and, on a trigger character, shows nothing. A destination starting with
`/` gets the empty answer too, since joined to the buffer's folder it would name the
filesystem root, and so does one holding `#`, whose fragment names no file.

The typed destination splits at its last `/`: the part before it names a folder relative to
the buffer's file, and `path-entries` lists that folder, folders ending in `/`, as `:e` lists
them; when the part after the `/` starts with `.`, it lists the hidden entries instead. Each
item is a `textEdit` from the destination's start, since a space or a parenthesis ends the
token, and is matched as its path, which the menu reads from that start. In the bare form a
file's path is written as [Destinations](#destinations) describes. In the angle form it gets
the closing `>`, the edit covering a `>` already after the cursor so the result holds one. A
folder's item shows its `/` but inserts the path without it, so typing the `/` lists the
folder through its trigger; a folder the bare form cannot hold gets the opening `<` alone, so
that listing continues in the angle form. An entry neither form can hold is left out.

The source leaves the filtering to the menu's fuzzy matcher, so typing a name does not read
the folder on every keystroke. Its `#:token-chars` hold every ASCII punctuation character
but `(`, `)`, `<`, `>`, `#` and `\`, so the answer stays while a path is typed. A `(` among
them would carry the token start back past `](`, and a `)` ends the link. A space or a
parenthesis drops the answer until `Ctrl-Space` or a trigger character asks again. The
answer is incomplete only while the part after the last `/` is empty: the first character
typed asks it again, which a leading `.` needs to list the hidden entries, and every later
one only narrows that answer. A typed `/` asks again through its trigger and lists the folder it
ends. Erasing back over a `/` keeps the folder's answer, whose paths still match what is
typed, until a `/` or `Ctrl-Space` asks again. Its empty answer outside a destination is
complete, so typing elsewhere does not ask it again. `(`, `<` and `/` are Markdown trigger
characters for it, so `](`, `](<` and a folder's `/` open the menu without `Ctrl-Space`.
