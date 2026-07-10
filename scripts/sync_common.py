"""Shared helpers for scripts/sync-*.py — pin reading, sexpr emission, atomic writes.

Every sync script regenerates checked-in runtime/scheme/*.scm data files from a
pinned upstream revision. This module holds the parts that don't vary between
scripts: how a pin file is read, how Python values become Scheme literals, and
how a generated file is written and read back.
"""

import os
import re
import sys
from pathlib import Path


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


def sexpr_dumps(value, *, vector_arrays: bool = False) -> str:
    """Render a TOML/JSON-parsed value as a canonical Scheme literal.

    - dict -> alist: a key whose value is a scalar becomes `(key . value)`;
      a key whose value is a dict becomes `(key <nested>)` (the wrapping
      parens are added by the *caller* iterating the dict, so this composes
      without double-wrapping; an empty dict becomes `(key)`, the empty
      tail). A key whose value is a list becomes `(key elem…)` — or, when
      `vector_arrays` is set, `(key . #(elem…))` (`#()` for an empty list).
      Keys are sorted for determinism.
    - list -> its elements, each dumped and space-joined (top-level only;
      nested-in-dict lists go through the branch above).
    - str -> a quoted Scheme string; bool -> `#t`/`#f`; int/float -> literal.

    `vector_arrays` exists because the two non-vector shapes above collide:
    a dict value that's a list and a dict value that's an empty dict both
    render as `(key elem…)`/`(key)`, so a reader can't tell a JSON array
    from a JSON object by shape alone. Emitting arrays as `#(...)` instead
    makes the two shapes distinct: `(key . #(...))` is always an array
    (`#()` when empty), `(key entries…)` is always a nested object (`(key)`
    when empty). Callers that don't need this distinction (e.g. grammar
    source tuples, which have no nested arrays) can omit the flag.

    Absent/empty values are never represented here as `#f` — callers emit
    the empty tail (e.g. `(settings)`) themselves when a value is missing.
    """
    if isinstance(value, bool):
        return "#t" if value else "#f"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, str):
        return scheme_str(value)
    if isinstance(value, dict):
        parts = []
        for k in sorted(value):
            v = value[k]
            if isinstance(v, dict):
                parts.append(f"({k} {sexpr_dumps(v, vector_arrays=vector_arrays)})")
            elif isinstance(v, list):
                if vector_arrays:
                    inner = " ".join(sexpr_dumps(x, vector_arrays=vector_arrays) for x in v)
                    parts.append(f"({k} . #({inner}))")
                else:
                    inner = " ".join(sexpr_dumps(x, vector_arrays=vector_arrays) for x in v)
                    parts.append(f"({k} {inner})")
            else:
                parts.append(f"({k} . {sexpr_dumps(v, vector_arrays=vector_arrays)})")
        return " ".join(parts)
    if isinstance(value, list):
        return " ".join(sexpr_dumps(x, vector_arrays=vector_arrays) for x in value)
    raise TypeError(f"sexpr_dumps: unsupported type {type(value).__name__}")


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
