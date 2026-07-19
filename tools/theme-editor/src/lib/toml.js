// Shared string-aware scanner. Yields { i, c, inStr } for every character;
// inStr=true while inside a "..." or '...' literal (delimiters and escapes included).
// Handles: single-quoted strings, escaped backslashes in double-quoted strings,
// quoted TOML keys containing dots or '=', and inline array/table values.
export function* scanString(s) {
  let inDouble = false, inSingle = false, escape = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (escape) { escape = false; yield { i, c, inStr: true }; continue; }
    if (inDouble) {
      if (c === '\\') { escape = true; yield { i, c, inStr: true }; continue; }
      if (c === '"') { inDouble = false; yield { i, c, inStr: true }; continue; }
      yield { i, c, inStr: true }; continue;
    }
    if (inSingle) {
      if (c === "'") { inSingle = false; yield { i, c, inStr: true }; continue; }
      yield { i, c, inStr: true }; continue;
    }
    if (c === '"') { inDouble = true; yield { i, c, inStr: true }; continue; }
    if (c === "'") { inSingle = true; yield { i, c, inStr: true }; continue; }
    yield { i, c, inStr: false };
  }
}

export function stripComment(line) {
  for (const { i, c, inStr } of scanString(line)) {
    if (!inStr && c === '#') return line.slice(0, i);
  }
  return line;
}

export function countChars(s, open, close) {
  let d = 0;
  for (const { c, inStr } of scanString(s)) {
    if (!inStr) {
      if (c === open) d++;
      if (c === close) d--;
    }
  }
  return d;
}

export function splitPairs(s) {
  const pairs = [];
  let cur = "", depth = 0;
  for (const { c, inStr } of scanString(s)) {
    if (inStr) { cur += c; continue; }
    if (c === '[' || c === '{') { depth++; cur += c; continue; }
    if (c === ']' || c === '}') { depth--; cur += c; continue; }
    if (c === ',' && depth === 0) { pairs.push(cur.trim()); cur = ""; continue; }
    cur += c;
  }
  if (cur.trim()) pairs.push(cur.trim());
  return pairs;
}

// Split a TOML dotted key (e.g. `a."b.c"` or `"ui.cursor"`) on dots outside
// string literals, then strip surrounding quotes from each segment.
export function splitDottedKey(s) {
  const parts = [];
  let cur = "";
  for (const { c, inStr } of scanString(s)) {
    if (inStr) { cur += c; continue; }
    if (c === '.') { parts.push(cur.trim().replace(/^["']|["']$/g, "")); cur = ""; continue; }
    cur += c;
  }
  if (cur.trim()) parts.push(cur.trim().replace(/^["']|["']$/g, ""));
  return parts;
}

export function parseInlineArray(s) {
  const inner = s.slice(1, -1).trim();
  if (!inner) return [];
  return splitPairs(inner).map(e => parseInlineVal(e));
}

// Unescape TOML basic-string escapes (double-quoted strings only; single-quoted
// TOML literals have no escapes). Unknown escape sequences keep their literal char.
export function unescapeBasic(s) {
  return s.replace(/\\(u[0-9a-fA-F]{4}|.)/gs, (m, esc) => {
    if (esc[0] === "u") return String.fromCodePoint(parseInt(esc.slice(1), 16));
    switch (esc) {
      case "\\": return "\\";
      case '"': return '"';
      case "n": return "\n";
      case "t": return "\t";
      case "r": return "\r";
      case "b": return "\b";
      case "f": return "\f";
      default: return esc;
    }
  });
}

const NUMBER_RE = /^[+-]?\d[\d_]*(\.[\d_]+)?$/;

export function parseInlineVal(s) {
  s = s.trim();
  if (s.startsWith('"') && s.endsWith('"')) return unescapeBasic(s.slice(1, -1));
  if (s.startsWith("'") && s.endsWith("'")) return s.slice(1, -1);
  if (s.startsWith('{') && s.endsWith('}')) return parseInlineTable(s);
  if (s.startsWith('[') && s.endsWith(']')) return parseInlineArray(s);
  if (s === "true") return true;
  if (s === "false") return false;
  if (NUMBER_RE.test(s)) return Number(s.replace(/_/g, ""));
  return s;
}

export function parseInlineTable(s) {
  const inner = s.slice(1, -1).trim();
  if (!inner) return {};
  const obj = {};
  for (const pair of splitPairs(inner)) {
    const eq = pair.indexOf('=');
    if (eq === -1) continue;
    const k = pair.slice(0, eq).trim().replace(/^["']|["']$/g, "");
    obj[k] = parseInlineVal(pair.slice(eq + 1).trim());
  }
  return obj;
}

export function parseTOML(text) {
  const result = {};
  let cur = result;
  const lines = text.split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = stripComment(lines[i]).trim();
    i++;
    if (!line) continue;

    const sec = line.match(/^\[([^\]]+)\]$/);
    if (sec) {
      cur = result;
      for (const part of splitDottedKey(sec[1])) {
        if (!cur[part]) cur[part] = {};
        cur = cur[part];
      }
      continue;
    }

    // Find '=' outside string literals (keys may contain quoted '=').
    let eq = -1;
    for (const { i: pos, c, inStr } of scanString(line)) {
      if (!inStr && c === '=') { eq = pos; break; }
    }
    if (eq === -1) continue;

    // "ui.cursor" → ["ui.cursor"] → "ui.cursor"; ui.cursor → ["ui","cursor"] → "ui.cursor".
    // Both produce the same flat key to match the shape upstream Helix themes use.
    const key = splitDottedKey(line.slice(0, eq).trim()).join(".");
    let val = line.slice(eq + 1).trim();

    let braceDepth = countChars(val, '{', '}');
    let bracketDepth = countChars(val, '[', ']');
    while ((braceDepth > 0 || bracketDepth > 0) && i < lines.length) {
      const nextLine = stripComment(lines[i]).trim();
      i++;
      val += " " + nextLine;
      braceDepth += countChars(nextLine, '{', '}');
      bracketDepth += countChars(nextLine, '[', ']');
    }
    val = val.trim();

    cur[key] = parseInlineVal(val);
  }
  return result;
}

// Single value formatter — arrays and nested tables are never wrapped in quotes.
export function formatVal(v) {
  if (Array.isArray(v)) return "[ " + v.map(formatVal).join(", ") + " ]";
  if (typeof v === "string") {
    return '"' + v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
      .replace(/\n/g, '\\n').replace(/\t/g, '\\t').replace(/\r/g, '\\r') + '"';
  }
  if (typeof v === "object" && v !== null) {
    const parts = Object.entries(v).map(([k, val]) => k + " = " + formatVal(val)).join(", ");
    return "{ " + parts + " }";
  }
  return String(v);
}

const SCOPE_KEYS = ["fg", "bg", "modifiers", "underline", "style"];

function isTable(v) {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

// Pull scope definitions from a parsed TOML object. Skips `palette` and `inherits`.
// Recursively promotes nested sections (e.g. `[ui.statusline.normal]`) into flat
// dotted keys ("ui.statusline.normal"), matching the shape the editor uses.
// A table is emitted as a scope def once it has no more sub-tables to descend
// into (or is empty, e.g. `"ui.cursor.insert" = {}`) — deeper keys keep recursing.
function walkScopes(ns, obj, prefix) {
  const keys = Object.keys(obj);
  const styleKeys = keys.filter(k => SCOPE_KEYS.includes(k));
  if (prefix && (keys.length === 0 || styleKeys.length > 0)) {
    const def = {};
    for (const k of styleKeys) def[k] = obj[k];
    ns[prefix] = def;
  }
  for (const k of keys) {
    if (SCOPE_KEYS.includes(k)) continue;
    if (!prefix && (k === "palette" || k === "inherits")) continue;
    const v = obj[k];
    const path = prefix ? prefix + "." + k : k;
    if (typeof v === "string") ns[path] = v;
    else if (isTable(v)) walkScopes(ns, v, path);
  }
}

export function extractScopes(parsed) {
  const ns = {};
  walkScopes(ns, parsed, "");
  return ns;
}

// Emit every scope as a flat top-level quoted dotted key, then [palette] last.
// We avoid [section] headers: TOML section semantics would merge a `[diff]` header
// with an existing top-level `"diff" = "overlay"` into one nested object, losing
// sub-keys after extractScopes. Flat keys match upstream Helix themes (rose_pine, etc.).
export function exportTOML(palette, scopes) {
  let out = "";
  for (const [k, v] of Object.entries(scopes)) out += '"' + k + '" = ' + formatVal(v) + '\n';
  out += "\n[palette]\n";
  for (const [k, v] of Object.entries(palette)) out += '"' + k + '" = ' + formatVal(v) + '\n';
  return out;
}
