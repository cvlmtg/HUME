# MotionMode: Separating Position from Anchor Semantics

## A concrete walkthrough

Buffer: `"hello world\n"`, cursor on `'h'` (position 0).

Before `l` is pressed, the `SelectionSet` contains one selection:

```
Selection { anchor: 0, head: 0 }   ← single-char selection on 'h'
```

Pressing `l` calls `apply_motion(buf, sels, MotionMode::Move, 1, move_right)`.

**Step 1 — inner motion function.** `move_right(buf, head=0)` returns
`next_grapheme_boundary(buf, 0)` = 1. It knows nothing about the old selection
or anchors — just a coordinate calculation.

**Step 2 — apply `MotionMode::Move`.** `apply_motion` builds the new selection:

```
Move → anchor = new_head (1), head = new_head (1)
Result: Selection { anchor: 1, head: 1 }
```

The cursor is on `'e'`, a single-character selection.

Now suppose the cursor is at `{ anchor: 2, head: 2 }` on `'l'` and the user
presses `l` in extend mode:

**Step 2 — apply `MotionMode::Extend`.**

```
Extend → anchor = old_anchor (2), head = new_head (3)
Result: Selection { anchor: 2, head: 3 }
```

The selection grew from `'l'` to cover both `'l'` characters (`"ll"`) — the
anchor stayed put.

## The two modes

| Mode | Anchor | Head | Typical use |
|------|--------|------|-------------|
| `Move`   | `new_head`   | `new_head` | Plain cursor move — `h`, `j`, `k`, `l` |
| `Extend` | `old_anchor` | `new_head` | Grow selection — extend mode, Ctrl+letter |

`Move` always produces a collapsed single-character selection (anchor == head).
`Extend` keeps the existing anchor, only moving the head.

> **Historical note:** `MotionMode` originally had a third value — `Select`,
> which set the anchor to the old *head*. This was the Kakoune model for word
> motions: `w` accumulated the traversed span from cursor to next word start.
> `Select` was removed when `w`/`b`/`W`/`B` were redesigned to select the
> whole destination word via `apply_word_select` — see [Word Motions](word-motions.md).

## Why separate the inner function from the mode

The inner function `fn(&Buffer, usize) -> usize` is a pure coordinate
calculation — it knows nothing about anchors or multi-cursor. `MotionMode` is
a concern of the keymap layer, not of the motion itself. This means:

- Adding a new motion (e.g. "next paragraph") requires one position function;
  Move and Extend variants come for free.
- Testing the motion is simple: just assert on the returned position.
- The same `move_right` inner function powers both `l` (Move) and `l` in
  extend mode (Extend) — no separate command needed.

```rust
match mode {
    MotionMode::Move   => Selection::cursor(new_head),
    MotionMode::Extend => Selection::new(sel.anchor, new_head),
}
```
