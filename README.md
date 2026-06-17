# HUME

**HUME's Unfinished Modal Editor**

*"We have no rational grounds to believe that a text editor, however carefully constructed, will ever reach a state we might call "finished". What we observe is a succession of patches, each one arising from the last, connected by habit rather than necessity. We call this progress, but that is merely a custom of thought.
The editor presents itself as a cursor, a buffer, a mode. Experience alone guides us.
I have written this editor for my own use, and release it without expectation. If it is useful to you, that is a happy coincidence. If it is not, I cannot say I am surprised."*

---

**The pragmatic modal editor.**

Zero friction, maximum output. HUME is built on a simple premise: the common case should be the short case, and the classic papercuts of text editing should be designed out from the start.

This project is driven by curiosity and the pure joy of hacking, not by deadlines. It is shared as-is for anyone who wants to explore a different pragmatic approach. Feel free to try it out, and expect a few rough edges.

## Why try it

- **Less typing for what you do most.** Selections come first, so acting on text is short and direct.
- **Paste that does the obvious thing.** `p` reaches for what you most likely meant.
- **Batteries included.** Plugins and language support work out of the box — no extra tooling to set up.
- **Comfortable with real text.** Emoji, accents, and other multi-byte characters are treated as single characters, the way you'd expect.
- **Usable immediately, yours to shape.** Sensible defaults on day one; customize everything in a single language when you're ready.

---

## Install (nightly builds)

Pre-built archives are published on every push to `main` at the [**nightly release**](../../releases/tag/nightly).

| Platform | Archive |
|---|---|
| macOS (Apple Silicon) | `hume-*-aarch64-apple-darwin.tar.gz` |
| Linux (x86\_64, glibc 2.39+) | `hume-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86\_64) | `hume-*-x86_64-pc-windows-msvc.zip` |

**macOS / Linux** — extract and run:
```
tar xzf hume-*.tar.gz
./hume-*/bin/hume
```
Or copy to `/usr/local/` for a system install: `cp -R hume-*/* /usr/local/`

**Windows** — extract the `.zip`, then run `hume.exe` from inside the folder, or add the folder to your `PATH`.

Run `hume --version` to confirm the build (e.g. `hume 0.1.0-f460770`). The same string is available inside the editor via `:version`.

## Getting started

Once inside the editor, type `:tutor` and press Enter to open the interactive tutorial. It covers motion, editing, search, multi-cursor, and file commands through hands-on exercises you can practice directly in the buffer.
