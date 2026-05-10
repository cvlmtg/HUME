# The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four layers, each owning one
responsibility. Understanding the split — and what each layer does *not* know —
is the key to extending the editor safely.

## The four layers

| Layer | Role | Knows keys? | Knows editor state? |
|-------|------|-------------|---------------------|
| **Registry** | Name → command lookup | No | No |
| **Commands** | What each command does | No | Yes |
| **Keymap** | Key sequence → command name | Yes | No |
| **Dispatch** | Resolve name, call command | No | Yes |

The keymap layer knows about keys but not about what any command does. The
command layer knows what each command does but not which key triggered it. The
registry bridges them by name. The dispatch layer glues everything together.

## Layer 1: The Registry

Every user-facing operation is a named command — a name string paired with a
function and some metadata. The registry is the single source of truth for what
commands exist.

Commands come in a few varieties:

- **Motion** commands move the cursor by returning a new position. They are
  pure: they take the current buffer and cursor state and return new cursor
  state, with no side effects.
- **Selection** commands similarly return a new selection state without
  modifying the buffer.
- **Edit** commands return a new buffer *and* new cursor state, plus a
  changeset (the description of what changed, used for undo).
- **Editor-level** commands have access to the full editor state: mode
  changes, registers, undo groups, named selections. Used for operations that
  don't fit the pure model.
- **Script-backed** commands are defined in Steel (HUME's scripting language)
  and stored by the procedure name that handles them.

The first three variants are pure functions — they cannot touch anything
outside their inputs. This makes them trivially testable and composable.

### Extend mode is a runtime parameter, not separate commands

There is no `"extend-left"` or `"extend-select-word"` command. Each base
command (e.g. `"move-left"`, `"select-word"`) receives a `Move`/`Extend`
parameter at dispatch time when the user is in extend mode. The command
branches internally:

- `Move` mode: the anchor collapses to the new cursor position (a fresh
  one-character selection).
- `Extend` mode: the anchor stays fixed, only the head moves.

This means adding a new motion requires **one function and one registration**
— the extend variant comes for free from the parameter.

### The extendable flag

Some commands should *always* extend when invoked via a Ctrl+key shortcut —
they declare themselves extendable at registration time. Commands that should
not extend (like undo or quit) carry the flag as false.

For Steel-defined commands, use `(define-command-extend! …)` to opt in. Use
this for composite commands whose last step is a motion or selection.

### Typed commands (`:` commands)

The registry also holds commands invoked from the `:` command line. They share
the same name namespace as key-bound commands, which prevents collisions and
provides a single source for `:help`.

## Layer 2: Commands

The command implementations are plain functions: they take the buffer, the
current selections, and a count, and return new state. No knowledge of keys,
modes, or how they were invoked.

## Layer 3: The Keymap

The keymap is a trie — a tree structure where each node represents one key in a
sequence — that maps key sequences to command names. The stored value at each
leaf is just a name string, not a function pointer.

This separation is deliberate: the Steel scripting layer rewrites keymap entries
to support user-defined keymaps. A user remap is just `key → command-name`.
The trie can be rewritten without touching any execution logic.

Three types of trie nodes:

- **Leaf**: a complete binding → dispatch the named command.
- **Interior**: more keys needed (e.g. `m` → `i` → `w` for inner-word).
- **WaitChar**: the next keypress is consumed as a character argument
  (used by find, replace, etc.).

### Three keymaps

HUME maintains three separate tries:

| Trie | Purpose |
|------|---------|
| Normal | Main keymap for Normal mode |
| Extend | Sparse overrides for Extend mode (checked first) |
| Insert | Single-key bindings for Insert mode |

The **extend trie** is intentionally sparse — by default it only overrides `o`
(which flips anchor and head, mirroring Helix/Kakoune's visual `o`). Any key
not found in the extend trie falls through to the normal trie with extend mode
active, which applies `Extend` semantics automatically.

This lets Steel customise per-key extend-mode overrides: "when in extend mode
and the user presses this key, run this specific command instead of the usual
one."

## Layer 4: Dispatch

The dispatcher is the glue. It receives a command name and an extend flag,
converts those to the appropriate mode parameter, looks up the command in the
registry, and calls it.

The flow on any keypress:

```
keypress
  → keymap trie walk     → a command name (e.g. "move-right")
  → dispatcher           → converts extend flag to Move/Extend mode
  → registry lookup      → the function for "move-right"
  → call                 → function(buffer, selections, count, mode)
```

### Three ways to get Extend mode

**1. Sticky extend mode.** The user presses `e` to enter Extend mode. All
subsequent commands run with Extend semantics until the mode is exited. The
extend trie is checked first for per-key overrides.

**2. Ctrl+key one-shot extend (kitty keyboard protocol).** When kitty protocol
is active, pressing `Ctrl+l` strips the Control modifier, looks up `l` in the
normal trie (`"move-right"`), and dispatches with Extend mode. Works only on
kitty-capable terminals; silently absent on legacy terminals.

**3. Explicit force-extend bindings.** Some keybindings are declared to always
extend — for example, `Ctrl+x` always accumulates line selections, even without
sticky extend mode or kitty. This works on any terminal.

To remap a command with its extend behaviour to a different key:

```scheme
(bind-key! 'normal "f"   "select-line")  ; Move mode
(bind-key! 'normal "C-f" "select-line")  ; Extend mode (automatic for Ctrl+letter)
```

The user only writes the base command name. Extend semantics come from the
dispatch layer automatically.

### WaitChar: parameterized commands

Some commands need a character argument: find-forward, find-backward, replace.
The keymap stores these as *WaitChar* nodes.

When the trie walk hits a WaitChar node, the dispatcher saves the command name
and waits. The next keypress is consumed as the character argument, and then
the command runs with that character. Extend mode is resolved at the moment the
character arrives — not at the moment the trigger key is pressed.

## Commands are mode-agnostic

Commands in the registry have no mode affinity. If Steel binds `"flip-selections"`
to a key in the insert keymap, and the insert handler resolves a leaf, it calls
the same dispatch path — the selection flips, the editor stays in Insert mode.
Whether that binding is useful is the user's responsibility. The editor doesn't
second-guess it.

## Insert mode limitations

Insert mode's keymap walk is single-key only. Multi-key sequences and WaitChar
commands won't work there — insert mode is optimised for typing, not for
command choreography. Single-key leaf commands bind fine.

## Independence of layers

The value of the split is that any layer can change without touching the others:

- **New command**: add the function, register it, bind a key. The dispatch
  layer is unchanged.
- **Rebind a key**: only touch the keymap.
- **Change dispatch** (e.g. add macro recording): only touch the dispatcher.
- **User keymaps via Steel**: rewrite keymap trie entries. The registry and
  dispatch layer are unaffected.

A change in one layer cannot corrupt another because they communicate only
through name strings.
