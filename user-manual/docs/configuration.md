# Configuration

HUME is configured via a Scheme file at:

- **macOS / Linux:** `$XDG_CONFIG_HOME/hume/init.scm` (defaults to `~/.config/hume/init.scm`)
- **Windows:** `%APPDATA%\hume\init.scm`

If the file does not exist, HUME starts with defaults. Parse errors show a warning and fall back to defaults. A bundled reference config ships at `runtime/init.scm.example` (inside the runtime directory — see [File locations](#file-locations)); HUME never auto-copies it, so copy it to the path above manually if you want a starting point.

## Setting options

```scheme
(set-option! "option-name" value)
```

Use `:set` from the command line for quick changes — see [Commands](commands.md).

## Global options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `theme` | string | `""` (built-in dark) | Active color theme name |
| `scrolloff` | integer | `3` | Minimum lines kept above/below cursor |
| `mouse-enabled` | bool | `#t` | Enable mouse support |
| `mouse-scroll-lines` | integer | `3` | Lines per mouse scroll tick |
| `mouse-select` | bool | `#f` | Mouse drag creates selections |
| `jump-list-capacity` | integer ≥ 1 | `100` | Max jump list entries |
| `jump-line-threshold` | integer | `5` | Line distance to record a jump |
| `history-capacity` | integer ≥ 1 | `100` | Undo tree capacity |
| `steel-init-budget-ms` | integer ≥ 1 | `10000` | Max init.scm evaluation time (ms) |
| `steel-command-budget-ms` | integer ≥ 1 | `1000` | Max Steel command evaluation time (ms) |
| `popup-border` | bool | `#t` | Show popup borders |
| `syntax-highlight-max-bytes` | integer ≥ 1 | `1048576` | Max bytes for syntax highlighting |

## Per-buffer options (with global default)

These options have a global default (set via `set-option!` at init.scm time, or `:set global <option>=<value>`) that new buffers inherit, and a per-buffer override (set via `:set buffer <option>=<value>`). The per-buffer override takes precedence when present.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tab-width` | integer | `4` | Spaces per indent level |
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping behavior |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `auto-pairs-enabled` | bool | `#t` | Enable auto-pair insertion |
| `language` | string | *(auto-detected)* | Language for syntax highlighting |

Use `:set buffer <option>=<value>` to override for the current buffer:

```
:set buffer tab-width=2
:set buffer language=markdown
```

Or set the global default from `init.scm`:

```scheme
(set-option! "line-number-style" "absolute")
(set-option! "tab-width" 2)
```

## Key bindings

```scheme
(bind-key! "normal" "ctrl-j" "move-down")
(bind-key! "normal" "g e" "goto-last-line")
(unbind-key! "normal" "ctrl-j")
```

`bind-key!` — binds a key in the given mode (`"normal"`, `"insert"`, `"extend"`).
`unbind-key!` — removes a binding.

### Key-string grammar

A key string is a **whitespace-separated** list of tokens. Each token is `[modifier-]*key` where the modifier separator is a **dash** `-` (not `+`):

| Component | Values |
|-----------|--------|
| Modifiers | `ctrl-`, `shift-`, `alt-` (case-insensitive, repeatable, any order) |
| Named keys | `space`, `tab`, `enter` / `return` / `cr` / `ret`, `esc` / `escape`, `lt` (`<`), `backspace` / `bs`, `delete` / `del`, `insert` / `ins`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `f1`–`f12` |
| Single char | Any single Unicode character; case is preserved (`"G"` and `"g"` are distinct) |

Multi-key sequences are space-separated: `"g e"`, `"m i w"`, `"ctrl-p h"`. Examples: `"ctrl-j"`, `"shift-tab"` (becomes `BackTab`), `"ctrl-shift-left"`, `"g e"`.

## Statusline

The statusline is fully configurable from Steel:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

Each argument is a list of element name strings: left, center, right.

**Default layout** (used when no `configure-statusline!` call is in your `init.scm`):

```
Position  FilePath  Language  ReadOnly  DirtyIndicator      MacroRecording  SearchMatches  KittyProtocol  │  Mode
└──────────────────── left ────────────────────────┘      └──────────────── right ───────────────────────┘
```

The mode label lives on the **right**, not the left.

Available elements:

| Element | Description |
|---------|-------------|
| `"Mode"` | Current mode label (`NOR`/`INS`/`EXT`/`CMD`/`SRC`/`SEL`) |
| `"Separator"` | Divider between sections |
| `"FileName"` | Current buffer filename (basename) |
| `"FilePath"` | Full path of current buffer |
| `"Cwd"` | Working directory |
| `"Position"` | Line and column position |
| `"Selections"` | Number of active selections |
| `"KittyProtocol"` | Kitty keyboard protocol indicator |
| `"DirtyIndicator"` | `[+]` when buffer has unsaved changes |
| `"LineEnding"` | Line ending type (LF/CRLF) |
| `"SearchMatches"` | Current search match count |
| `"MiniBuf"` | Pending key sequence hint |
| `"MacroRecording"` | Macro recording indicator |
| `"Language"` | Buffer language |
| `"ReadOnly"` | `[RO]` indicator |

## Themes

```scheme
(set-option! "theme" "ember")
```

Built-in themes: `dark`, `light`, `ember`, `gruvbox`.

Custom themes are TOML files placed in the `themes/` subdirectory of your HUME config directory.

## Language detection

HUME detects file languages from extension, glob pattern, or shebang line. Define custom languages from Steel:

```scheme
(define-language! "my-lang"
  #:extensions '(".myl")
  #:glob "*.my"
  #:shebangs '("myinterpreter"))
```

The definition registers the language and associates it with tree-sitter grammars installed via PLUM:

```
# open a file that triggers the my-lang language, then:
:plum-install-grammar
```

Hooks can trigger on language detection:

```scheme
(declare-plugin "my-plugin" #:events '(on-language-set))
```

## Example init.scm

```scheme
(set-option! "theme" "ember")
(set-option! "line-number-style" "absolute")
(set-option! "tab-width" 2)
(set-option! "scrolloff" 8)

(bind-key! "normal" "ctrl-h" "select-prev-word")
(bind-key! "normal" "ctrl-l" "select-next-word")

(declare-plugin "username/hume-plugin-example" #:commands '("hello"))
```

## File locations

HUME resolves its directories per OS:

| Path | macOS / Linux | Windows |
|------|---------------|---------|
| Config dir (`init.scm`, user `themes/`) | `$XDG_CONFIG_HOME/hume/` (default `~/.config/hume/`) | `%APPDATA%\hume\` |
| Data dir (plugin clones, tree-sitter grammars) | `$XDG_DATA_HOME/hume/` (default `~/.local/share/hume/`) | `%LOCALAPPDATA%\hume\` (fallback `%APPDATA%\hume\`) |
| Runtime dir (bundled `runtime/`: `tutor.txt`, `themes/`, `init.scm.example`, core plugins) | `$HUME_RUNTIME` if set; else `../share/hume/` relative to the binary; else `./runtime` in dev | `$HUME_RUNTIME` if set; else the binary's directory; else `./runtime` in dev |

Notable subpaths inside the data dir: `data/plugins/` (PLUM-managed plugin clones), `data/grammars/` and `data/grammars/sources/` (compiled and source tree-sitter grammars). Plugin sandboxed filesystem operations are restricted to `data/plugins/`, `data/grammars/`, and `runtime/plugins/` (read-only).

Note: on macOS HUME follows the XDG convention (`~/.config/hume/`, `~/.local/share/hume/`) rather than `~/Library/Application Support/`.