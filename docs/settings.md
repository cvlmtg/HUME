# Settings

Settings can be changed in three ways:

1. **Steel config** (`~/.config/hume/init.scm`): `(set-option! "key" "value")` — applies as a global setting at startup.
2. **Command prompt** (`:` key): `:set global key=value` or `:set buffer key=value` — takes effect immediately.
3. **Buffer-local overrides** only affect the current buffer; global settings affect all buffers that don't have an override.

---

## Global-Only Settings

These settings can only be set globally (not overridden per-buffer).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `scroll-margin` | integer | `3` | Vertical scroll margin: number of lines to keep visible above and below the cursor |
| `scroll-margin-h` | integer | `5` | Horizontal scroll margin: number of columns to keep visible left and right of the cursor (for non-wrapping lines) |
| `mouse-scroll-lines` | integer | `3` | Number of lines to scroll per mouse wheel tick |
| `mouse-enabled` | bool | `true` | Enable mouse support |
| `mouse-select` | bool | `false` | Allow click-to-move and click-drag selection with the mouse |
| `jump-list-capacity` | integer ≥ 1 | `100` | Maximum number of entries in the jump list |
| `jump-line-threshold` | integer | `5` | Minimum line distance for a motion to be recorded as a jump-list entry |

---

## Per-Buffer Settings

These settings can be set globally (affecting all buffers) or overridden for a single buffer.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tab-width` | integer 1–255 | `4` | Width of a tab character in columns, and number of spaces inserted when pressing Tab |
| `wrap-mode` | `none` \| `indent:N` | `indent:76` | Soft line wrapping. `none` disables wrapping; `indent:N` wraps at column N with continuation lines indented to match the wrapped line's indent level |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display. `absolute`: plain line numbers. `relative`: distance from cursor. `hybrid`: absolute on the cursor line, relative elsewhere |
| `auto-pairs-enabled` | bool | `true` | Enable auto-pairs: automatically insert closing delimiters and skip over them on close |

---

## Whitespace Rendering

Controls how invisible characters are displayed. Set sub-fields independently via `:set` or `(set-option!)`.

| Key | Scope | Values | Default | Description |
|-----|-------|--------|---------|-------------|
| `whitespace-space` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render space characters |
| `whitespace-tab` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render tab characters |
| `whitespace-newline` | global / buffer | `none` \| `all` \| `trailing` | `none` | Render newline characters |

- `none`: never shown
- `all`: always shown
- `trailing`: shown only on trailing whitespace (useful for catching accidental trailing spaces)

---

## Statusline Configuration (Steel only)

The statusline is configured via `(set-statusline! section elements)` in `init.scm`. Each section is a space-separated list of element names.

```scheme
(set-statusline! "left"   "mode separator file-name dirty-indicator")
(set-statusline! "center" "")
(set-statusline! "right"  "macro-recording selections position")
```

**Sections:** `"left"`, `"center"`, `"right"`

**Available elements:**

| Element | Description |
|---------|-------------|
| `mode` | Current mode (`NORMAL`, `INSERT`, `EXTEND`) |
| `separator` | Visual divider between element groups |
| `file-name` | Name of the current file (basename only) |
| `dirty-indicator` | Shows `[+]` when the buffer has unsaved changes |
| `position` | Cursor line and column |
| `selections` | Number of active selections (hidden when just one) |
| `search-matches` | Current match index and total when a search is active |
| `mini-buf` | Contents of the mini-buffer (search prompt, command prompt) |
| `macro-recording` | Recording indicator when a macro is being captured |
| `kitty-protocol` | Shows `[kitty]` when the kitty keyboard protocol is active |

---

## Steel Scripting API

All settings and keymap changes available from `init.scm`:

### `(set-option! key value)`

Apply a global setting. Equivalent to `:set global key=value`. Validation failures are surfaced as startup warnings; the rest of the config continues loading.

```scheme
(set-option! "tab-width" "2")
(set-option! "wrap-mode" "none")
(set-option! "scroll-margin" "5")
```

### `(keymap-bind! mode key-sequence command)`

Bind a key sequence to a named command in the given mode.

- `mode`: `"normal"`, `"extend"`, or `"insert"`
- `key-sequence`: space-separated key tokens, e.g. `"g r"`, `"ctrl-k"`, `"alt-j"`
- `command`: name of any registered command

```scheme
(keymap-bind! "normal" "g r" "redo")
(keymap-bind! "normal" "ctrl-k" "move-up")
```

### `(keymap-unbind! mode key-sequence)`

Remove an existing binding.

```scheme
(keymap-unbind! "normal" "Ctrl+c")
```

### `(keymap-bind-wait-char! mode key-sequence command)`

Bind a key sequence to a wait-char node. The next keypress after the sequence is captured and made available to the command via `(pending-char)`.

```scheme
(keymap-bind-wait-char! "normal" "m r" "helix-replace-surround")
```

### `(define-command! name doc lambda)`

Register a Steel lambda as a named mappable command. The command can then be bound with `keymap-bind!` or called via `(exec ...)`.

```scheme
(define-command! "my-command" "Description shown in command help."
  (lambda ()
    (exec "move-right")
    (exec "delete")))
```

If a command with the same name is already registered, the new registration is rejected with a warning (no shadowing).

### `(exec command-name)`

Queue a named command for execution. Only valid inside a `define-command!` lambda — calling `exec` at the top level of a script is an error.

```scheme
(exec "move-right")
(exec (string-append "surround-" suffix))
```

### `(pending-char)`

Returns the wait-char argument as a single-character string. Empty string if no wait-char is pending. Only meaningful inside a command registered with `keymap-bind-wait-char!`.

```scheme
(let ((ch (pending-char)))
  (exec (string-append "find-" ch)))
```

### `(set-statusline! section elements)`

Configure a statusline section. See [Statusline Configuration](#statusline-configuration-steel-only) above.
