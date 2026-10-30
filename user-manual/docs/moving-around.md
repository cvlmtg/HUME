# Moving Around

Getting the cursor where you want it is most of editing. HUME's motions put the common jumps (a word, a character on this line, a matching bracket, the line you were on a minute ago) one or two keystrokes away.

Motions move the cursor and replace the current selection. In Extend mode they grow or shrink it instead; see [Extend mode](selections.md#extend-mode).

## Basic movement

| Key | Movement |
|-----|----------|
| `h` | One character left |
| `l` | One character right |
| `j` | One line down |
| `k` | One line up |
| `w`/`W` | Select next word/WORD |
| `b`/`B` | Select previous word/WORD |
| `x`/`X` | Select the current line, then the next/previous one on each press (see [Whole lines](selections.md#whole-lines)) |

A word breaks at punctuation, so `don't` is three words. A WORD breaks only at whitespace, so `don't` is one WORD. Set `word-chars` (see [Buffer options](settings.md#buffer-options)) to widen what counts as a word: adding `-` makes `foo-bar` one word instead of three.

By default, `w`/`W`/`b`/`B` also cover the whitespace *before* the destination word. When none precedes it (the first word of a line, where a leading run would be indentation, or a word right after punctuation), they cover its *trailing* whitespace instead. So deleting a word never leaves a double space behind. Use `:set global word-selects-whitespace=after` (or per buffer) to prefer the whitespace *after* the word, or `=off` to select only the bare word. See [Buffer options](settings.md#buffer-options).

<div class="key-demo">
<strong>Cursor on the first character, press <code>w</code></strong><br>
default&nbsp;&nbsp;&nbsp;Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
off&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;Lorem <span class="sel">ipsu<span class="head">m</span></span> dolor sit
</div>

## Character find

Search within the current line for a specific character:

| Key | Movement |
|-----|----------|
| `f` + char | Jump forward to next occurrence of char (inclusive) |
| `F` + char | Jump backward to previous occurrence of char (inclusive) |
| `t` + char | Jump forward to the character before the next occurrence of char (exclusive) |
| `T` + char | Jump backward to the character after the previous occurrence of char (exclusive) |
| `=` | Repeat last find forward |
| `-` | Repeat last find backward |

After `f`, `F`, `t`, or `T`, HUME waits for the target character; `Tab` counts as one.

These are jumps, not extends: the selection collapses to a single character where you land, not a span from where the cursor started:

<div class="key-demo">
<strong>Cursor on the first character, press <code>f</code> <code>i</code></strong><br>
Lorem <span class="head">i</span>psum dolor sit<br>
<br>
<strong>Cursor on the first character, press <code>t</code> <code>i</code></strong><br>
Lorem<span class="head">&nbsp;</span>ipsum dolor sit
</div>

## Line movement

The idiomatic line movements live under the `g` prefix:

| Key | Movement |
|-----|----------|
| `g g` | First line of file |
| `g e` | Last line of file |
| `g h` | Start of line |
| `g l` | End of line (last character) |
| `g s` | First non-whitespace character on the line |
| `Home` | Start of line |
| `End` | End of line |

## Matching pairs

| Key | Movement |
|-----|----------|
| `#` | Jump to the matching bracket or tag |

From an opening bracket or tag, `#` jumps to the closing one, and back again from the closing side.

::: details Where the cursor needs to be
For a bracket, `#` searches a selection that stays on one line, so it still finds `(` `)` `[` `]` `{` `}` when a motion leaves the cursor past one. For a tag, the cursor must be on the tag's own markup: its `<`, its `>`, the name, or an attribute. A count has no effect.
:::

## Structural navigation

When the language's grammar supports it (see [Syntax Highlighting](syntax-highlighting.md)), these commands jump to the next or previous instance of a structural kind and select all of it, with the cursor (head) on its first character:

| Key | Command | Selects |
|-----|---------|---------|
| `g f` / `g F` | `goto-next-function` / `goto-prev-function` | The next/previous function |
| `g t` / `g T` | `goto-next-class` / `goto-prev-class` | The next/previous class or type |
| `g a` / `g A` | `goto-next-argument` / `goto-prev-argument` | The next/previous argument |
| `g c` / `g C` | `goto-next-comment` / `goto-prev-comment` | The next/previous comment |
| `g u` / `g U` | `goto-next-test` / `goto-prev-test` | The next/previous unit test |
| `g v` / `g V` | `goto-next-value` / `goto-prev-value` | The next/previous array/tuple/struct value |
| `g x` / `g X` | `goto-next-tag` / `goto-prev-tag` | The next/previous tag pair |

Lowercase jumps forward, uppercase backward, and each kind uses the same letter as its [text object](selections.md) key. They stop at either end of the buffer instead of wrapping. Each one records a jump-list entry, so `Ctrl-o` returns to where you jumped from. Without a matching grammar, `g x`/`g X` find tags with a lexical scan, and the others do nothing, with no message.

## Jump to a line by number

Type `:` then a line number and press `Enter` to jump there:

| Command | Movement |
|---------|----------|
| `:<n>` | Jump to line `<n>` (e.g. `:42`) |
| `:goto <n>` | Same, full form |

Line numbers are 1-based. A number past the end of the file lands on the last line. The jump is recorded, so `Ctrl-o` brings you back.

## Paragraph movement

| Key | Selects |
|-----|---------|
| `{` | The previous paragraph |
| `}` | The next paragraph |

A paragraph is a block of non-empty lines between empty lines. Both keys select the whole paragraph they land on, plus the gap of empty lines below it, if any. The jump is recorded, so `Ctrl-o` brings you back. Pressing `{`/`}` past the first/last paragraph does nothing. See [Text objects](selections.md#text-objects) for `m i p`, which selects the paragraph without moving.

## Scrolling

| Key | Effect |
|-----|--------|
| `PageDown` | Scroll one viewport down |
| `PageUp` | Scroll one viewport up |
| `Ctrl-d` | Scroll half a viewport down |
| `Ctrl-u` | Scroll half a viewport up |

## View prefix (`z`)

Press `z` then a second key to reposition the view; the cursor itself stays put:

| Key | Effect |
|-----|--------|
| `z k` | Scroll cursor to top of screen |
| `z z` | Center view on cursor |
| `z j` | Scroll cursor to bottom of screen |

As with the motion keys, `k` is up and `j` is down.

## Search navigation

| Key | Effect |
|-----|--------|
| `/pattern` | Search forward |
| `?pattern` | Search backward |
| `n` | Next match |
| `N` | Previous match |
| `*` | Select the word under the cursor and search for it as a whole word, ignoring any current selection. Does nothing on whitespace or a blank line |
| `m /` | Turn every search match in the buffer into a selection |
| `Ctrl-/` | Search for the primary selection's text, literally (kitty only). Does nothing when the selection is only a line ending |

### `m /` precondition

`m /` uses the buffer's live search pattern if one is active, or else the search register `s` (the last pattern submitted to `/` or `?`, or set by `*` or `Ctrl-/`). With neither, it does nothing: no error, and the selections stay unchanged. It runs from Normal or Extend mode. See [Register prefix](copy-and-paste.md#register-prefix) for the full list of registers.

### Regex syntax

`/`, `?`, `s` (sift-within), and `*` all use [Rust regex](https://docs.rs/regex) syntax. Notable points:

- **Smart case.** A pattern with no uppercase letter matches case-insensitively; one uppercase letter anywhere makes it case-sensitive. This looks at the raw pattern text, so an escape like `\W` or `\S` counts as uppercase and makes the search case-sensitive. Override with `(?i)` or `(?-i)`.
- **Anchors.** At `/` and `?`, `^` and `$` match at the start and end of the buffer. At `s` they match at the start and end of each selection; see [Sift within](selections.md#sift-within-s).
- **Other inline flags**: `(?m)` multiline `^`/`$`, `(?s)` dot-matches-newline, `(?x)` extended (whitespace ignored), `(?U)` swap greedy/non-greedy.
- **`.` does not match newlines** unless you add `(?s)`.
- **No backreferences, no lookaround, no possessive quantifiers.** `\c` and `\C` are not case toggles: a pattern containing them is invalid.
- **Invalid patterns** do nothing during live preview: the cursor stays put. Pressing `Enter` on one reports it and leaves your last search as it was. Pressing `Enter` on a pattern that matches nothing shows "no match"; the pattern still becomes the current search.

### Flags

Start a pattern typed at `/`, `?`, or `s` with one or more of these letters followed by `/` to change how it's matched:

| Flag | Effect |
|------|--------|
| `m` | Move (or extend) every selection to its own next match, instead of only the primary. No effect at `s`, which already applies to every selection. |
| `v` | Match the pattern literally: none of its characters are treated as regex syntax. |

Flags combine in any order: `mv/.rs` moves every selection to its own next literal `.rs`. `n` and `N` keep repeating with whichever flags were last used, until you search again.

A pattern that starts with a `/` of its own (a path, say) is read as plain text, not as an empty flag prefix: `/usr/bin` searches for `/usr/bin`. Only text starting with `m` and/or `v` followed by `/` is read as flags. To search for such text literally, prefix it with `v/`: `v/m/s` searches for the literal text `m/s`.

## Search and replace

HUME has no substitute command. Replace with multiple selections instead:

1. Search for the pattern with `/foo` (or `*` on a word to match it).
2. Press `m /` to turn every match in the buffer into a selection.
3. Press `c` and type the replacement once: every selected instance changes together.
4. `Esc` returns you to Normal.

To replace within one region instead of the whole buffer, select the region first (e.g. `x` for a line, or `m i {` for a block), then use `s` (sift-within) with a regex instead of `m /`.

## Jump list

HUME keeps a jump list of recent cursor positions.

| Key | Effect |
|-----|--------|
| `Ctrl-o` | Jump to previous position |
| `Ctrl-i` | Jump to next position |
| `Tab` | Jump to next position (except under the kitty protocol, where `Tab` moves between panes) |

To get back to the buffer you were last in, use `:b #`. `:e #` does the same for a buffer that has a file name.
