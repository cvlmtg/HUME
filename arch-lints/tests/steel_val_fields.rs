//! # Steel values Rust keeps are rooted
//!
//! Steel's collector traces only its own stack, globals and registered
//! roots, so a `SteelVal` stored in a Rust struct loses the heap cells it
//! reaches at the next full collection. Every such store holds a
//! `hume_scripting::Rooted` instead.
//!
//! `steel_val_is_never_a_stored_field` scans `hume-scripting/src` and
//! `hume-editor/src` (non-test code) for `SteelVal` inside a `struct`,
//! `enum`, `union` or `type` declaration, outside `rooted.rs` itself.
//!
//! **Opt-out**: `// steel-val-transient: <reason>`, for a type whose values
//! never live across a Steel evaluation.

use arch_lints::{scan_type_bodies, type_body_hits, workspace_root, workspace_source_paths};

const OPT_OUT_MARKER: &str = "// steel-val-transient:";
const TOKEN: &str = "SteelVal";

#[test]
fn type_body_hits_finds_fields_payloads_and_aliases() {
    let src = r#"
pub struct Braced {
    keep: SteelVal,
}
struct Tuple(SteelVal);
pub(crate) struct MultiLine(
    u8,
    steel::rvals::SteelVal,
);
enum Kinds {
    Tuple(SteelVal),
    Named { proc: SteelVal },
}
struct Nested {
    calls: Vec<(KeyEvent, steel::rvals::SteelVal)>,
}
type Call = (SteelVal, Vec<SteelVal>);
"#;
    let lines: Vec<&str> = src.lines().collect();
    let hits: Vec<&str> = type_body_hits(src, TOKEN)
        .into_iter()
        .map(|i| lines[i].trim())
        .collect();
    assert_eq!(
        hits,
        [
            "keep: SteelVal,",
            "struct Tuple(SteelVal);",
            "steel::rvals::SteelVal,",
            "Tuple(SteelVal),",
            "Named { proc: SteelVal },",
            "calls: Vec<(KeyEvent, steel::rvals::SteelVal)>,",
            "type Call = (SteelVal, Vec<SteelVal>);",
        ]
    );
}

#[test]
fn type_body_hits_ignores_code_outside_declarations() {
    let src = r#"
struct Plain {
    n: u8,
}
impl Plain {
    fn take(&self, value: SteelVal) -> SteelVal {
        value
    }
}
fn render() -> String {
    format!("{} {{", 1)
}
struct Later {
    kept: RootedSteelVal,
    other: SteelValue,
}
fn after(value: SteelVal) {}
trait Host {
    fn call(&mut self, proc: SteelVal);
}
"#;
    assert_eq!(type_body_hits(src, TOKEN), Vec::<usize>::new());
}

/// A `SteelVal` stored in a struct or enum of `hume-scripting` or
/// `hume-editor` is reported with its file and line.
#[test]
fn steel_val_is_never_a_stored_field() {
    let workspace_root = workspace_root();
    let rooted_rs = workspace_root.join("hume-scripting/src/rooted.rs");
    let scanned = [
        workspace_root.join("hume-scripting/src"),
        workspace_root.join("hume-editor/src"),
    ];
    let paths: Vec<_> = workspace_source_paths(&workspace_root, &[rooted_rs])
        .into_iter()
        .filter(|p| scanned.iter().any(|dir| p.starts_with(dir)))
        .collect();
    assert!(!paths.is_empty(), "the scan found no source files");

    for path in &paths {
        let src = std::fs::read_to_string(path).expect("readable source");
        for (index, line) in src.lines().enumerate() {
            if let Some((_, reason)) = line.split_once(OPT_OUT_MARKER) {
                assert!(
                    !reason.trim().is_empty(),
                    "{}:{}: an opt-out names why the value never outlives an evaluation",
                    path.display(),
                    index + 1
                );
            }
        }
    }

    let violations = scan_type_bodies(&paths, &workspace_root, OPT_OUT_MARKER, TOKEN);

    assert!(
        violations.is_empty(),
        "\n`SteelVal` stored in a type: the collector cannot see it, so the heap \
         cells it captures are freed by the next full collection.\n\
         Store a `hume_scripting::Rooted` instead.\n\
         Opt out with `// steel-val-transient: <reason>` only for a type whose \
         values never live across a Steel evaluation.\n\
         Violations:\n{}\n",
        violations
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
