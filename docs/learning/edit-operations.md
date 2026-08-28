# Edit Operations: Acting on Selections

## The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
selection set — an ordered list of selections with one marked primary. Selections
are always **inclusive**: a single-position selection covers exactly one character,
never a zero-width point. Each selection is either:

- **Single-character** (`anchor == head`): the cursor sits on exactly one character.
- **Multi-character** (`anchor != head`): a contiguous region of selected text.

An operation like "insert character `x`" means:

- For a **single-character selection**: insert `x` before the cursor character;
  the cursor advances to the next character.
- For a **multi-character selection**: replace the entire selected region with `x`.

This is the same rule in both cases. Single-cursor editing, visual-mode editing,
and multicursor editing all fall out of the same loop.

## Multi-selection edit ordering

A selection set can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**:
inserting a character at offset 0 shifts every position to its right, so
naively applying edits one-by-one would corrupt subsequent offsets.

HUME avoids this entirely with a change-set builder: all input positions are
expressed in **original-buffer coordinates**, and the builder handles the
translation internally. See the [Changesets](changesets.md) section.

## Primary vs secondary selections

All selections are **equal for editing** — insert, delete, and motions apply
to every selection in the set simultaneously. The *primary* is just the
"focused" one. It is distinguished in five specific situations:

1. **Status bar**: shows the primary's line and column position. You can't
   display all N cursors at once — one has to be canonical.

2. **Viewport scrolling**: the editor scrolls to keep the primary visible.
   Other cursors may be off-screen — that is fine and expected.

3. **Single-selection commands**: "keep primary" drops every selection except
   the primary (its extent is preserved); "remove primary" removes it and
   promotes the next one.
   Both commands operate on exactly one selection, identified by the primary.

4. **Registers**: when you yank with N cursors, the
   register stores a **list of N strings**, one per selection in document
   order. Pasting with N cursors maps each slot back to the corresponding
   cursor. If the cursor count doesn't match at paste time, the full register
   content is pasted at every cursor as a fallback.

   HUME uses mnemonic register names rather than the traditional Vim/Helix
   convention (`"`, `+`, `_`). Since 10 named registers (`0`–`9`) cover all
   real workflows, letters are freed for intuitive special names:

   | Key | Register | Notes |
   |-----|----------|-------|
   | `0`–`9` | Named storage | Text or macros; last write wins. `"5y`/`"5p` round-trip. |
   | `k` | Kill ring | `"kp` pastes ring head; `"ky`/`"kd`/`"kc` push onto ring |
   | `q` | Default macro | `QQ` records, `qq` replays |
   | `c` | System clipboard | Requires OS integration |
   | `b` | Black hole | Discards writes |
   | `s` | Search | Holds last search pattern |

   Not every name in the table works as a `"` prefix: only `0`–`9`, `k`,
   `c`, and `b` are addressable that way. `q` is driven by the `Q`/`q` keys
   themselves, and `s` is written by search — neither can be named after `"`.

   There is no single "default register" that all bare yanks and deletes flow
   through. The bare behaviour is split: a bare yank writes to the system
   clipboard *and* pushes a copy onto the kill ring; a bare delete or change
   pushes onto the kill ring only, leaving the clipboard untouched. The split
   is what lets a stray delete never overwrite cross-app text the user just
   copied, and a bare paste choose the right source via the heuristic
   described in [Kill Ring and Smart-p](kill-ring-and-smart-p.md).

   **Why not `a`–`z`?** Traditional named registers borrow letters for text
   storage, forcing special registers into punctuation (`+`, `_`). HUME flips
   this: numbers for user storage, letters for special registers.

   **Two namespaces, two purposes.** The digit slots `"0`–`"9` are the
   *deterministic, durable* namespace — for scripted, macro, or surgical use where
   you write a value and read it back verbatim later (the same slots hold macros:
   `Q5`/`q5`). The kill ring (`"k`, `[`/`]`) is the *interactive* namespace: the
   ring head is non-deterministic (every `d`/`c`/`y` shifts it), so it is
   in-the-moment reach-back, not storage.

   The kill ring (bounded history of recent captures) is accessible via `"k`
   (head paste) and `[`/`]` cycling. See [Kill Ring and Smart-p](kill-ring-and-smart-p.md).

    **Macro model:** macros are stored in registers. `QQ` records into
    register `q` (the default macro register). `Q3` records into register
    `3`. `qq` replays from `q`, `q3` replays from `3`. This is the same
    register-based model as Vim and Helix: Helix records with `Q` to a
    *selected* register (default `@`) and replays with `q`, and any named
    register can be targeted with `"<reg>Q` / `"<reg>q`, so it is not a
    single fixed slot.

    **Why register-based macros?** Storing macros in named, durable slots
    means two independent macros (e.g. one that transforms a line, another
    that moves between sections) coexist without re-recording. HUME reuses
    the same `0`–`9` namespace that already holds yanked text, avoiding the
    full `a`–`z` overhead of Vim's named registers; the `q` default keeps
    the common case a one-key operation.

5. **Paste-as-replace**: In a select-then-act model, `p`/`P`
   has to handle two distinct cases:

   - **Cursor** (`anchor == head`, a fresh 1-char selection): insert the
     register contents *after* or *before* the cursor char. Same as Vim's `p`/`P`.
   - **Explicit selection** (more than 1 char, created intentionally): *replace*
     the selected text with the register contents — unless that text is
     already exactly what's about to be pasted, in which case the selection
     collapses first and the paste lands next to it instead of over it (see
     the kill-ring article's "repeat vs. swap" for why). Displaced text is
     not written back to the register — the kill ring already holds the
     selection's history, so the user can reach it via `"kp` (head) or by
     cycling with `[`/`]`.

   The selection state already encodes whether the user made an intentional
   selection — no separate command needed. HUME avoids the Vim `"0` register
   workaround by never clobbering the ring entry on replace: the kill ring
   already holds the selection's history.

   A register's content chooses its own paste shape. Content that ends in a
   newline is *linewise*: over a cursor it inserts as new line(s) below or
   above; over a selection it replaces line-by-line and reflows what is left.
   Content without a trailing newline is *charwise*: it lands inline at the
   cursor. The distinction falls out of inspecting the yanked text — no
   separate `p` vs `P`-linewise command names it.

**Why cycle the primary?** In a keyboard-only multi-cursor world, cycling
forward and backward through primaries is how you "focus" a different cursor
— to make the viewport scroll to it, read its position in the status bar, or
remove just that cursor. There is no mouse click to promote a cursor; cycling
is the keyboard equivalent.

The primary is just a pointer into the sorted set of selections. It is updated
automatically whenever the set changes: merges, removals, and splits all adjust
the pointer so it keeps tracking the intended selection.
