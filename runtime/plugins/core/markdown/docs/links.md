# core:markdown — Links

## Link paste

The `on-paste` handler returns `[previous](pasted)` when:

- `previous` is text the paste replaces: not `#f` (a paste at a cursor replaces nothing)
  and on one line;
- `pasted` is a URL: a scheme, `://`, and no whitespace, the whole text matching;
- `previous` holds no `[` or `]` and does not end with `\`, any of which would keep the
  result from reading as a link. A whole link (`[text](destination)`) holds both, so
  pasting over one replaces it, the way a link's destination is updated;
- `previous` is not a URL: pasting over one replaces it, the way a pasted URL is updated.

Otherwise it returns `#f` and the paste goes in unchanged. The patterns are all lowercase,
so `regex` matches them in either case, and they write `[^\s]` for `\S` for that reason.
A link paste leaves the whole link selected, so a second `p` replaces the link with the bare
URL: smart paste's repeat check compares the selection with the pasted text, and the link
differs from the URL.

## Link paths

The `markdown:paths` source answers only when the token before the cursor starts right
after `](` in a buffer with a file: it emits an empty list otherwise, which leaves the menu
to the other sources and, on a trigger character, shows nothing. Its `#:token-chars "/.-"`
make the whole relative path the token, `../` included. The token splits at its last `/`:
the part before it names a folder relative to the buffer's file, and `path-entries` lists
that folder, folders ending in `/`, as `:e` lists them; when the part after the `/` starts
with `.`, it lists the hidden entries instead. Each item inserts the folder part followed by
the name and is matched as that path, which is what the token holds. A token starting with `/` gets the empty answer: joined to the buffer's folder it
would name the filesystem root.

The source leaves the filtering to the menu's fuzzy matcher, so typing a name does not read
the folder on every keystroke. Its answer is incomplete only while the part after the last
`/` is empty: the first character typed asks it again, which a leading `.` needs to list
the hidden entries, and every later one only narrows that answer. A typed `/` asks again
through its trigger and lists the folder it ends. Erasing back over a `/` keeps the
folder's answer, whose paths still match what is typed, until a `/` or `Ctrl-Space` asks
again. Its empty answer outside a destination is complete, so typing elsewhere does not ask
it again. `(` and `/` are Markdown trigger characters for it, so `](` and a folder's `/`
open the menu without `Ctrl-Space`.
