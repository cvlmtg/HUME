# Moving Around

All movement happens in Normal mode. Motions move the cursor and change the current selection.

## Basic movement

| Key | Movement |
|-----|----------|
| `h` | One character left |
| `l` | One character right |
| `j` | One line down |
| `k` | One line up |
| `w`/`W` | Next word/WORD start |
| `b`/`B` | Previous word/WORD start |

A `word` breaks at punctuation, so `don't` is three `words`. A `WORD` only breaks at spaces, so `don't` is one `WORD`.

## Character find

Search within the current line for a specific character:

| Key | Movement |
|-----|----------|
| `f` + char | Jump forward to next occurrence of char (inclusive) |
| `F` + char | Jump backward to previous occurrence of char (inclusive) |
| `t` + char | Jump forward to just before next occurrence of char (exclusive) |
| `T` + char | Jump backward to just after previous occurrence of char (exclusive) |
| `=` | Repeat last find forward |
| `-` | Repeat last find backward |

After pressing `f`, `F`, `t`, or `T`, HUME waits for the target character.

## Line movement

The idiomatic line movements live under the `g` prefix:

| Key | Movement |
|-----|----------|
| `g h` | Start of line |
| `g l` | End of line (last character) |
| `g s` | First non-whitespace character on the line |
| `Home` | Start of line |
| `End` | End of line |

For vim users, `0`, `$`, and `^` are also mapped as a convenience — one keystroke instead of two — and behave the same as `g h`, `g l`, and `g s` respectively.

## Goto prefix (`g`)

Press `g` followed by a second key for line jumps:

| Key | Movement |
|-----|----------|
| `g g` | First line of file |
| `g e` | Last line of file |
| `g h` | Start of line |
| `g l` | End of line |
| `g s` | First non-whitespace on line |

## Paragraph movement

| Key | Movement |
|-----|----------|
| `{` | Previous paragraph start |
| `}` | Next paragraph start |

A paragraph is a block of non-blank lines delimited by blank lines; `{` and `}` jump to the first line of the surrounding paragraphs.

## Scrolling

| Key | Effect |
|-----|--------|
| `PageDown` | Scroll one viewport down |
| `PageUp` | Scroll one viewport up |
| `Ctrl+d` | Scroll half a viewport down |
| `Ctrl+u` | Scroll half a viewport up |

## View prefix (`z`)

Press `z` followed by a second key to reposition the view (the cursor itself stays put):

| Key | Effect |
|-----|--------|
| `z z` | Center view on cursor |
| `z t` | Scroll cursor to top of screen |
| `z b` | Scroll cursor to bottom of screen |

## Search navigation

| Key | Effect |
|-----|--------|
| `/pattern` | Search forward |
| `?pattern` | Search backward |
| `n` | Next match |
| `N` | Previous match |
| `*` | Use the primary selection as the search pattern. When the whole selection is word-class, the pattern is wrapped in word boundaries (`\b…\b`) for whole-word search; otherwise the text is searched literally. |
| `m /` | Turn every search match in the buffer into a selection |

## Jump list

HUME maintains a jump list of recent cursor positions.

| Key | Effect |
|-----|--------|
| `Ctrl+o` | Jump to previous position |
| `Ctrl+i` / `Tab` | Jump to next position |
| `Ctrl+6` | Jump to alternate (most-recently-focused) buffer |
