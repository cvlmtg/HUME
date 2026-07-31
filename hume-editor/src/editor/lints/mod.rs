//! Compile-time lints enforced as `cargo test` unit tests.
//!
//! Each lint scans a curated list of source files for patterns that violate an
//! architectural rule. Tests fail with a human-readable violation list so the
//! offending line is easy to locate and fix. One file per lint (below);
//! shared string/source-scanning helpers live in this module.

mod dispatch_funnel;
mod field_classification;
mod grapheme;
mod manual_options;
mod plugin_manifest;
mod resync_derived_state;
mod scm_headers;
mod statusline_writes;

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Collect all `.rs` files under `dir`, recursively, excluding any
/// directory named `tests` and any file named `tests.rs`.  Results are
/// sorted for deterministic test output.
fn collect_source_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let n = name.to_string_lossy();
        if path.is_dir() && n != "tests" {
            collect_source_rs(&path, out);
        } else if path.is_file() && n.ends_with(".rs") && n != "tests.rs" {
            out.push(path);
        }
    }
}

/// The portion of `line` before any line comment (`//`), skipping `//`
/// that appears inside a string literal — a naive `line.find("//")`
/// would truncate a call like `write_global(key, "a//b", ...)`
/// at the string's embedded `//`, hiding the rest of the line (and any
/// forbidden pattern in it) from every lint that strips comments this
/// way. Escaped quotes (`\"`) inside a string keep it open; char
/// literals (`'x'`, `'\x'`) are skipped so their quote marks don't
/// falsely open/close string tracking; a bare `'` that isn't a char
/// literal (a lifetime) is left alone. Raw strings (`r"..."`) are not
/// handled — none of the scanned patterns appear inside one today.
fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') => {
                i += 4; // escaped char literal: '\x'
            }
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => {
                i += 3; // char literal: 'x'
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => i += 1,
        }
    }
    line
}

/// Extracts every double-quoted string literal's contents from `s`,
/// verbatim (no escape processing — plugin command names never contain a
/// `"`, so a naive quote-delimited split is exact here).
fn quoted_strings(s: &str) -> Vec<String> {
    s.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The text between the first occurrence of `heading` and the next
/// top-level (`\n## `) heading — used to scope a key scan to just one
/// section of a markdown doc, not the whole file.
fn section_after<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("heading '{heading}' not found in configuration.md"));
    let after = &text[start + heading.len()..];
    let end = after.find("\n## ").unwrap_or(after.len());
    &after[..end]
}

/// Every `` `key` `` in a markdown table's first column: a line trimmed
/// to start with `` | ` `` (no other content in this file's tables looks
/// like that once scoped to one `## `-delimited section).
fn first_column_keys(section: &str) -> std::collections::BTreeSet<String> {
    section
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("| `")?;
            let (key, _) = rest.split_once('`')?;
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

/// Extract top-level field names from a Rust struct's body text (the
/// text strictly between its outer `{` and matching `}`). Strips
/// `///`/`//` comment lines and `#[...]` attribute lines first (so a
/// doc comment mentioning a colon, or `#[cfg(test)]` on its own line,
/// is never mistaken for a field), then splits on depth-0 commas —
/// tracking `(){}[]<>` nesting so a field's own generic type (e.g.
/// `Vec<(BufferId, Option<String>)>`, or a wrapped multi-line type)
/// is never mistaken for a field boundary — and takes the last
/// identifier before each segment's first depth-0 `:` (skipping `::`
/// path separators, including the ones inside `pub(in crate::editor)`)
/// as that field's name.
fn struct_field_names(body: &str) -> Vec<String> {
    let stripped: String = body
        .lines()
        .filter(|line| {
            let t = line.trim();
            !t.starts_with("///") && !t.starts_with("//") && !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join(" ");

    fn field_name_in_segment(seg: &[char]) -> Option<String> {
        let mut depth = 0i32;
        let mut colon_at = None;
        for (i, &c) in seg.iter().enumerate() {
            match c {
                '(' | '[' | '{' | '<' => depth += 1,
                ')' | ']' | '}' | '>' => depth -= 1,
                ':' if depth == 0 => {
                    let is_path_sep = seg.get(i + 1) == Some(&':') || (i > 0 && seg[i - 1] == ':');
                    if !is_path_sep {
                        colon_at = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        seg[..colon_at?]
            .iter()
            .collect::<String>()
            .split_whitespace()
            .next_back()
            .map(str::to_string)
    }

    let chars: Vec<char> = stripped.chars().collect();
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut seg_start = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                if let Some(name) = field_name_in_segment(&chars[seg_start..i]) {
                    names.push(name);
                }
                seg_start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(name) = field_name_in_segment(&chars[seg_start..]) {
        names.push(name);
    }
    names
}

/// Extract the top-level field names of `struct_decl` (e.g.
/// `"pub(crate) struct EditorState {"`) as found in `src`, excluding
/// `exempt` (fields governed by their own separate classification —
/// `EditorState.config`, `Editor.state`/`Editor.view`).
fn struct_fields_excluding(
    src: &str,
    struct_decl: &str,
    exempt: &[&str],
) -> std::collections::BTreeSet<String> {
    let struct_start = src
        .find(struct_decl)
        .unwrap_or_else(|| panic!("{struct_decl:?} not found in editor/mod.rs"));
    let body_start = src[struct_start..]
        .find('{')
        .expect("no opening brace for struct")
        + struct_start;
    let mut depth = 0i32;
    let mut body_end = body_start;
    for (i, c) in src[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    body_end = body_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    struct_field_names(&src[body_start + 1..body_end])
        .into_iter()
        .filter(|f| !exempt.contains(&f.as_str()))
        .collect()
}

/// Diff `fields` against `classification`'s names in both directions —
/// shared assertion body for `editor_state_fields_are_classified` and
/// `editor_fields_are_classified`. `struct_name`/`const_name` only shape
/// the panic messages.
fn assert_fields_classified(
    fields: &std::collections::BTreeSet<String>,
    classification: &[(&str, &str)],
    struct_name: &str,
    const_name: &str,
) {
    let classified: std::collections::BTreeSet<&str> =
        classification.iter().map(|(name, _)| *name).collect();

    let unclassified: Vec<&String> = fields
        .iter()
        .filter(|f| !classified.contains(f.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "{struct_name} gained new field(s) {unclassified:?} with no entry in \
         {const_name} — decide whether :reload-config's reset should touch \
         it (add the mechanism to reset_config_state and classify it \
         \"config: …\" here), or whether it genuinely survives a reload \
         (classify it \"preserved\"), then add the entry"
    );

    let stale: Vec<&str> = classified
        .iter()
        .filter(|name| !fields.contains(name.to_string().as_str()))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "{const_name} lists {stale:?}, which is no longer a field on \
         {struct_name} — remove the stale entry"
    );
}

#[test]
fn strip_line_comment_cases() {
    // Fail oracle: revert strip_line_comment to a naive `line.find("//")`
    // and the string-literal cases below (2nd and 4th) must start failing.
    assert_eq!(strip_line_comment("foo(); // bar"), "foo(); ");
    assert_eq!(strip_line_comment("foo();"), "foo();");
    assert_eq!(strip_line_comment(r#"call("a//b")"#), r#"call("a//b")"#);
    assert_eq!(
        strip_line_comment(r#"call("a//b") // note"#),
        r#"call("a//b") "#
    );
    assert_eq!(
        strip_line_comment(r#"let q = '"'; // c"#),
        r#"let q = '"'; "#
    );
    assert_eq!(
        strip_line_comment(r#""a\"//b""#),
        r#""a\"//b""#,
        "escaped quote must not end the string early"
    );
}
