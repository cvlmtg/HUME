# core:colorizer

<img src="/core_colorizer.png" alt="core:colorizer in action" />

Paints color literals with the color they name: hex colors of three, four, six or eight digits (`#f00`, `#ff0000cc`), `rgb()`, `rgba()`, `hsl()`, `hsla()` and `oklch()` calls, and, in CSS, SCSS and Less files, color names such as `rebeccapurple`. By default a literal's background takes its color and its text turns black or white, whichever reads.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:colorizer")
```

Requires `core:stdlib` loaded first. It starts when you open a file.

A literal's alpha is ignored. An `oklch()` color outside what the screen can show is painted with the nearest color it can, as browsers do. `hwb()`, `lab()`, `lch()`, `oklab()` and `color()` are not painted, nor a call whose arguments use `var()`, `calc()` or a relative color, nor a call written across several lines.

A `#` followed by hex digits is painted when no ASCII letter, digit, `_`, `-` or `&` comes right before it, and no ASCII letter, digit, `_` or `-` comes right after the digits. So a CSS id selector spelled in hex digits, such as `#bad {`, is painted too, and `div#bad` is not.

Outside the languages in `'named-languages`, a three- or four-digit `#` with no letter, such as the issue number `#123`, is not painted.

Only the lines on screen are painted. After you scroll, the lines that came into view appear unpainted until `hook-viewport-debounce-ms` after you stop; lower it in [Settings](../settings.md#global-options) to shorten the wait.

To keep painting from slowing the editor down, one render reads at most `'max-render-chars` characters of the lines on screen, from the top. A line that does not fit what is left stays unpainted. At the default of 7,100 characters, a full-screen 200x60 terminal of lines up to 120 characters paints whole. A screen of longer lines, such as minified CSS, loses its last lines.

A session can style about 65,000 distinct colors, a budget it shares with themes, grammars and other plugins. Once it runs out, a color not painted before in the session stays unpainted, and a warning says so once.

| Command | Effect |
|---------|--------|
| `:toggle-colorizer` | Turn painting off for the current buffer, or back on |
| `colorizer/toggle` | The same, as an editor command you can bind to a key |

Configure with `#:config`:

```scheme
(load-plugin! "core:colorizer"
  #:config (hash 'mode 'foreground 'named-languages '("css" "scss" "less" "html")))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'enabled` | boolean | `#t` | Whether painting starts on in each buffer |
| `'mode` | `'background` or `'foreground` | `'background` | `'background` paints the literal's background with its color; `'foreground` paints its text |
| `'named-languages` | list of strings | `'("css" "scss" "less")` | Languages whose files also paint color names, and paint a `#123` with no letter |
| `'max-render-chars` | integer (≥ 1) | `7100` | How many characters one render reads at most |
