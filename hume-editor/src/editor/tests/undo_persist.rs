use std::path::{Path, PathBuf};

use super::before_save::warning_texts;
use super::*;
use pretty_assertions::assert_eq;

/// A file holding `content` and a state directory, both in one temp dir.
pub(super) struct Disk {
    pub(super) dir: tempfile::TempDir,
    pub(super) file: PathBuf,
}

impl Disk {
    pub(super) fn new(content: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, content).unwrap();
        Self { dir, file }
    }

    pub(super) fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// Where the saved history of `file` lands.
    pub(super) fn undo_file(&self) -> PathBuf {
        let resolved = AbsolutePath::canonicalize(&self.file).unwrap();
        hume_platform::path::mirrored_under(&self.state().join("undo"), &resolved, ".undo").unwrap()
    }
}

/// An editor with `disk`'s state directory, `settings` applied, showing
/// `file` opened the way `:e` opens it.
pub(super) fn editor_on(disk: &Disk, file: &Path, settings: &[&str]) -> Editor {
    let mut ed = editor_from("-[x]>\n");
    ed.state.dirs.state = Some(disk.state());
    for arg in settings {
        run_set(&mut ed, arg).expect("the setting applies");
    }
    let bid = open_path(&mut ed, file);
    ed.enter_buffer(FocusedPane::current(&ed.view), bid);
    ed
}

#[test]
fn reopening_a_saved_file_brings_back_its_undo_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    let first = text(&ed);
    type_text(&mut ed, "b");
    let second = text(&ed);
    type_cmd(&mut ed, ":w");

    let mut again = editor_on(&disk, &disk.file, &[]);
    assert_eq!(text(&again), second);
    assert!(!again.state.has_unsaved_changes(again.focused_buffer_id()));
    again.feed_key(key('u'));
    assert_eq!(text(&again), first);
    again.feed_key(key('u'));
    assert_eq!(text(&again), "one\n");
    again.feed_key(key('U'));
    assert_eq!(text(&again), first);
    assert!(
        warning_texts(&again).is_empty(),
        "{:?}",
        warning_texts(&again)
    );
}

#[test]
fn saving_with_undo_persist_off_keeps_no_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &[]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    assert!(!disk.state().join("undo").exists());
    assert!(!editor_on(&disk, &disk.file, &[]).doc().can_undo());
}

#[test]
fn turning_undo_persist_on_for_one_buffer_saves_its_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &[]);
    run_set(&mut ed, "buffer undo-persist=true").expect("the setting applies");
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    let mut again = editor_on(&disk, &disk.file, &[]);
    again.feed_key(key('u'));
    assert_eq!(text(&again), "one\n");
}

#[test]
fn a_file_changed_elsewhere_opens_without_history_or_warning() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");
    std::fs::write(&disk.file, "changed elsewhere\n").unwrap();

    let again = editor_on(&disk, &disk.file, &[]);
    assert!(!again.doc().can_undo());
    assert!(
        warning_texts(&again).is_empty(),
        "{:?}",
        warning_texts(&again)
    );
}

#[test]
fn a_damaged_undo_file_warns_and_the_file_opens_with_fresh_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");
    let saved = std::fs::read(disk.undo_file()).unwrap();
    std::fs::write(disk.undo_file(), &saved[..saved.len() / 2]).unwrap();

    let again = editor_on(&disk, &disk.file, &[]);
    assert!(!again.doc().can_undo());
    let warned = warning_texts(&again);
    assert!(
        warned.iter().any(|w| w.contains("undo history")),
        "{warned:?}"
    );
}

#[test]
fn a_damaged_undo_file_of_a_file_changed_elsewhere_opens_without_warning() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");
    std::fs::write(&disk.file, "changed elsewhere\n").unwrap();
    let saved = std::fs::read(disk.undo_file()).unwrap();
    std::fs::write(disk.undo_file(), &saved[..saved.len() - 1]).unwrap();

    let again = editor_on(&disk, &disk.file, &[]);
    assert!(!again.doc().can_undo());
    assert!(
        warning_texts(&again).is_empty(),
        "{:?}",
        warning_texts(&again)
    );
}

#[test]
fn a_file_that_is_no_undo_history_warns() {
    let disk = Disk::new("one\n");
    std::fs::create_dir_all(disk.undo_file().parent().unwrap()).unwrap();
    std::fs::write(disk.undo_file(), b"not an undo history").unwrap();

    let again = editor_on(&disk, &disk.file, &[]);
    assert!(!again.doc().can_undo());
    let warned = warning_texts(&again);
    assert!(
        warned
            .iter()
            .any(|w| w.contains("undo history not restored")),
        "{warned:?}"
    );
}

#[test]
fn a_reload_from_disk_saves_the_undo_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    let edited = text(&ed);
    type_cmd(&mut ed, ":w");
    std::fs::write(&disk.file, "two\n").unwrap();
    ed.execute_typed("e!", None).expect(":e! reloads the file");

    let mut again = editor_on(&disk, &disk.file, &[]);
    assert_eq!(text(&again), "two\n");
    again.feed_key(key('u'));
    assert_eq!(text(&again), edited);
}

/// Edit `two` in `disk`'s file to `X` and save it with `undo-persist` on,
/// then walk selection undo once: the state that walk reaches.
fn saved_selection_walk(disk: &Disk) -> String {
    let mut ed = editor_on(disk, &disk.file, &["global undo-persist=true"]);
    select(&mut ed, &[(4, 6)], 0);
    type_text(&mut ed, "X");
    type_cmd(&mut ed, ":w");
    selection_undo(&mut ed);
    state(&ed)
}

fn selection_undo(ed: &mut Editor) {
    ed.feed_key(key('m'));
    ed.feed_key(key('u'));
}

#[test]
fn selection_undo_reaches_the_selections_of_the_saved_edits() {
    let disk = Disk::new("one two\n");
    let walked = saved_selection_walk(&disk);

    let mut again = editor_on(&disk, &disk.file, &[]);
    selection_undo(&mut again);
    assert_eq!(state(&again), walked);
}

#[test]
fn a_split_opened_on_a_saved_file_reaches_its_selection_history() {
    let disk = Disk::new("one two\n");
    let walked = saved_selection_walk(&disk);

    let mut split = editor_from("-[x]>\n");
    split.state.dirs.state = Some(disk.state());
    split
        .execute_typed(&format!("vsplit {}", disk.file.display()), None)
        .expect("the split opens the file");
    selection_undo(&mut split);
    assert_eq!(state(&split), walked);
}

#[test]
fn undo_directory_moves_where_history_is_saved() {
    let disk = Disk::new("one\n");
    let elsewhere = disk.dir.path().join("elsewhere");
    let setting = format!("global undo-directory={}", elsewhere.display());
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true", &setting]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    assert!(elsewhere.is_dir());
    assert!(!disk.state().join("undo").exists());
    let mut again = editor_on(&disk, &disk.file, &[&setting]);
    again.feed_key(key('u'));
    assert_eq!(text(&again), "one\n");
}

/// The editor `hume <args>` starts in `disk`'s directory, with its state
/// directory and no config, settled as before the first key.
fn started_on(disk: &Disk, args: &[impl AsRef<Path>]) -> Editor {
    let cwd = AbsolutePath::canonicalize(disk.dir.path()).unwrap();
    let dirs = hume_platform::dirs::Dirs {
        state: Some(disk.state()),
        ..hume_platform::dirs::Dirs::none()
    };
    let mut started = start_headless(args, &cwd, dirs, crate::cli::ConfigSource::Skip);
    started.settle();
    started
}

#[test]
fn the_file_hume_starts_on_brings_back_its_history() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    let mut started = started_on(&disk, &[&disk.file]);
    started.feed_key(key('u'));
    assert_eq!(text(&started), "one\n");
}

#[test]
fn selection_undo_on_the_file_hume_starts_on_reaches_the_saved_selections() {
    let disk = Disk::new("one two\n");
    let walked = saved_selection_walk(&disk);

    let mut started = started_on(&disk, &[&disk.file]);
    selection_undo(&mut started);
    assert_eq!(state(&started), walked);
}

/// `hume other.txt <disk's file>:1` parks the file's cursor in the pane
/// showing `other.txt`; the pane that shows the file is the one that walks
/// its saved selections.
#[test]
fn a_cursor_parked_for_a_saved_file_leaves_its_selection_history_to_the_pane_showing_it() {
    let disk = Disk::new("one two\n");
    let walked = saved_selection_walk(&disk);

    std::fs::write(disk.dir.path().join("other.txt"), "other\n").unwrap();
    let parked = PathBuf::from(format!("{}:1", disk.file.display()));
    let mut started = started_on(&disk, &[Path::new("other.txt"), &parked]);
    started
        .execute_typed(&format!("vsplit {}", disk.file.display()), None)
        .expect("the split opens the file");
    selection_undo(&mut started);
    assert_eq!(state(&started), walked);
}

#[test]
fn a_save_as_saves_the_history_under_the_new_file() {
    let disk = Disk::new("one\n");
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true"]);
    type_text(&mut ed, "a");
    let edited = text(&ed);
    let other = disk.dir.path().join("other.txt");
    type_cmd(&mut ed, &format!(":w {}", other.display()));

    let mut again = editor_on(&disk, &other, &[]);
    assert_eq!(text(&again), edited);
    again.feed_key(key('u'));
    assert_eq!(text(&again), "one\n");
}

#[test]
fn a_history_that_cannot_be_saved_leaves_its_warning_on_the_statusline() {
    let disk = Disk::new("one\n");
    let blocker = disk.dir.path().join("blocker");
    std::fs::write(&blocker, "").unwrap();
    let setting = format!("global undo-directory={}", blocker.join("undo").display());
    let mut ed = editor_on(&disk, &disk.file, &["global undo-persist=true", &setting]);
    type_text(&mut ed, "a");
    type_cmd(&mut ed, ":w");

    assert_eq!(std::fs::read_to_string(&disk.file).unwrap(), text(&ed));
    let status = ed.state.status_msg.text().unwrap_or_default();
    assert!(status.starts_with("undo history not saved: "), "{status:?}");
}
