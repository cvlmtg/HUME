# HUME — Project Instructions

## What is this?
HUME (HUME's Unfinished Modal Editor) is a modal text editor for the terminal, written in Rust. This is a vibe coding / learning project — not a production product.

## Key files
- `README.md` — Project description
- `GOALS.md` — Vision, decisions, and open questions
- `PLAN.md` — Tech stack, architecture, and milestones

## Rules
- **Update GOALS.md** when a decision is made (add to the decisions table, remove from open questions)
- **Update PLAN.md** when the plan changes (milestones, tech stack, architecture)
- **Rust idioms**: Write idiomatic Rust. Prefer pattern matching, iterators, and the type system over runtime checks. Use `Result` and `Option` — no `.unwrap()` in non-test code.
- **Modern terminals only**: Do not add compatibility shims for legacy terminals. Target terminals supporting 24-bit color, kitty keyboard protocol, etc.
- **Cross-platform**: macOS primary, Linux and Windows (Git Bash / WSL) secondary. Use `crossterm` or similar abstractions for platform differences — no platform-specific code unless behind `cfg` gates.
- **Keep it simple**: This is a learning project. Prefer clarity over cleverness, and direct solutions over premature abstraction.
