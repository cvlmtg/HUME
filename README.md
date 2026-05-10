# HUME

**HUME's Unfinished Modal Editor**

*"We have no rational grounds to believe that a text editor, however carefully constructed, will ever reach a state we might call "finished". What we observe is a succession of patches, each one arising from the last, connected by habit rather than necessity. We call this progress, but that is merely a custom of thought.
The editor presents itself as a cursor, a buffer, a mode. Experience alone guides us.
I have written this editor for my own use, and release it without expectation. If it is useful to you, that is a happy coincidence. If it is not, I cannot say I am surprised."*

---

This project is built for the joy of building, guided by curiosity rather than roadmap. There is no expectation that it will ever reach production, but feel free to try it out. Use with caution.

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
