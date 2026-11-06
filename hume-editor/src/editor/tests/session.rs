use std::path::{Path, PathBuf};

use super::*;
use pretty_assertions::assert_eq;

/// A project directory holding files, beside a state and a config
/// directory, all in one temp dir.
pub(super) struct Project {
    dir: tempfile::TempDir,
}

/// The `init.scm` that turns sessions on.
pub(super) const PERSIST: &str = r#"(set-option! "session-persist" #t)"#;

impl Project {
    pub(super) fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = Self { dir };
        std::fs::create_dir(project.work()).unwrap();
        std::fs::create_dir(project.dir.path().join("config")).unwrap();
        for (name, content) in files {
            project.write(name, content);
        }
        project
    }

    pub(super) fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    pub(super) fn file(&self, name: &str) -> PathBuf {
        self.work().join(name)
    }

    pub(super) fn write(&self, name: &str, content: &str) {
        std::fs::write(self.file(name), content).unwrap();
    }

    pub(super) fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// Where the session of `dir` lands under the default directory.
    pub(super) fn session_of(&self, dir: &Path) -> PathBuf {
        let resolved = AbsolutePath::canonicalize(dir).unwrap();
        hume_platform::path::mirrored_under(&self.state().join("sessions"), &resolved, ".session")
            .unwrap()
    }

    pub(super) fn session_file(&self) -> PathBuf {
        self.session_of(&self.work())
    }

    /// The editor `hume <args>` starts in the project with `init` as its
    /// `init.scm`, settled as before the first key.
    pub(super) fn start(&self, args: &[&str], init: &str) -> Editor {
        let config = self.dir.path().join("config");
        std::fs::write(config.join("init.scm"), init).unwrap();
        let cwd = AbsolutePath::canonicalize(&self.work()).unwrap();
        let dirs = Dirs {
            config: Some(config),
            state: Some(self.state()),
            ..Dirs::none()
        };
        let args: Vec<_> = args.iter().map(|name| self.file(name)).collect();
        let mut started = start_headless(&args, &cwd, dirs, crate::cli::ConfigSource::Default);
        started.settle();
        started
    }
}

/// Quit `ed` with `:qa` and save its session, as `hume_editor::run` does
/// once the run loop ends.
pub(super) fn quit(ed: &mut Editor) -> Result<(), String> {
    type_cmd(ed, ":qa");
    assert!(ed.state.should_quit, "sanity: :qa quits");
    ed.save_session()
}

#[test]
fn quitting_a_run_started_with_no_files_saves_its_session() {
    let project = Project::new(&[]);
    let mut ed = project.start(&[], PERSIST);
    quit(&mut ed).expect("the session saves");

    let bytes = std::fs::read(project.session_file()).expect("a session file");
    let mut header = b"HUMESESS".to_vec();
    header.extend_from_slice(&1u32.to_le_bytes());
    assert_eq!(bytes[..header.len()], header[..]);
}

#[test]
fn a_run_started_with_a_file_saves_no_session() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let mut ed = project.start(&["a.txt"], PERSIST);
    quit(&mut ed).expect("nothing to save");
    assert!(!project.state().join("sessions").exists());
}

#[test]
fn with_session_persist_off_no_session_is_saved() {
    let project = Project::new(&[]);
    let mut ed = project.start(&[], "");
    quit(&mut ed).expect("nothing to save");
    assert!(!project.state().join("sessions").exists());
}

#[test]
fn session_directory_moves_where_the_session_is_saved() {
    let project = Project::new(&[]);
    let elsewhere = project.dir.path().join("elsewhere");
    let init = format!(
        "{PERSIST}\n(set-option! \"session-directory\" {})",
        steel_path(&elsewhere)
    );
    let mut ed = project.start(&[], &init);
    quit(&mut ed).expect("the session saves");

    let cwd = AbsolutePath::canonicalize(&project.work()).unwrap();
    let moved = hume_platform::path::mirrored_under(&elsewhere, &cwd, ".session").unwrap();
    assert!(moved.is_file());
    assert!(!project.state().join("sessions").exists());
}

#[test]
fn the_session_is_saved_under_the_working_directory_at_quit() {
    let project = Project::new(&[]);
    let sub = project.work().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":cd sub");
    quit(&mut ed).expect("the session saves");

    assert!(project.session_of(&sub).is_file());
    assert!(!project.session_file().exists());
}

#[test]
fn a_session_that_cannot_be_written_is_reported_on_exit() {
    let project = Project::new(&[]);
    std::fs::write(project.state(), "a file where the directory goes").unwrap();
    let mut ed = project.start(&[], PERSIST);
    let saved = quit(&mut ed);
    assert!(saved.is_err());

    let mut out = Vec::new();
    crate::editor::session::report_unsaved(&mut out, &saved);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.starts_with("hume: session not saved: "),
        "reported: {out}"
    );
}
