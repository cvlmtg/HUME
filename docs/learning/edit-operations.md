# Edit Operations: Acting on Selections

## The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
`SelectionSet` — a struct wrapping a `Vec<Selection>` with a `primary` index
that identifies the main cursor. Selections are always
**inclusive**: `anchor == head` is a 1-char selection covering the character at
that index, not a zero-width point. Each `Selection` is either:

- **Single-character** (`anchor == head`): the cursor sits on exactly one character.
- **Multi-character** (`anchor != head`): a contiguous region of selected text.

An operation like "insert character `x`" means:

- For a **single-character selection**: insert `x` before the cursor character;
  the cursor advances to the next character.
- For a **multi-character selection**: replace the entire selected region with `x`.

This is the same rule in both cases. Single-cursor editing, visual-mode editing,
and multicursor editing all fall out of the same loop.

## Multi-selection edit ordering

A `SelectionSet` can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**:
inserting a character at offset 0 shifts every position to its right, so
naively applying edits one-by-one would corrupt subsequent offsets.

HUME avoids this entirely with `ChangeSetBuilder`: all input positions are
expressed in **original-buffer coordinates**, and the builder handles the
translation internally. See the [Changesets](changesets.md) section.

## Primary vs secondary selections

All selections are **equal for editing** — insert, delete, and motions apply
to every selection in the set simultaneously. The *primary* is just the
"focused" one. It is distinguished in four specific situations:

1. **Status bar**: shows the primary's line and column position. You can't
   display all N cursors at once — one has to be canonical.

2. **Viewport scrolling**: the editor scrolls to keep the primary visible.
   Other cursors may be off-screen — that is fine and expected.

3. **Single-selection commands**: `cmd_keep_primary_selection` (keep primary
   only) and `cmd_remove_primary_selection` (remove primary) operate on
   exactly one selection. The primary determines which one.

4. **Registers** (`editor/src/ops/register.rs`): when you yank with N cursors, the
   register stores a **list of N strings**, one per selection in document
   order. Pasting with N cursors maps each slot back to the corresponding
   cursor. If the cursor count doesn't match at paste time, the full register
   content is pasted at every cursor as a fallback.

   HUME uses mnemonic register names rather than the traditional Vim/Helix
   convention (`"`, `+`, `_`). Since 10 named registers (`0`–`9`) cover all
   real workflows, letters are freed for intuitive special names:

   | Key | Register | Notes |
   |-----|----------|-------|
   | `0`–`9` | Named storage | Text or macros; last write wins |
   | `q` | Default macro | `QQ` records, `q` replays |
   | `c` | System clipboard | Deferred to M7 |
   | `b` | Black hole | Discards writes |
   | `s` | Search | Holds last search pattern |

   The default register (receives all yanks/deletes when no register is
   named) is an internal sentinel (`'"'`) — users never type it.

   **Why not `a`–`z`?** Traditional named registers borrow letters for text
   storage, forcing special registers into punctuation (`+`, `_`). HUME flips
   this: numbers for user storage, letters for special registers.

   **Macro model (M5):** macros are stored in registers (Vim model, not
   Helix's single-slot model). `QQ` records into register `q` (the default
   macro register). `Q3` records into register `3`. `qq` replays from `q`,
   `q3` replays from `3`.

   **Why Vim-style macros over Helix-style?** Helix has a single macro slot
   (`Q` records, `Q` replays). Users complained — one slot is enough for a
   single task, but when you need two independent macros (e.g. one that
   transforms a line, another that moves between sections) you must
   re-record the first each time. HUME's register-based macros solve this
   without the full `a`–`z` namespace overhead. Ten slots (`0`–`9`) covers
   real workflows; the `q` default keeps the common case a one-key operation.

5. **Paste-as-replace** (`editor/src/ops/edit/`): In a select-then-act model, `p`/`P`
   has to handle two distinct cases:

   - **Cursor** (`anchor == head`, a fresh 1-char selection): insert the
     register contents *after* or *before* the cursor char. Same as Vim's `p`/`P`.
   - **Explicit selection** (more than 1 char, created intentionally): *replace*
     the selected text with the register contents, and return the displaced text
     to the caller so it can be written back to the register (a swap).

   The key insight is `sel.is_cursor()` — the selection state already encodes
   whether the user made an intentional selection. No separate `R` command
   needed. No `"0` register hack needed (in Vim, yanking always writes `"0`
   in addition to `"`, so after a delete you can still paste the pre-delete
   yank with `"0p`; HUME avoids the problem by never clobbering the register
   on replace).

   The return type of `paste_after`/`paste_before` is `(Buffer, SelectionSet,
   ChangeSet, Vec<String>)`. The fourth element contains the displaced text
   (empty strings for cursor pastes). The editor layer writes it back to the
   source register, completing the swap.

**Why cycle the primary?** In a keyboard-only multi-cursor world,
`cmd_cycle_primary_forward` and `cmd_cycle_primary_backward` are how you
"focus" a different cursor — to make the viewport scroll to it, read its
position in the status bar, or target it with `cmd_remove_primary_selection`.
There is no mouse click to promote a cursor; cycling is the keyboard
equivalent.

Internally, `SelectionSet.primary` is an index into the sorted
`Vec<Selection>`. The index is updated whenever the set changes: merges that
absorb the primary, removals before or at it, and splits all adjust the index
so it keeps pointing at the intended selection.
