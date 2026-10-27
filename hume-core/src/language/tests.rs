use std::path::Path;

use super::*;

fn matches(pattern: &str, path: &str) -> bool {
    let glob = FileGlob::new(pattern).expect("a valid glob");
    glob.glob().compile_matcher().is_match(Path::new(path))
}

#[test]
fn a_relative_glob_matches_the_last_components_of_any_path() {
    assert!(matches("Makefile", "/x/Makefile"));
    assert!(matches("Makefile", "/x/y/Makefile"));
    assert!(!matches("Makefile", "/x/GNUMakefile"));
}

#[test]
fn an_absolute_glob_matches_only_from_the_root() {
    assert!(matches("/etc/hosts", "/etc/hosts"));
    assert!(!matches("/etc/hosts", "/x/etc/hosts"));
}

#[test]
fn a_glob_starting_at_any_directory_matches_like_its_relative_form() {
    for path in ["/x/foo", "/x/y/foo", "/x/afoo"] {
        assert_eq!(matches("*/foo", path), matches("foo", path), "{path}");
    }
}

#[test]
fn an_extension_is_dot_separated_names_with_no_separator() {
    for invalid in ["", ".rs", "rs.", "a..b", "a/b", r"a\b"] {
        assert_eq!(
            FileExtension::new(invalid.to_owned()),
            Err(InvalidExtension(invalid.to_owned())),
            "{invalid:?}"
        );
    }
    for valid in ["rs", "js.map", "d.ts"] {
        assert_eq!(
            FileExtension::new(valid.to_owned()).map(|e| e.as_str().to_owned()),
            Ok(valid.to_owned())
        );
    }
}

#[test]
fn a_shebang_interpreter_is_one_name_with_no_space_or_separator() {
    for invalid in ["", "v run", "bin/python", r"bin\python", " python"] {
        assert_eq!(
            ShebangInterpreter::new(invalid.to_owned()),
            Err(InvalidShebangInterpreter(invalid.to_owned())),
            "{invalid:?}"
        );
    }
    for valid in ["python", "python3.12", "octave-cli", "R"] {
        assert_eq!(
            ShebangInterpreter::new(valid.to_owned()).map(|i| i.as_str().to_owned()),
            Ok(valid.to_owned())
        );
    }
}
