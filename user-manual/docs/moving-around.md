# Moving Around

Getting the cursor where you want it is most of editing. HUME's motions are built so the common jumps — a word, a character on this line, a matching bracket, the line you were on a minute ago — are one or two keystrokes away.

All movement happens in Normal mode. Motions move the cursor and change the current selection.

## Basic movement

| Key | Movement |
|-----|----------|
| `h` | One character left |
| `l` | One character right |
| `j` | One line down |
| `k` | One line up |
| `w`/`W` | Select next word/WORD |
| `b`/`B` | Select previous word/WORD |

A `word` breaks at punctuation, so `don't` is three `words`. A `WORD` only breaks at spaces, so `don't` is one `WORD`.

By default, `w`/`W`/`b`/`B` also cover the whitespace *before* the destination word — except the first word of a line, which takes its *trailing* whitespace instead, since a leading run there would be indentation. This means deleting a word never leaves a double space behind. Turn it off with `:set global word-selects-whitespace=false` (or per buffer) to select just the bare word instead — see [Configuration](configuration.md).

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

## Goto prefix (`g`)

Press `g` followed by a second key for line jumps:

| Key | Movement |
|-----|----------|
| `g g` | First line of file |
| `g e` | Last line of file |
| `g h` | Start of line |
| `g l` | End of line |
| `g s` | First non-whitespace on line |

## Jump to a line by number

Type `:` then a line number and press `Enter` to jump there:

| Command | Movement |
|---------|----------|
| `:<n>` | Jump to line `<n>` (e.g. `:42`) |
| `:goto <n>` | Same, full form |

Line numbers are 1-based. A number past the end of the file lands on the last line. The jump is recorded, so `Ctrl+o` brings you back.

## Paragraph movement

| Key | Movement |
|-----|----------|
| `{` | Up to the blank line above this paragraph |
| `}` | Down to the start of the next paragraph |

A paragraph is a block of non-blank lines delimited by blank lines. The two keys aren't quite mirror images: `}` lands on the first line of the next paragraph, while `{` lands in the blank gap above the current one — at the top of that gap when it's several lines deep.

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
| `*` | Search the whole word under the cursor, ignoring any current selection. Words are wrapped in word boundaries (`\b…\b`); punctuation is searched literally. Does nothing on whitespace or a blank line |
| `m /` | Turn every search match in the buffer into a selection |
| `Ctrl+/` | Use the primary selection's text, literally, as the search pattern — no word expansion, no boundaries (kitty only). Does nothing when the selection is just a line ending |

### `m /` precondition

`m /` uses the buffer's live search pattern if one is active; otherwise it falls back to the search register `s` (the last pattern submitted to `/` or `?`, or set by `*`). If neither is available it is a **silent no-op** — no error, selections unchanged. It runs from Normal or Extend mode. See [Register prefix](editing.md#register-prefix) for the full list of registers.

### Regex syntax

`/`, `?`, `s` (select-within), and `*` all use [Rust regex](https://docs.rs/regex) syntax. Notable points:

- **Smart case.** A pattern with no uppercase letter matches case-insensitively; a single uppercase letter anywhere makes it case-sensitive. Note that this looks at the raw pattern text, so an escape like `\W` or `\S` counts as uppercase and will quietly make the search case-sensitive. Override with `(?i)` or `(?-i)`.
- **Other inline flags** — `(?m)` multiline `^`/`$`, `(?s)` dot-matches-newline, `(?x)` extended (whitespace ignored), `(?U)` swap greedy/non-greedy.
- **`.` does not match newlines** by default; use `(?s)` if you need it to.
- **No backreferences, no lookaround, no possessive quantifiers.** No Vim-style `\c` / `\C` case toggles either — they'd match the literal letters `c` / `C`.
- **Invalid patterns** do nothing during live preview: the cursor stays put. Pressing `Enter` on one still stores it, so `n` and `N` will do nothing until you search for something valid again.

## Search and replace

HUME has no `:s/foo/bar/g` substitute command. Find-and-replace is done with multi-cursor selection:

1. Search for the pattern with `/foo` (or `*` on a word to match it).
2. Press `m /` to turn every match in the buffer into a selection.
3. Press `c` to change them all at once — type the replacement once and every selected instance updates together.
4. `Esc` returns you to Normal.

To replace within a single region instead of the whole buffer, select the region first (e.g. `x` for a line, or `m i {` for a block), then use `s` (select-within) with a regex instead of `m /`.

## Jump list

HUME maintains a jump list of recent cursor positions.

| Key | Effect |
|-----|--------|
| `Ctrl+o` | Jump to previous position |
| `Ctrl+i` | Jump to next position |
| `Tab` | Jump to next position (except under the kitty protocol, where `Tab` moves between panes) |

To get back to the buffer you were last in, use `:b #`. `:e #` does the same but only for buffers that have a file on disk.
