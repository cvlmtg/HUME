# Installation

HUME ships as one archive: the editor plus the themes, plugins and language definitions it reads at runtime. Download it, extract it, and run it. You add syntax highlighting and language servers later, on demand, only for the languages you use.

## Download

Grab the latest release from the [**releases page**](https://github.com/cvlmtg/HUME/releases). Most people want the newest tagged version. A [`nightly`](https://github.com/cvlmtg/HUME/releases/tag/nightly) build carries the newest changes, but it may be unstable or contain half-finished features.

| Platform | Archive |
|---|---|
| macOS (Apple Silicon) | `hume-*-aarch64-apple-darwin.tar.gz` |
| Linux (x86\_64, glibc 2.28+) | `hume-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86\_64) | `hume-*-x86_64-pc-windows-msvc.zip` |

### macOS / Linux

Extract and run:

```sh
tar xzf hume-*.tar.gz
./hume-*/bin/hume
```

Or copy it into `~/.local` for a single-user install (no sudo needed):

```sh
mkdir -p ~/.local && cp -R hume-*/bin hume-*/share ~/.local/
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
cp -R hume-*/bin hume-*/share /usr/local/
```

::: warning macOS: "cannot be opened" dialog
The binaries aren't signed. If you extract the archive by double-clicking it in Finder instead of using `tar` as above, Gatekeeper may refuse to run them with a "cannot be opened" dialog. Clear the quarantine flag from the whole extracted folder once, and it won't come back:

```sh
xattr -dr com.apple.quarantine hume-*
```
:::

### Windows

Extract the `.zip`, then run `hume.exe` from inside the extracted folder, or add that folder to your `PATH`.

Keep the whole extracted folder intact. HUME finds its runtime files (themes, plugins, language definitions) relative to the `hume` binary:

- macOS/Linux: `../share/hume/runtime/` next to `bin/hume`.
- Windows: `runtime/` next to `hume.exe`.

A binary moved out on its own can't find them.

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

Run it from the repository root, as above, and HUME picks up the `runtime/` directory there.

::: warning `cargo install` needs one extra step
`cargo install --git https://github.com/cvlmtg/HUME hume-editor` installs the binary but **not** the runtime files, so themes, plugins, `:tutor`, and syntax highlighting won't work. Point HUME at a copy of the `runtime/` directory to fix it:

```sh
export HUME_RUNTIME=/path/to/HUME/runtime
```
:::

## Syntax highlighting

HUME highlights code with tree-sitter and downloads prebuilt grammars on demand instead of shipping them. That needs `curl` and `tar` on your `PATH`. See [Syntax Highlighting](syntax-highlighting.md) for the walkthrough.

## Terminal compatibility

HUME targets modern terminals. It runs no capability check at launch and degrades quietly instead of refusing to start. It works best in a terminal that supports:

- **24-bit true color.** HUME always sends colors as RGB. Terminals without true color show approximate or wrong colors, but HUME still runs.
- **Synchronized output.** HUME uses it once per frame to avoid tearing. Terminals that don't recognise it ignore it harmlessly.
- **The kitty keyboard protocol.** HUME requests it at startup and uses it whenever the terminal answers. It unlocks a few extra key combinations, which the manual marks "kitty only"; everything else works without it. While the protocol is on, the statusline shows a cat (ᓚᘏᗢ).

Terminals that support the kitty keyboard protocol:

- kitty, Ghostty, iTerm2, Alacritty, Rio, and Warp
- foot, on Linux
- Windows Terminal 1.25 or later, which includes WSL and Git Bash shells running inside it
- WezTerm, once you turn it on (see below)

In iTerm2, **Settings → Profiles → Keys → Allow application to change how keys are reported** must be checked.

Terminal.app, GNOME Terminal and other terminals built on VTE, Konsole, and mintty (Git Bash's own window) don't support the protocol, and neither does tmux, even inside a terminal that does. Use Extend mode (`e`) there instead of the `Ctrl` one-shot extends.

WezTerm leaves the protocol off by default. To turn it on, add this line to your `.wezterm.lua`:

```lua
config.enable_kitty_keyboard = true
```

WezTerm on macOS doesn't send `Ctrl-|` correctly, so HUME never receives it. To make it work, add this line to the `config.keys` table in your `.wezterm.lua`:

```lua
{ key = '|', mods = 'CTRL|SHIFT', action = wezterm.action.SendString '\x1b[92:124;6u' },
```
