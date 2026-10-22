# Installation

HUME ships as one archive: the editor plus the themes, plugins and language definitions it reads at runtime. Download it, extract it, run it, and you have an editor. Syntax highlighting and language servers are added later, on demand, only for the languages you actually use.

## Download

Grab the latest release from the [**releases page**](https://github.com/cvlmtg/HUME/releases). The newest tagged version is the one most people want; a [`nightly`](https://github.com/cvlmtg/HUME/releases/tag/nightly) build is also published if you want the newest changes. However keep in mind that the nightly release might be unstable or contain half-finished features.

| Platform | Archive |
|---|---|
| macOS (Apple Silicon) | `hume-*-aarch64-apple-darwin.tar.gz` |
| Linux (x86\_64, glibc 2.39+) | `hume-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86\_64) | `hume-*-x86_64-pc-windows-msvc.zip` |

### macOS / Linux

Extract and run:

```sh
tar xzf hume-*.tar.gz
./hume-*/bin/hume
```

Or copy it into `~/.local` for a single-user install (no sudo needed):

```sh
mkdir -p ~/.local && cp -R hume-*/bin hume-*/libexec hume-*/share ~/.local/
```

Then make sure `~/.local/bin` is on your `$PATH`:

::: code-group

```sh [bash]
# Add to ~/.profile if not already there
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH" ;;
esac
```

```sh [zsh]
# Add to ~/.zshrc if not already there
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH" ;;
esac
```

```fish [fish]
# fish_add_path skips duplicates
fish_add_path $HOME/.local/bin
```

:::

Or copy it into `/usr/local/` for a system-wide install:

```sh
cp -R hume-*/bin hume-*/libexec hume-*/share /usr/local/
```

::: warning macOS: "cannot be opened" dialog
The binaries aren't signed, so Gatekeeper may refuse to run them with a "cannot be opened" dialog. This happens if you extract the archive by double-clicking it in Finder rather than with `tar` above. Clear the quarantine flag from the whole extracted folder once and it won't come back:

```sh
xattr -dr com.apple.quarantine hume-*
```
:::

### Windows

Extract the `.zip`, then run `hume.exe` from inside the extracted folder, or add that folder to your `PATH`.

Keep the whole extracted folder intact: HUME finds its runtime files (themes, plugins, language definitions) and `hume-grammar-build`, the helper that compiles grammars, relative to the binary's location. On macOS/Linux they are `../share/hume/runtime/` and `../libexec/hume/` next to `bin/hume`; on Windows, `runtime/` and `hume-grammar-build.exe` next to `hume.exe`. Moving just the binary out on its own leaves it without them.

### Check it works

```sh
hume --version
```

prints something like `hume x.y.z-f460770`. The same string is available inside the editor with `:version`.

## Building from source

**Prerequisites:** a Rust toolchain. Install one from [rustup.rs](https://rustup.rs/).

```sh
git clone https://github.com/cvlmtg/HUME
cd HUME
cargo build --release
./target/release/hume
```

Run it from the repository root, as above. HUME picks up the `runtime/` directory sitting there.

::: warning `cargo install` needs one extra step
`cargo install --git https://github.com/cvlmtg/HUME hume-editor hume-grammar-build` installs the binaries but **not** the runtime files, so themes, plugins, `:tutor`, and syntax highlighting won't work. Point HUME at a copy of the `runtime/` directory to fix it:

```sh
export HUME_RUNTIME=/path/to/HUME/runtime
```
:::

## Syntax highlighting

HUME highlights code with tree-sitter, and installs grammars on demand rather than shipping them. That first install needs `git`, `curl`, and a C and C++ compiler on your `PATH`. See [Syntax Highlighting](syntax-highlighting.md) for the walkthrough.

## Terminal compatibility

HUME targets modern terminals and degrades quietly rather than refusing to start. There is no capability check at launch. You'll get the best results from a terminal that supports:

- **24-bit true color.** Colors are always emitted as RGB. Terminals without true color will show approximate or wrong colors, but HUME still runs.
- **Synchronized output.** Sent once per frame to avoid tearing; terminals that don't recognise it ignore it harmlessly.
- **The kitty keyboard protocol.** Detected automatically on WezTerm, kitty, ghostty, and foot. It unlocks a handful of extra key combinations. Everything else works without it, and the manual marks those keys "kitty only".

WezTerm on macOS doesn't send `Ctrl-|` correctly, so HUME never receives it. To make it work, add this line to the `config.keys` table in your `.wezterm.lua`:

```lua
{ key = '|', mods = 'CTRL|SHIFT', action = wezterm.action.SendString '\x1b[92:124;6u' },
```
