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


def scheme_list(items: list) -> str:
    if not items:
        return "'()"
    return "'(" + " ".join(scheme_str(x) for x in items) + ")"


def read_pin(path: Path) -> str:
    """Extract the single quoted pin literal from a `*-pin.scm` file.

    Works for both hex SHAs (helix-pin.scm) and non-hex release tags
    (mason-pin.scm) — the pin is just the first double-quoted string left
    after stripping `;;` comment lines.
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


def write_generated_file(path: Path, header: str, rows: list[str]) -> None:
    """Assemble `header` + the row lines + a trailing newline and write it
    atomically — the shape every generated `*.scm` file shares."""
    write_atomic(path, "\n".join([header, "\n".join(rows), ""]))


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
