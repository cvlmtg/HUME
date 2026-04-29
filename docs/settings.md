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
| `scrolloff` | integer | `3` | Number of lines to keep visible above and below the cursor |
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

The statusline is configured via `(configure-statusline! left center right)` in `init.scm`. Each argument is a quoted list of element name strings.

```scheme
(configure-statusline!
  '("Mode" "Separator" "FileName" "DirtyIndicator")
  '()
  '("MacroRecording" "Selections" "Position"))
```

**Available elements (PascalCase strings):**

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode (`NORMAL`, `INSERT`, `EXTEND`) |
| `"Separator"` | Visual divider between element groups |
| `"FileName"` | Name of the current file (basename only) |
| `"DirtyIndicator"` | Shows `[+]` when the buffer has unsaved changes |
| `"Position"` | Cursor line and column |
| `"Selections"` | Number of active selections (hidden when just one) |
| `"SearchMatches"` | Current match index and total when a search is active |
| `"MiniBuf"` | Contents of the mini-buffer (search prompt, command prompt) |
| `"MacroRecording"` | Recording indicator when a macro is being captured |
| `"KittyProtocol"` | Shows `[kitty]` when the kitty keyboard protocol is active |

---

## Steel Scripting API

All settings and keymap changes available from `init.scm`:

### `(set-option! key value)`

Apply a global setting. Equivalent to `:set global key=value`. Validation failures are surfaced as startup warnings; the rest of the config continues loading.

```scheme
(set-option! "tab-width" "2")
(set-option! "wrap-mode" "none")
(set-option! "scrolloff" "5")
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

Register a Steel lambda as a named mappable command. The command can then be bound with `keymap-bind!` or invoked via `(call! ...)`.

```scheme
(define-command! "my-command" "Description shown in command help."
  (lambda ()
    (call! "move-right")
    (call! "delete")))
```

If a command with the same name is already registered, the new registration is rejected with a warning (no shadowing).

### `(call! command-name)`

Queue a named command for execution. `call-command!` is a back-compat alias; prefer `call!`.

```scheme
(call! "move-right")
(call! (string-append "surround-" suffix))
```

### `(pending-char)`

Returns the wait-char argument as a single-character string. Empty string if no wait-char is pending. Only meaningful inside a command registered with `keymap-bind-wait-char!`.

```scheme
(let ((ch (pending-char)))
  (call! (string-append "find-" ch)))
```

### `(configure-statusline! left center right)`

Configure all three statusline sections in one call. See [Statusline Configuration](#statusline-configuration-steel-only) above.
