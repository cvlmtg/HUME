"""Shared helpers for scripts/sync-*.py — pin reading, sexpr emission, atomic writes.

Every sync script regenerates checked-in runtime/scheme/*.scm data files from a
pinned upstream revision. This module holds the parts that don't vary between
scripts: how a pin file is read, how Python values become Scheme literals, and
how a generated file is written and read back.
"""

import os
import re
import sys
import urllib.request
from pathlib import Path
from urllib.error import URLError


class Sym(str):
    """A bare (unquoted) Scheme symbol, as read back by `read_sexpr`.

    Distinguishes `foo` from `"foo"` — both are Python strings otherwise.
    """

    __slots__ = ()


def scheme_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def scheme_str_list(items) -> str:
    return " ".join(scheme_str(x) for x in items)


def scheme_list(items: list) -> str:
    if not items:
        return "'()"
    return "'(" + scheme_str_list(items) + ")"


def read_pin(path: Path) -> str:
    """Extract the single quoted pin literal from a `*-pin.scm` file.

    Works for any pin literal, such as a release tag (mason-pin.scm,
    treesitter-hume-pin.scm) — the pin is just the first double-quoted string
    left after stripping `;;` comment lines.
    """
    text = path.read_text(encoding="utf-8")
    body = re.sub(r";;.*", "", text)
    m = re.search(r'"([^"]+)"', body)
    if not m:
        sys.exit(f"error: could not extract pin value from {path}")
    return m.group(1)


def write_atomic(path: Path, content: str) -> None:
    tmp = path.with_suffix(".scm.tmp")
    tmp.write_text(content, encoding="utf-8")
    os.replace(tmp, path)
    print(f"wrote {path}", file=sys.stderr)


def generated_file_text(header: str, rows: list[str]) -> str:
    """`header` + the row lines + a trailing newline — the shape every
    generated `*.scm` file shares."""
    return "\n".join([header, "\n".join(rows), ""])


def write_generated_file(path: Path, header: str, rows: list[str]) -> None:
    """Write `generated_file_text(header, rows)` atomically."""
    write_atomic(path, generated_file_text(header, rows))


# A language or server name names its data file: lowercase so it stays
# distinct on case-insensitive file systems, no leading dot so it is never a
# dotfile.
FILE_NAME = re.compile(r"[a-z0-9_-][a-z0-9._-]*")


def check_file_names(what: str, names) -> None:
    """Fatal unless every name matches FILE_NAME; `what` names the kind in the
    error."""
    for name in names:
        if not FILE_NAME.fullmatch(name):
            sys.exit(f"error: {what} name '{name}' does not match {FILE_NAME.pattern}")


def named_files(directory: Path) -> dict[str, Path]:
    """`{name: path}` for each `<directory>/<name>.scm`, sorted by name. A
    missing directory has none. Dotfiles (Finder's `.DS_Store`) are skipped.
    The directory holds generated files only, so any other entry is fatal."""
    if not directory.is_dir():
        return {}
    entries = sorted(p for p in directory.iterdir() if not p.name.startswith("."))
    foreign = [p.name for p in entries if p.suffix != ".scm"]
    if foreign:
        sys.exit(f"error: {directory} holds files that are not generated: {foreign}")
    return {p.stem: p for p in entries}


def write_named_files(directory: Path, header, bodies: dict[str, list[str]]) -> None:
    """Write `<directory>/<name>.scm` for each `name` in `bodies`, headed by
    `header(name)`, and delete the file of any name not in `bodies`. Every
    name is checked against FILE_NAME before anything is written."""
    check_file_names(f"{directory} file", bodies)
    directory.mkdir(parents=True, exist_ok=True)
    existing = named_files(directory)

    for name, rows in bodies.items():
        path = directory / f"{name}.scm"
        write_generated_file(path, header(name), rows)
        read_sexpr(path)  # self-check: emitted file must re-parse

    for name, path in existing.items():
        if name not in bodies:
            path.unlink()
            print(f"removed {path}", file=sys.stderr)


def read_named_files(directory: Path) -> dict:
    """`{name: body}` for each file `named_files` lists."""
    return {name: read_sexpr(path) for name, path in named_files(directory).items()}


def fetch_bytes(url: str, *, timeout: int) -> bytes:
    """GET `url`, returning the raw response body. Aborts the whole sync
    loudly on failure — this is for a script's one registry/source-of-truth
    fetch, not a per-asset download where a single failure should only skip
    that one asset (see `sync-lsp-sources.py`'s `sha256_of_url`, which has
    its own narrower catch for exactly that reason)."""
    print(f"fetching {url}", file=sys.stderr)
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.read()
    except URLError as e:
        sys.exit(f"error: failed to fetch {url}: {e}")


def read_sexpr(path: Path):
    """Parse the single top-level literal sexpr in a generated data file.

    Supports exactly the subset this pipeline emits: nested lists, quoted
    strings (`\\` and `\"` escapes), dotted pairs `(k . v)`, bare symbols,
    `#t`/`#f`, integers, `#(...)` vector literals, and `;` line comments.
    Lists become Python lists; a dotted pair `(k . v)` becomes a 2-tuple
    `(Sym, value)`; bare symbols become `Sym` instances so callers can tell
    them apart from quoted strings. A `#(...)` vector also becomes a plain
    Python list — nothing in this pipeline needs to distinguish a vector
    from a list once parsed back into Python, only the Scheme reader does.
    """
    text = path.read_text(encoding="utf-8")
    n = len(text)
    pos = 0

    def skip_ws():
        nonlocal pos
        while pos < n:
            c = text[pos]
            if c == ";":
                nl = text.find("\n", pos)
                pos = n if nl == -1 else nl + 1
            elif c.isspace():
                pos += 1
            else:
                return

    def read_string():
        nonlocal pos
        pos += 1  # opening quote
        out = []
        while True:
            if pos >= n:
                raise ValueError(f"{path}: unterminated string")
            c = text[pos]
            if c == "\\":
                pos += 1
                out.append(text[pos])
                pos += 1
            elif c == '"':
                pos += 1
                return "".join(out)
            else:
                out.append(c)
                pos += 1

    def read_atom():
        nonlocal pos
        start = pos
        while pos < n and text[pos] not in " \t\n\r()\";":
            pos += 1
        tok = text[start:pos]
        if tok == "#t":
            return True
        if tok == "#f":
            return False
        try:
            return int(tok)
        except ValueError:
            return Sym(tok)

    def read_value():
        nonlocal pos
        skip_ws()
        if pos >= n:
            raise ValueError(f"{path}: unexpected end of input")
        c = text[pos]
        if c == "#" and pos + 1 < n and text[pos + 1] == "(":
            pos += 2
            items = []
            while True:
                skip_ws()
                if pos >= n:
                    raise ValueError(f"{path}: unterminated vector")
                if text[pos] == ")":
                    pos += 1
                    return items
                items.append(read_value())
        if c == "(":
            pos += 1
            items = []
            while True:
                skip_ws()
                if pos >= n:
                    raise ValueError(f"{path}: unterminated list")
                if text[pos] == ")":
                    pos += 1
                    return items
                if (
                    text[pos] == "."
                    and pos + 1 < n
                    and text[pos + 1].isspace()
                    and len(items) == 1
                ):
                    pos += 1
                    cdr = read_value()
                    skip_ws()
                    if pos >= n or text[pos] != ")":
                        raise ValueError(f"{path}: malformed dotted pair")
                    pos += 1
                    return (items[0], cdr)
                items.append(read_value())
        if c == '"':
            return read_string()
        return read_atom()

    skip_ws()
    value = read_value()
    skip_ws()
    if pos != n:
        raise ValueError(f"{path}: trailing content after top-level form")
    return value
