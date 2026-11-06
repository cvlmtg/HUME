use std::os::unix::fs::PermissionsExt;

use super::super::session::{PERSIST, Project, quit};
use super::*;
use pretty_assertions::assert_eq;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_saved_session_is_readable_by_its_owner_only() {
    let project = Project::new(&[]);
    let mut ed = project.start(&[], PERSIST);
    quit(&mut ed).expect("the session saves");

    let session = project.session_file();
    assert_eq!(mode(&session), 0o600);
    let state = project.state();
    for dir in session
        .ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(&state))
    {
        assert_eq!(mode(dir), 0o700, "{}", dir.display());
    }
}

#[test]
fn a_symlink_to_the_session_root_keys_the_same_session() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let link = project.work().with_file_name("link");
    std::os::unix::fs::symlink(project.work(), &link).unwrap();
    let init = format!(
        "{PERSIST}\n(set-option! \"session-root\" {})",
        steel_path(&link)
    );
    let mut ed = project.start(&[], &init);
    type_cmd(&mut ed, ":e a.txt");
    quit(&mut ed).expect("the session saves");

    assert!(project.session_file().is_file());
    let again = project.start(&[], PERSIST);
    assert_eq!(again.doc().display_name(), "a.txt");
}
