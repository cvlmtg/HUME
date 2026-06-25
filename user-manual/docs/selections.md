# Selections

Selections are central to how HUME works. Every editing command acts on the current selection — there is no cursor-without-selection. Even a single-character "cursor position" is a one-character selection.

## How selections work

A selection has two ends: the **anchor** and the **head**. The head is the moving end; the anchor stays fixed until you reset the selection. The selection always covers at least one character.

## Building a selection

### Extend mode

Press `e` to enter Extend mode. In Extend mode, every motion grows the selection instead of moving it. Press `e` again or `Esc` to return to Normal. The status bar shows `EXT` while Extend mode is active.

You can also do a one-shot extend without entering Extend mode: under the kitty keyboard protocol, `Ctrl+h`/`Ctrl+j`/`Ctrl+k`/`Ctrl+l`/`Ctrl+w`/`Ctrl+b` run the corresponding motion with extend on for that single keypress. (`Ctrl+x` / `Ctrl+X` likewise extend line selection on any terminal.)

### Text objects

Text objects select structured regions in one step. They use the `m` prefix — `m i` for inner content, `m a` for around (including delimiters or surrounding whitespace):

| Sequence | Selects |
|----------|---------|
| `m i w` / `m a w` | Inner word / word + surrounding whitespace |
| `m i W` / `m a W` | Inner WORD / WORD + surrounding whitespace |
| `m i (` / `m a (` | Inside `()` / including `()` |
| `m i [` / `m a [` | Inside `[]` / including `[]` |
| `m i {` / `m a {` | Inside `{}` / including `{}` |
| `m i <` / `m a <` | Inside `<>` / including `<>` |
| `m i "` / `m a "` | Inside `"…"` / including `"…"` |
| `m i '` / `m a '` | Inside `'…'` / including `'…'` |
| `` m i ` `` / `` m a ` `` | Inside `` `…` `` / including `` `…` `` |
| `m i a` / `m a a` | Argument (trimmed) / argument + separator comma |
| `m i l` / `m a l` | Line content (no newline) / full line (with newline) |

`i` (inner) selects content without the delimiters. `a` (around) includes the delimiters.

Two shortcuts select the inner word directly:

| Key | Effect |
|-----|--------|
| `m m` | Inner word (same as `m i w`) |
| `M M` | Inner WORD (same as `m i W`) |

There is no paragraph text object; use the `{` and `}` paragraph motions.

## Select all

| Key | Effect |
|-----|--------|
| `%` | Select entire buffer |

## Flipping and collapsing the selection

| Key | Effect |
|-----|--------|
| `;` | Collapse selection to head and exit Extend mode |
| `Ctrl+;` | Collapse selection to anchor and exit Extend mode (kitty only) |
| `Ctrl+e` | Swap anchor and head of each selection (any mode; works on legacy terminals too) |
| `o` (in Extend mode) | Flip which end is the head |

## Multiple selections

HUME supports multiple simultaneous selections. Each selection behaves independently — editing commands act on all of them at once.

| Action | Key | Effect |
|--------|-----|--------|
| Select within selection | `s` | Enter a regex pattern; each selection is filtered to its sub-matches |
| Split on newlines | `S` | Split multi-line selections into one selection per line |
| Copy to next line | `C` | Duplicate each selection to the same character column on the line below, adding a multi-cursor. No text is copied — the new selections are empty cursors at the same column. Repeating `C` stacks cursors line by line for column-style editing. HUME has no rectangular/visual-block selection primitive. |
| Trim whitespace | `_` | Remove leading/trailing whitespace from all selections |
| Keep primary | `,` | Remove all selections except the primary |
| Remove primary | `Ctrl+,` | Remove the primary selection, promote next (kitty only) |
| Cycle primary forward | `)` | Make the next selection the primary |
| Cycle primary backward | `(` | Make the previous selection the primary |

### Select within (`s`)

Press `s` to enter Select mode. Type a regex pattern and press `Enter`. Each existing selection is filtered to only the sub-ranges matching the pattern, creating one new selection per match. This is useful for splitting a line selection into individual tokens:

1. Select a line (`x`)
2. Press `s` and type `\w+` to select each word individually
3. Press `d` to delete all words at once

`s` requires at least one non-collapsed selection — on a bare single-character cursor it is a silent no-op. See [Regex syntax](moving-around.md#regex-syntax) for the pattern flavor and case-sensitivity rules.