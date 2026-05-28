# MotionMode: Separating Position from Anchor Semantics

In most text editors, "move the cursor" and "extend the selection" are handled
by separate key bindings — arrow keys move, Shift+arrow extends. In HUME, the
two behaviours are the same command with a different mode parameter. This is
why `h` can both move the cursor (normal use) and grow a selection (in extend
mode) without needing a separate `"extend-left"` command.

## A concrete walkthrough

Buffer: `"hello world\n"`, cursor on `'h'` (position 0). The selection is a
single-character selection on `'h'`: anchor = 0, head = 0.

Pressing `l` invokes the motion framework with `MotionMode::Move`, a count of 1,
and a one-step position function (`move_right`).

**Step 1 — inner position function.** `move_right` computes the next grapheme
boundary from position 0, returning 1. It knows nothing about anchors or
multi-cursor — just a coordinate calculation.

**Step 2 — apply `MotionMode::Move`.** The framework collapses anchor and head
to the new position:

```
Move → anchor = 1, head = 1   (single-char selection on 'e')
```

Now suppose the cursor is at position 2 (on `'l'`) and the user presses `l` in
extend mode:

**Step 2 — apply `MotionMode::Extend`.** The framework keeps the old anchor
and moves only the head:

```
Extend → anchor = 2 (unchanged), head = 3
```

The selection grew from `'l'` to cover both `'l'` characters — the anchor
stayed put.

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
> whole destination word — see [Word Motions](word-motions.md).

## Why separate the inner function from the mode

The inner position function is a pure coordinate calculation — it knows
nothing about anchors or multi-cursor. `MotionMode` is a concern of the
dispatch layer, not of the motion itself. This means:

- Adding a new motion (e.g. "next paragraph") requires one position function;
  `Move` and `Extend` variants come for free.
- Testing the motion is simple: just assert on the returned position.
- The same `move_right` position function powers both `l` (Move) and `l` in
  extend mode (Extend) — no separate command needed.

The framework branches on the mode when constructing the resulting selection:

```
Move   → anchor = new_head, head = new_head   (collapsed single-char)
Extend → anchor = old_anchor, head = new_head (anchor stays, head moves)
```
