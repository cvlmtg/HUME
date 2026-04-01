# The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four files, each owning one responsibility.
Understanding the split — and what each layer does *not* know — is the key to
extending the editor safely.

## The four files

| File | Role | Knows about keys? | Knows about `&mut Editor`? |
|---|---|---|---|
| `registry.rs` | Name → function pointer + extend pairing | No | No |
| `commands.rs` | Editor-level command implementations | No | Yes (via `&mut Editor` param) |
| `keymap.rs` | Key sequence → command name | Yes | No |
| `mappings.rs` | Resolve name, call function | No | Yes (`&mut self`) |

## Layer 1: Command Registry (`registry.rs`)

The registry is the single source of truth for what commands exist and how they
relate to each other. Every user-facing operation is a named `MappableCommand`
— a function pointer wrapped with a `&'static str` name. Four variants exist:

```rust
enum MappableCommand {
    Motion    { name, fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet },
    Selection { name, fun: fn(&Buffer, SelectionSet) -> SelectionSet },
    Edit      { name, fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet) },
    EditorCmd { name, fun: fn(&mut Editor, usize) },
}
```

The first three are pure functions — they take buffer/selections and return new
ones. `EditorCmd` takes `&mut Editor` for composite operations that need mode
changes, registers, undo groups, or parameterized motions.

### Extend-variant pairing

The registry also stores an **extend map**: a mapping from base command names
to their extend variants. Each command declares its extend variant at
registration time via an `extend:` argument on the registration macro:

```rust
motion!("move-left", "Move cursors one grapheme to the left.", cmd_move_left, extend: "extend-left");
motion!("extend-left", "Extend selections one grapheme to the left.", cmd_extend_left);

editor_cmd!("open-line-below", "...", cmd_open_line_below, repeatable, extend: "flip-selections");
```

This inserts `"move-left" → "extend-left"` into the extend map. Commands
without an `extend:` argument have no extend variant.

### Extend variants are independent commands

Every extend variant (e.g. `"extend-left"`, `"extend-select-line"`) is
registered as a standalone command in the registry — it has its own name, its
own function pointer, and can be looked up or invoked like any other command.
The extend map only records the *pairing* between a base command and its extend
counterpart; it does not make extend variants second-class.

This means extend variants can be **bound to any key independently**:

- A user can bind `Ctrl+x` → `"extend-select-line"` in their keymap — this
  works on any terminal, kitty or legacy, because the trie resolves the name
  directly without going through extend-mode resolution.
- Internal code can look up extend variants by name (e.g. `page_scroll` looks
  up `"extend-down"` directly in the registry).
- The automatic extend resolution (sticky extend mode, Ctrl one-shot extend) is
  a *convenience* — it lets the user press `e` then `l` instead of binding a
  separate key for every extend variant. But it's not the only way to invoke
  them.

## Layer 2: Editor Commands (`commands.rs`)

This file holds ~35 `EditorCmd` implementations as free functions:
`cmd_change`, `cmd_find_forward`, `cmd_open_line_below`, etc. Each is a
`fn(&mut Editor, usize)` registered by name in the registry. This parallels
how `ops/motion.rs` holds pure motion functions.

## Layer 3: Keymap (`keymap.rs`)

The keymap is a trie that maps key sequences to command names. Each binding
resolves to a `KeymapCommand`:

```rust
struct KeymapCommand {
    name: &'static str,
}
```

The keymap stores *names*, not function pointers, and not extend variants.
This is what the Steel scripting layer will rewrite to support user keymaps —
and it can do so without touching any execution logic. A user remap is just
`key → command-name`.

Three types of trie nodes exist:

- **Leaf**: a complete binding → dispatch the command name.
- **Interior**: more keys needed (e.g. `m` → `i` → `w` for inner-word).
- **WaitChar**: the next keypress is consumed as a character argument (f/t/F/T/r).

The `cmd!` and `wait_char!` macros construct bindings:

```rust
t.bind_leaf(key!('h'), cmd!("move-left"));
t.bind(key!('f'), wait_char!("find-forward"));
```

## Layer 4: Dispatch (`mappings.rs`)

`mappings.rs` is the glue. `execute_keymap_command` takes a command name, an
`extend: bool` flag, and a count. It resolves the extend variant if needed,
looks up the command in the registry, and calls the function pointer:

```
keypress
  → keymap.rs  trie walk  →  KeymapCommand { name: "move-right" }
  → mappings.rs            →  if extend: registry.extend_variant("move-right") → "extend-right"
  → registry.rs            →  registry.get("extend-right") → MappableCommand::Motion { fun }
  → mappings.rs            →  fun(buf, sels, count)
```

### Extend-mode resolution

When extend mode is active (`self.extend == true`), the dispatcher passes
`extend: true` to `execute_keymap_command`. The function resolves the extend
variant via the registry:

```rust
let resolved = if extend {
    self.registry.extend_variant(name).unwrap_or(name)
} else {
    name
};
```

If the command has no extend variant, the base command runs unchanged. This
means commands like `"delete"` or `"undo"` behave the same regardless of
extend mode.

### Ctrl+key one-shot extend (kitty keyboard protocol)

When the kitty keyboard protocol is enabled, `Ctrl+motion` acts as one-shot
extend. The dispatch flow:

1. Look up the full key (with modifiers) in the trie.
   - Match found → dispatch it (explicit Ctrl bindings like Ctrl+c always work).
   - No match, and Ctrl is pressed:
     - **Kitty disabled** → no-op (legacy terminals can't reliably distinguish
       Ctrl+letter from control codes).
     - **Kitty enabled** → strip Ctrl, look up bare key in trie:
       - No match → no-op.
       - Match found → check if the command has an extend variant in the registry:
         - No extend variant → no-op (e.g. Ctrl+u won't run undo).
         - Has extend variant → dispatch with `extend=true`.

**Shifted characters and `REPORT_ALTERNATE_KEYS`**: without this flag, the
kitty protocol sends the base codepoint plus modifier flags — Ctrl+} would
arrive as `Char(']')` with `SHIFT | CONTROL`, and stripping Ctrl would leave
`]` which doesn't match the trie binding for `}`. To avoid a hardcoded
keyboard-layout shift map, we enable `REPORT_ALTERNATE_KEYS` in the kitty
protocol flags at startup. This makes the terminal send the shifted character
as an alternate key; crossterm replaces the base keycode with the alternate and
strips SHIFT. So Ctrl+} arrives as `Char('}')` with just `CONTROL` — stripping
Ctrl gives us the correct bare key, and it works on any keyboard layout.

The `extend` flag is passed as a **parameter** to `execute_keymap_command` — no
temporary editor state mutation.

### WaitChar: parameterized commands

Some commands need a character argument: `f` (find), `t` (till), `r` (replace).
The trie stores these as `WaitChar` nodes:

```rust
wait_char!("find-forward")
```

When the trie walk hits a `WaitChar` node, `mappings.rs` stores the pending
command in `Editor.wait_char`. The *next* keypress is consumed as the character
argument, stored in `Editor.pending_char`, and the command is dispatched. The
command function (e.g. `cmd_find_forward` in `commands.rs`) reads the character
via `ed.pending_char.take()`.

Extend resolution for wait-char commands happens at **char-consumption time**
(not when the trigger key is pressed), since extend mode could toggle between
the two keypresses.

## Commands are mode-agnostic

Commands in the registry have no mode affinity. `"flip-selections"` is just a
name that resolves to a function pointer. If Steel binds it to a key in the
insert keymap trie, `handle_insert` walks the trie, gets a `Leaf`, and calls
`execute_keymap_command` — which calls the function. The selection flips, the
editor stays in Insert mode.

Whether that binding is *useful* is the user's responsibility. The editor
doesn't second-guess it.

## Insert mode limitations

Normal mode accumulates multi-key sequences in `pending_keys` and handles
`WaitChar` state. Insert mode does neither — its trie walk is single-key only.
This means:

- **Multi-key sequences** (e.g. `mi` for inner-word) won't work in Insert mode.
- **WaitChar commands** (e.g. `find-forward`, which needs a second keypress for
  the character argument) will silently do nothing, because Insert mode doesn't
  set `wait_char` or consume the follow-up character.
- **Simple Leaf commands** (e.g. `flip-selections`, `collapse-selection`) work
  fine if bound to a single key in the insert trie.

This is a design constraint, not a bug. Insert mode is optimised for typing —
complex command sequences belong in Normal mode. If a future need arises for
multi-key insert bindings, the `pending_keys` / `WaitChar` machinery from
`handle_normal` would need to be replicated in `handle_insert`.

## Independence of layers

The layering means any of the four files can change independently:

- **New command**: add the function in the appropriate `ops/` file or
  `commands.rs`, register it in `registry.rs`, bind a key in `keymap.rs`.
  `mappings.rs` is unchanged.
- **Rebind a key**: only touch `keymap.rs`.
- **Change dispatch** (e.g. add macro recording): only touch `mappings.rs`.
- **User keymaps via Steel**: rewrite `keymap.rs` trie entries. The registry
  and dispatch layer are unaffected.
