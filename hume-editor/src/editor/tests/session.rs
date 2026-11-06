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
        self.start_in(&self.work(), args, init)
    }

    /// [`Self::start`] in `dir` instead of the project directory, with the
    /// bundled runtime, so `init` can load core plugins.
    pub(super) fn start_in(&self, dir: &Path, args: &[&str], init: &str) -> Editor {
        let config = self.dir.path().join("config");
        std::fs::write(config.join("init.scm"), init).unwrap();
        let cwd = AbsolutePath::canonicalize(dir).unwrap();
        let dirs = Dirs {
            config: Some(config),
            state: Some(self.state()),
            runtime: Some(repo_runtime_dir()),
            search_path: std::env::var_os("PATH"),
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
fn a_cd_before_quitting_keeps_the_session_at_the_directory_hume_started_in() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let sub = project.work().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":cd sub");
    quit(&mut ed).expect("the session saves");

    assert!(!project.session_of(&sub).exists());
    let again = project.start(&[], PERSIST);
    assert_eq!(again.doc().display_name(), "a.txt");
}

#[test]
fn runs_under_one_session_root_share_its_session_and_keep_their_own_cwd() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let sub = project.work().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let init = format!(
        "{PERSIST}\n(set-option! \"session-root\" {})",
        steel_path(&project.work())
    );
    let mut ed = project.start(&[], &init);
    type_cmd(&mut ed, ":e a.txt");
    quit(&mut ed).expect("the session saves");

    let again = project.start_in(&sub, &[], &init);
    assert_eq!(again.doc().display_name(), "a.txt");
    assert_eq!(again.state.cwd, AbsolutePath::canonicalize(&sub).unwrap());
    assert!(!project.session_of(&sub).exists());
}

#[test]
fn the_example_config_keys_the_session_by_the_git_repository() {
    let project = Project::new(&[("a.txt", "a\n")]);
    git_init(&project.work());
    let sub = project.work().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let init = format!(
        "(load-plugin! \"core:stdlib\")\n{PERSIST}\n\
         (set-option! \"session-root\" (or (call! \"stdlib/git-toplevel\") (cwd)))"
    );
    let mut ed = project.start_in(&sub, &[], &init);
    assert!(
        session_warnings(&ed).is_empty(),
        "{:?}",
        session_warnings(&ed)
    );
    type_cmd(&mut ed, ":e ../a.txt");
    quit(&mut ed).expect("the session saves");

    assert!(project.session_file().is_file());
    let again = project.start(&[], &init);
    assert_eq!(again.doc().display_name(), "a.txt");
}

#[test]
fn a_session_root_that_is_not_a_directory_warns_and_saves_nothing() {
    let project = Project::new(&[]);
    let missing = project.work().join("missing");
    let init = format!(
        "{PERSIST}\n(set-option! \"session-root\" {})",
        steel_path(&missing)
    );
    let mut ed = project.start(&[], &init);
    let warnings = session_warnings(&ed);
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("session not restored: ")),
        "{warnings:?}"
    );
    assert!(quit(&mut ed).is_err());
    assert!(!project.state().join("sessions").exists());
}

#[test]
fn reloading_the_config_puts_the_session_root_back_where_hume_started() {
    let project = Project::new(&[]);
    let sub = project.work().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":cd sub");
    run_set(&mut ed, &format!("global session-root={}", sub.display())).unwrap();
    type_cmd(&mut ed, ":reload-config");
    let root = ed.state.settings.session_root.location(&ed.state.cwd);
    assert_eq!(
        root,
        Ok(AbsolutePath::canonicalize(&project.work()).unwrap())
    );
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

// ── Restore ──────────────────────────────────────────────────────────────

/// Each tab's layout, the file every pane shows and the focused pane
/// marked `*`, the current tab marked `>`.
fn layout(ed: &Editor) -> Vec<String> {
    fn tree(ed: &Editor, node: &hume_engine::pipeline::LayoutTree, focus: PaneId) -> String {
        match node {
            hume_engine::pipeline::LayoutTree::Leaf(pid) => {
                let name = ed
                    .state
                    .buffers
                    .get(ed.view.panes[*pid].buffer_id())
                    .display_name();
                if *pid == focus {
                    format!("*{name}")
                } else {
                    name
                }
            }
            hume_engine::pipeline::LayoutTree::Split {
                direction,
                children,
                ..
            } => format!(
                "{direction:?}({}, {})",
                tree(ed, &children.0, focus),
                tree(ed, &children.1, focus)
            ),
        }
    }
    let current = ed.state.tabs.current_pos();
    ed.state
        .tabs
        .layouts(ed.view.focused_layout())
        .enumerate()
        .map(|(n, l)| {
            let mark = if n == current { ">" } else { "" };
            format!("{mark}{}", tree(ed, l.tree(), l.focused()))
        })
        .collect()
}

/// The open buffers' names, in open order.
fn buffer_names(ed: &Editor) -> Vec<String> {
    ed.state
        .buffers
        .iter()
        .map(|(_, buf)| buf.display_name())
        .collect()
}

fn alternate(ed: &Editor) -> Option<String> {
    let bid = ed.state.buffers.second_most_recent()?;
    Some(ed.state.buffers.get(bid).display_name())
}

/// Quit `ed`, then start again in the same project the way `hume` does.
fn restart(project: &Project, mut ed: Editor) -> Editor {
    quit(&mut ed).expect("the session saves");
    project.start(&[], PERSIST)
}

fn session_warnings(ed: &Editor) -> Vec<String> {
    super::before_save::warning_texts(ed)
        .into_iter()
        .filter(|w| w.contains("session"))
        .collect()
}

fn lines(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn buffers_come_back_in_open_order_with_the_same_alternate() {
    let project = Project::new(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":e b.txt");
    type_cmd(&mut ed, ":e c.txt");
    type_cmd(&mut ed, ":e a.txt");
    assert_eq!(alternate(&ed).as_deref(), Some("c.txt"), "sanity");

    let again = restart(&project, ed);
    assert_eq!(buffer_names(&again), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(again.doc().display_name(), "a.txt");
    assert_eq!(alternate(&again).as_deref(), Some("c.txt"));
    assert!(session_warnings(&again).is_empty());
}

#[test]
fn tabs_splits_and_focus_come_back() {
    let project = Project::new(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":vsplit b.txt");
    type_cmd(&mut ed, ":split c.txt");
    type_cmd(&mut ed, ":tabnew b.txt");
    type_cmd(&mut ed, ":tabnew c.txt");
    type_cmd(&mut ed, ":vsplit a.txt");
    let unrotated = layout(&ed);
    ed.feed_key(key_ctrl('p'));
    ed.feed_key(key('r'));
    assert_ne!(layout(&ed), unrotated, "sanity: the rotate moved the panes");
    type_cmd(&mut ed, ":tabprev");
    let before = layout(&ed);
    assert_eq!(before.len(), 3, "sanity: {before:?}");

    let again = restart(&project, ed);
    assert_eq!(layout(&again), before);
}

#[test]
fn selections_scroll_and_wrap_pins_come_back_per_pane() {
    let project = Project::new(&[("a.txt", &lines(200)), ("b.txt", &lines(200))]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e b.txt");
    ed.feed_key(key_ctrl('d'));
    type_cmd(&mut ed, ":e a.txt");
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    press(&mut ed, "C");
    let a_state = state(&ed);
    let a_top = ed.view.panes[ed.view.focused_pane()].viewport.top();
    pin_wrap(&mut ed, hume_engine::pane::WrapMode::Soft { width: 20 });
    let pid = ed.view.focused_pane();
    let bid = ed
        .state
        .buffers
        .find_by_path(&AbsolutePath::canonicalize(&project.file("b.txt")).unwrap())
        .unwrap();
    let b_top = ed.view.panes[pid].saved_scrolls[bid].top();
    assert_ne!(b_top, a_top, "sanity: the two buffers scrolled apart");

    let mut again = restart(&project, ed);
    let pid = again.view.focused_pane();
    assert_eq!(state(&again), a_state);
    assert_eq!(again.view.panes[pid].viewport.top(), a_top);
    assert_eq!(
        again.view.panes[pid].wrap().mode,
        Some(hume_engine::pane::WrapMode::Soft { width: 20 })
    );
    type_cmd(&mut again, ":e b.txt");
    let pid = again.view.focused_pane();
    assert_eq!(again.view.panes[pid].viewport.top(), b_top);
    assert_eq!(again.view.panes[pid].wrap().mode, None);
}

#[test]
fn the_jump_list_comes_back_where_it_stood() {
    let project = Project::new(&[("a.txt", &lines(100))]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":50");
    type_cmd(&mut ed, ":90");
    type_cmd(&mut ed, ":20");
    ed.feed_key(key_ctrl('o'));
    let back = state(&ed);

    let mut again = restart(&project, ed);
    assert_eq!(state(&again), back);
    again.feed_key(key_ctrl('o'));
    let older = state(&again);
    again.feed_key(key_ctrl('i'));
    assert_eq!(state(&again), back);
    again.feed_key(key_ctrl('i'));
    assert_ne!(state(&again), back);
    assert_ne!(state(&again), older);
}

#[test]
fn macros_and_numbered_registers_come_back() {
    let project = Project::new(&[]);
    let mut ed = project.start(&[], PERSIST);
    let keys = vec![
        key_ctrl('w'),
        key_esc(),
        key_shift_tab(),
        KeyEvent::from(KeyCode::Function(5)),
        key('é'),
    ];
    ed.state.registers.write_macro('q', keys.clone());
    let pieces = vec![
        hume_ops::register::Piece::new("one\n", hume_ops::register::Shape::Linewise),
        hume_ops::register::Piece::new("two", hume_ops::register::Shape::Charwise),
    ];
    ed.state.registers.write_text('3', pieces.clone());
    ed.state.registers.write_macro('7', vec![key('x')]);

    let again = restart(&project, ed);
    let read = |name| again.state.registers.read(name);
    assert_eq!(read('q').and_then(|r| r.as_macro()), Some(&keys[..]));
    assert_eq!(read('3').and_then(|r| r.as_pieces()), Some(&pieces[..]));
    assert_eq!(read('7').and_then(|r| r.as_macro()), Some(&[key('x')][..]));
}

/// Every prompt's history, oldest entry first, in `entries_by_kind` order.
fn histories(ed: &Editor) -> Vec<Vec<String>> {
    ed.state
        .history
        .entries_by_kind()
        .iter()
        .map(|(_, entries)| entries.iter().cloned().collect())
        .collect()
}

#[test]
fn prompt_history_and_the_last_search_come_back() {
    let project = Project::new(&[("a.txt", "one two\nthree two\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, "?one");
    type_cmd(&mut ed, "/two");
    ed.state
        .history
        .get_mut(crate::editor::minibuf::history::HistoryKind::Pipe)
        .push("sort".to_owned());
    quit(&mut ed).expect("the session saves");
    let saved = histories(&ed);
    assert!(
        saved.iter().all(|ring| !ring.is_empty()),
        "sanity: {saved:?}"
    );

    let mut again = project.start(&[], PERSIST);
    assert_eq!(histories(&again), saved);
    assert_eq!(again.state.registers.search_register(), Some("two"));
    let before = state(&again);
    again.feed_key(key('n'));
    assert_ne!(state(&again), before);
}

#[test]
fn a_restored_file_keeps_its_saved_undo_history() {
    let project = Project::new(&[("a.txt", "one\n")]);
    let init = format!("{PERSIST}\n(set-option! \"undo-persist\" #t)");
    let mut ed = project.start(&[], &init);
    type_cmd(&mut ed, ":e a.txt");
    type_text(&mut ed, "x");
    type_cmd(&mut ed, ":w");
    quit(&mut ed).expect("the session saves");

    let mut again = project.start(&[], &init);
    assert_eq!(text(&again), "xone\n");
    again.feed_key(key('u'));
    assert_eq!(text(&again), "one\n");
}

#[test]
fn the_scratch_buffer_hume_starts_on_is_replaced() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    let again = restart(&project, ed);
    assert!(
        again
            .state
            .buffers
            .iter()
            .all(|(_, buf)| buf.file().is_some())
    );
}

// ── When nothing is restored ─────────────────────────────────────────────

/// A project whose saved session shows `a.txt` beside `b.txt`.
fn saved_two_files() -> Project {
    let project = Project::new(&[("a.txt", &lines(100)), ("b.txt", &lines(100))]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":vsplit b.txt");
    type_cmd(&mut ed, ":50");
    quit(&mut ed).expect("the session saves");
    project
}

#[test]
fn a_run_started_with_a_file_restores_nothing() {
    let project = saved_two_files();
    let ed = project.start(&["a.txt"], PERSIST);
    assert_eq!(buffer_names(&ed), ["a.txt"]);
    assert_eq!(layout(&ed), [">*a.txt"]);
}

#[test]
fn with_session_persist_off_nothing_is_restored() {
    let project = saved_two_files();
    let ed = project.start(&[], "");
    assert!(ed.state.buffers.iter().all(|(_, buf)| buf.file().is_none()));
    assert!(
        project.session_file().is_file(),
        "turning it off deletes nothing"
    );
}

#[test]
fn a_damaged_session_warns_and_starts_fresh() {
    let project = saved_two_files();
    let mut bytes = std::fs::read(project.session_file()).unwrap();
    bytes.truncate(bytes.len() - 3);
    std::fs::write(project.session_file(), &bytes).unwrap();

    let ed = project.start(&[], PERSIST);
    assert!(ed.state.buffers.iter().all(|(_, buf)| buf.file().is_none()));
    let warnings = session_warnings(&ed);
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("session not restored: ")),
        "{warnings:?}"
    );
}

#[test]
fn a_file_that_is_not_a_session_warns() {
    let project = saved_two_files();
    std::fs::write(project.session_file(), "not a session").unwrap();
    let ed = project.start(&[], PERSIST);
    assert!(
        session_warnings(&ed)
            .iter()
            .any(|w| w.contains("not a session file")),
        "{:?}",
        session_warnings(&ed)
    );
}

#[test]
fn a_session_of_another_format_is_ignored_without_a_warning() {
    let project = saved_two_files();
    let mut bytes = std::fs::read(project.session_file()).unwrap();
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    std::fs::write(project.session_file(), &bytes).unwrap();

    let ed = project.start(&[], PERSIST);
    assert!(ed.state.buffers.iter().all(|(_, buf)| buf.file().is_none()));
    assert!(session_warnings(&ed).is_empty());
}

#[test]
fn a_file_changed_since_the_save_opens_at_its_start() {
    let project = saved_two_files();
    project.write("b.txt", &lines(120));
    let mut ed = project.start(&[], PERSIST);
    assert_eq!(layout(&ed), [">Horizontal(a.txt, *b.txt)"]);
    assert_eq!(ed.current_view().primary().head().offset(), co(0));
    assert_eq!(
        ed.view.panes[ed.view.focused_pane()].viewport.top(),
        Default::default()
    );
    type_cmd(&mut ed, ":e a.txt");
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(0),
        "a.txt never moved"
    );
}

#[test]
fn a_missing_file_is_skipped_and_its_pane_shows_another() {
    let project = saved_two_files();
    std::fs::remove_file(project.file("b.txt")).unwrap();
    let ed = project.start(&[], PERSIST);
    assert_eq!(buffer_names(&ed), ["a.txt"]);
    assert_eq!(layout(&ed), [">Horizontal(a.txt, *a.txt)"]);
    let (jumps, _) = ed.state.panes.entries[ed.view.focused_pane()]
        .jumps
        .history();
    assert!(jumps.is_empty());
    assert!(session_warnings(&ed).is_empty());
}

#[test]
fn with_every_file_gone_the_scratch_stays_and_registers_come_back() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    ed.state.registers.write_macro('q', vec![key('x')]);
    quit(&mut ed).expect("the session saves");
    std::fs::remove_file(project.file("a.txt")).unwrap();

    let again = project.start(&[], PERSIST);
    assert!(
        again
            .state
            .buffers
            .iter()
            .all(|(_, buf)| buf.file().is_none())
    );
    assert_eq!(
        again.state.registers.read('q').and_then(|r| r.as_macro()),
        Some(&[key('x')][..])
    );
}

// ── :save-session ────────────────────────────────────────────────────────

#[test]
fn save_session_starts_a_session_in_a_run_started_with_a_file() {
    let project = Project::new(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
    let mut ed = project.start(&["a.txt"], PERSIST);
    type_cmd(&mut ed, ":e b.txt");
    type_cmd(&mut ed, ":save-session");
    assert!(project.session_file().is_file());
    assert_eq!(status(&ed), "session saved");
    type_cmd(&mut ed, ":e c.txt");
    quit(&mut ed).expect("quitting saves the session it started");

    let again = project.start(&[], PERSIST);
    assert_eq!(buffer_names(&again), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(again.doc().display_name(), "c.txt");
}

#[test]
fn save_session_is_refused_with_session_persist_off() {
    let project = Project::new(&[("a.txt", "a\n")]);
    let mut ed = project.start(&["a.txt"], "");
    type_cmd(&mut ed, ":save-session");
    assert!(has_error(&ed, "session-persist is off"), "{}", status(&ed));
    quit(&mut ed).expect("nothing to save");
    assert!(!project.state().join("sessions").exists());
}

#[test]
fn save_session_in_a_session_run_saves_it_now_and_again() {
    let project = Project::new(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":e a.txt");
    type_cmd(&mut ed, ":save-session");
    type_cmd(&mut ed, ":e b.txt");
    type_cmd(&mut ed, ":save-session");
    assert!(session_warnings(&ed).is_empty());

    // Not quitting: the last manual save is what the next run sees.
    let again = project.start(&[], PERSIST);
    assert_eq!(again.doc().display_name(), "b.txt");
}

#[test]
fn save_session_reports_a_session_it_cannot_write() {
    let project = Project::new(&[]);
    std::fs::write(project.state(), "a file where the directory goes").unwrap();
    let mut ed = project.start(&[], PERSIST);
    type_cmd(&mut ed, ":save-session");
    assert!(has_error(&ed, "save-session: "), "{}", status(&ed));
}

#[test]
fn a_failed_save_session_starts_no_session() {
    let project = Project::new(&[("a.txt", "a\n")]);
    std::fs::write(project.state(), "a file where the directory goes").unwrap();
    let mut ed = project.start(&["a.txt"], PERSIST);
    type_cmd(&mut ed, ":save-session");
    assert!(has_error(&ed, "save-session: "), "{}", status(&ed));
    std::fs::remove_file(project.state()).unwrap();
    quit(&mut ed).expect("a run started with a file keeps no session");
    assert!(!project.session_file().exists());
}

#[test]
fn a_script_cannot_save_the_session() {
    let project = Project::new(&[]);
    let mut ed = project.start(&[], PERSIST);
    run(
        &mut ed,
        &project.work(),
        r#"(define-command! "try-save" "" (lambda (pane) (call! "save-session")))"#,
    );
    call(&mut ed, "try-save");
    assert!(!project.session_file().exists());
    let log: Vec<_> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.clone())
        .collect();
    assert!(log.iter().any(|t| t.contains("save-session")), "{log:#?}");
}
