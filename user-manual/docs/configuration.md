# Configuration

HUME is configured via a Scheme file at:

- **macOS / Linux:** `$XDG_CONFIG_HOME/hume/init.scm` (defaults to `~/.config/hume/init.scm`)
- **Windows:** `%APPDATA%\hume\init.scm`

If the file does not exist, HUME starts with defaults. Parse errors show a warning and fall back to defaults.

## Setting options

```scheme
(set-option! "option-name" value)
```

Use `:set` from the command line for quick changes — see [Command Line](command-line.md).

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

## Per-buffer options

Set an option for the current buffer only from the command line or init.scm:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tab-width` | integer | `4` | Spaces per indent level |
| `wrap-mode` | `none` \| `soft[:N]` \| `word[:N]` \| `indent[:N]` | `indent` | Line wrapping behavior |
| `line-number-style` | `absolute` \| `relative` \| `hybrid` | `hybrid` | Line number display in the gutter |
| `auto-pairs-enabled` | bool | `#t` | Enable auto-pair insertion |
| `language` | string | *(auto-detected)* | Language for syntax highlighting |

Use `:set buffer <option>=<value>`:

```
:set buffer tab-width=2
:set buffer language=markdown
```

## Key bindings

```scheme
(bind-key! "normal" "ctrl+j" "move-down")
(bind-key! "normal" "g e" "goto-last-line")
(unbind-key! "normal" "ctrl+j")
```

`bind-key!` — binds a key in the given mode (`"normal"`, `"insert"`, `"extend"`).
`unbind-key!` — removes a binding.

## Statusline

The statusline is fully configurable from Steel:

```scheme
(configure-statusline! '("Mode" "Separator" "FileName") '() '("Position"))
```

Each argument is a list of element name strings. Available elements:

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

(bind-key! "normal" "ctrl+h" "select-prev-word")
(bind-key! "normal" "ctrl+l" "select-next-word")

(declare-plugin "username/hume-plugin-example" #:commands '("hello"))
```