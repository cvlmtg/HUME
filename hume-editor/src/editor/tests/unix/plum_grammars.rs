//! `core:plum`'s grammar install pipeline: index, ranged fetch, digest,
//! unpack, manifest checks and registration.
//!
//! The tests install from a local folder of `<language>.tar.gz` archives:
//! the fixture archives `scripts/fetch-test-grammars.sh` fetches, a copy of
//! them edited where the test needs a broken one, or one archive repacked
//! from a fixture. A `curl` shim serves the fixture bundle the script packs
//! as the release. The last test installs from the pinned release over the
//! network.

use super::*;
use crate::editor::tests::before_save::warning_texts;
use hume_platform::path::test_support::write_script;
use test_fixtures::{
    fixture_archive_dir, fixture_bundle_dir, grammar_platform_ext, require_grammars,
};

/// The release tag `core:plum` pins: the one string in its pin file.
fn release_tag() -> &'static str {
    include_str!("../../../../../runtime/plugins/core/plum/data/treesitter-hume-pin.scm")
        .lines()
        .find_map(|line| line.strip_prefix('"')?.strip_suffix('"'))
        .expect("the pin file holds one quoted tag")
}

fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn index_path(bundle: &Path) -> PathBuf {
    bundle.join(format!("grammars-{}.index", platform()))
}

fn tar_path(bundle: &Path) -> PathBuf {
    bundle.join(format!("grammars-{}.tar", platform()))
}

/// A copy of the fixture bundle under `work`, for a test to edit.
fn bundle_copy(work: &Path) -> PathBuf {
    let bundle = work.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    for path in [index_path, tar_path] {
        std::fs::copy(path(&fixture_bundle_dir()), path(&bundle)).unwrap();
    }
    bundle
}

fn archive_path(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.tar.gz"))
}

/// A copy of the fixture archive folder under `work`, for a test to edit.
fn folder_copy(work: &Path) -> PathBuf {
    let folder = work.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    for entry in std::fs::read_dir(fixture_archive_dir()).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, folder.join(path.file_name().unwrap())).unwrap();
    }
    folder
}

/// Puts `from`'s archive of `name` in place of `folder`'s.
fn replace_archive(folder: &Path, from: &Path, name: &str) {
    std::fs::copy(archive_path(from, name), archive_path(folder, name)).unwrap();
}

/// Rewrites the line of `bundle`'s index that starts with `prefix` through
/// `edit`.
fn edit_index_line(bundle: &Path, prefix: &str, edit: impl Fn(&str) -> String) {
    let index = std::fs::read_to_string(index_path(bundle)).unwrap();
    let edited: String = index
        .lines()
        .map(|line| {
            let line = if line.starts_with(prefix) {
                edit(line)
            } else {
                line.to_owned()
            };
            line + "\n"
        })
        .collect();
    std::fs::write(index_path(bundle), edited).unwrap();
}

/// A json manifest naming a symbol its library does not export.
fn wrong_symbol(manifest: String) -> String {
    manifest.replace(
        "(symbol . \"tree_sitter_json\")",
        "(symbol . \"tree_sitter_nosuch\")",
    )
}

/// A folder under `work` holding only `name`'s fixture archive, repacked
/// with its manifest passed through `edit`.
fn one_archive_folder(work: &Path, name: &str, edit: impl Fn(String) -> String) -> PathBuf {
    archive_folder(work, name, |grammar| {
        let manifest = grammar.join("manifest.scm");
        std::fs::write(&manifest, edit(std::fs::read_to_string(&manifest).unwrap())).unwrap();
    })
}

/// A folder under `work` holding only `name`'s fixture archive, repacked
/// after `edit` changes the unpacked archive.
fn archive_folder(work: &Path, name: &str, edit: impl Fn(&Path)) -> PathBuf {
    let grammar = work.join("unpacked").join(name);
    test_fixtures::copy_fixture_grammar(&grammar, name);
    edit(&grammar);

    let folder = work.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let library = format!("{name}.{}", grammar_platform_ext());
    tar_gz(
        &grammar,
        &["manifest.scm", &library, "queries"],
        &archive_path(&folder, name),
    );
    folder
}

/// A JSON buffer with PLUM loaded, installing from the local folder `folder`.
fn plum_from_folder(data_tmp: &Path, folder: &Path) -> Editor {
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    load_plum_with(
        &mut ed,
        data_tmp,
        "",
        Some(&format!("(hash 'grammar-source {})", steel_path(folder))),
    );
    type_cmd(&mut ed, ":set buffer language=json");
    ed
}

/// An editor with PLUM loaded and no `grammar-source`, so every grammar comes
/// from `release`.
fn plum_from_release(data_tmp: &Path, release: &ReleaseShim) -> Editor {
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp);
    release.install(&mut ed);
    ed
}

fn grammar_dir(data_tmp: &Path, name: &str) -> PathBuf {
    canonical_data_dir(data_tmp).join("grammars").join(name)
}

/// `name`'s install folders, oldest first.
fn installs(data_tmp: &Path, name: &str) -> Vec<String> {
    let mut installs: Vec<String> = entry_names(&grammar_dir(data_tmp, name))
        .into_iter()
        .filter(|entry| entry.parse::<u64>().is_ok())
        .collect();
    installs.sort_by_key(|entry| entry.parse::<u64>().unwrap());
    installs
}

/// `name`'s newest install folder.
fn current_install(data_tmp: &Path, name: &str) -> Option<PathBuf> {
    installs(data_tmp, name)
        .pop()
        .map(|install| grammar_dir(data_tmp, name).join(install))
}

fn installed(data_tmp: &Path, name: &str) -> bool {
    current_install(data_tmp, name).is_some_and(|dir| {
        dir.join(format!("{name}.{}", grammar_platform_ext()))
            .exists()
    })
}

/// The yaml fixture, renamed in `grammar` as the json grammar: a json
/// grammar whose library is another one.
fn yaml_as_json(grammar: &Path) {
    std::fs::remove_dir_all(grammar).unwrap();
    test_fixtures::copy_fixture_grammar(grammar, "yaml");
    let ext = grammar_platform_ext();
    std::fs::rename(
        grammar.join(format!("yaml.{ext}")),
        grammar.join(format!("json.{ext}")),
    )
    .unwrap();
    let manifest = grammar.join("manifest.scm");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("(name . \"yaml\")", "(name . \"json\")"),
    )
    .unwrap();
}

/// Whether the session's json grammar is the yaml library [`yaml_as_json`]
/// installs.
fn json_runs_the_yaml_library(ed: &mut Editor) -> bool {
    let languages = &mut ed.state.config.languages;
    let id = languages.id_of("json").expect("json is a language");
    languages
        .load_grammar(id, &mut ed.view.registry)
        .is_some_and(|bundle| {
            bundle
                .grammar
                .language()
                .id_for_node_kind("block_mapping", true)
                != 0
        })
}

fn staging_dir(data_tmp: &Path) -> PathBuf {
    canonical_data_dir(data_tmp).join("plum-staging/grammars")
}

/// The names of the entries of `dir`, none when it does not exist.
fn entry_names(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("read {dir:?}: {e}"),
    }
}

/// What an install left in PLUM's grammar staging directory.
fn staging_leftovers(data_tmp: &Path) -> Vec<String> {
    entry_names(&staging_dir(data_tmp))
}

/// Runs `plum/ensure-grammars` on `names` from a command.
fn ensure_grammars(ed: &mut Editor, data_tmp: &Path, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|name| format!("{name:?}")).collect();
    run(
        ed,
        data_tmp,
        &format!(
            r#"(define-typed-command! "ensure" "" (lambda () (call! "plum/ensure-grammars" '({}))))"#,
            names.join(" ")
        ),
    );
    type_cmd(ed, ":ensure");
}

fn log(ed: &Editor) -> String {
    ed.state.message_log.format_for_display()
}

#[test]
fn install_from_a_local_folder_unpacks_registers_and_highlights() {
    require_grammars(&["json"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &fixture_archive_dir());
    let bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(installed(data_tmp.path(), "json"), "{}", log(&ed));
    let dir = current_install(data_tmp.path(), "json").unwrap();
    assert!(dir.join("manifest.scm").exists());
    assert!(dir.join("queries/highlights.scm").exists());
    assert!(ed.state.config.languages.has_grammar("json"));
    ed.reparse_stale_buffers();
    assert!(ed.state.buffers.get(bid).syntax.is_some());
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// A `curl` shim standing in for the release: it serves the files of `bundle`
/// by the URL's file name, and records each URL and byte range it is asked
/// for, one `<url> <range>` line each.
struct ReleaseShim {
    dir: tempfile::TempDir,
}

impl ReleaseShim {
    fn serving(bundle: &Path) -> Self {
        Self::running(&serve_body(bundle))
    }

    /// A shim that records each request, then runs `body` with `$out`,
    /// `$range` and `$url` set from its arguments.
    fn running(body: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            "curl",
            &format!(
                "PATH=/usr/bin:/bin\n\
                 out=''; range=''; prev=''; url=''\n\
                 for a in \"$@\"; do\n\
                   [ \"$prev\" = -o ] && out=\"$a\"\n\
                   [ \"$prev\" = -r ] && range=\"$a\"\n\
                   prev=\"$a\"; url=\"$a\"\n\
                 done\n\
                 echo \"$url $range\" >> '{requests}'\n{body}",
                requests = dir.path().join("requests").display(),
            ),
            0o755,
        );
        Self { dir }
    }

    /// Puts the shim ahead of the system tools on `ed`'s search path.
    fn install(&self, ed: &mut Editor) {
        ed.state.dirs.search_path =
            Some(format!("{}:/usr/bin:/bin", self.dir.path().display()).into());
    }

    /// The requests made so far, in order.
    fn requests(&self) -> Vec<String> {
        match std::fs::read_to_string(self.dir.path().join("requests")) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => panic!("read requests: {e}"),
        }
    }
}

/// The shim body that serves the files of `bundle` by the URL's file name.
fn serve_body(bundle: &Path) -> String {
    format!(
        "file='{bundle}'/\"$(basename \"$url\")\"\n\
         if [ -n \"$range\" ]; then\n\
           start=${{range%-*}}; end=${{range#*-}}\n\
           head -c $((end + 1)) \"$file\" | tail -c $((end - start + 1)) > \"$out\"\n\
         else\n\
           cp \"$file\" \"$out\"\n\
         fi\n",
        bundle = bundle.display(),
    )
}

fn release_url(file: &str) -> String {
    format!(
        "https://github.com/cvlmtg/treesitter.hume/releases/download/{}/{file}",
        release_tag()
    )
}

/// The release request for the index.
fn index_request() -> String {
    format!(
        "{} ",
        release_url(&format!("grammars-{}.index", platform()))
    )
}

/// The release request for `name`'s byte range, as `bundle`'s index lists it.
fn member_request(bundle: &Path, name: &str) -> String {
    let index = std::fs::read_to_string(index_path(bundle)).unwrap();
    let fields: Vec<u64> = index
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap()
        .split(' ')
        .take(2)
        .map(|n| n.parse().unwrap())
        .collect();
    format!(
        "{} {}-{}",
        release_url(&format!("grammars-{}.tar", platform())),
        fields[0],
        fields[0] + fields[1] - 1
    )
}

/// With no `grammar-source`, the index and the member come from the release
/// the pin names, for this platform.
#[test]
fn install_from_the_release_fetches_the_pinned_tag_for_this_platform() {
    require_grammars(&["json"]);
    let release = ReleaseShim::serving(&fixture_bundle_dir());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_release(data_tmp.path(), &release);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(installed(data_tmp.path(), "json"), "{}", log(&ed));
    assert_eq!(
        release.requests(),
        [
            index_request(),
            member_request(&fixture_bundle_dir(), "json")
        ]
    );
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// A folder that holds every requested grammar serves them all, and the
/// release is never asked for anything.
#[test]
fn a_folder_holding_every_name_never_reaches_the_release() {
    require_grammars(&["json", "markdown", "markdown.inline"]);
    let release = ReleaseShim::serving(&fixture_bundle_dir());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &fixture_archive_dir());
    release.install(&mut ed);

    ensure_grammars(&mut ed, data_tmp.path(), &["json", "markdown"]);

    assert_eq!(error_log(&ed), Vec::<String>::new());
    for name in ["json", "markdown", "markdown.inline"] {
        assert!(installed(data_tmp.path(), name), "{name}: {}", log(&ed));
    }
    assert_eq!(release.requests(), Vec::<String>::new());
}

/// A grammar the folder holds comes from the folder, one it lacks from the
/// release, in one batch that reads the release index once.
#[test]
fn a_grammar_the_folder_lacks_comes_from_the_release() {
    require_grammars(&["json", "yaml"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| manifest);
    let release = ReleaseShim::serving(&fixture_bundle_dir());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    release.install(&mut ed);

    ensure_grammars(&mut ed, data_tmp.path(), &["json", "yaml"]);

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(installed(data_tmp.path(), "json"), "{}", log(&ed));
    assert!(installed(data_tmp.path(), "yaml"), "{}", log(&ed));
    assert_eq!(
        release.requests(),
        [
            index_request(),
            member_request(&fixture_bundle_dir(), "yaml")
        ]
    );
}

/// A grammar the plan takes from the folder is installed from the folder,
/// even when its archive leaves the folder before its turn comes.
#[test]
fn a_grammar_planned_from_the_folder_never_comes_from_the_release() {
    require_grammars(&["json", "yaml"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| manifest);
    let release = ReleaseShim::running(&format!(
        "{}case \"$url\" in *.tar) rm -f '{}' ;; esac\n",
        serve_body(&fixture_bundle_dir()),
        archive_path(&folder, "json").display(),
    ));
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    release.install(&mut ed);

    ensure_grammars(&mut ed, data_tmp.path(), &["yaml", "json"]);

    assert!(installed(data_tmp.path(), "yaml"), "{}", log(&ed));
    assert!(!installed(data_tmp.path(), "json"), "{}", log(&ed));
    assert_eq!(
        release.requests(),
        [
            index_request(),
            member_request(&fixture_bundle_dir(), "yaml")
        ]
    );
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// Every missing name is checked against the folder and the release before
/// anything installs; the error names both.
#[test]
fn a_name_in_neither_the_folder_nor_the_release_installs_nothing() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| manifest);
    let release = ReleaseShim::serving(&fixture_bundle_dir());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    release.install(&mut ed);

    ensure_grammars(&mut ed, data_tmp.path(), &["json", "toml"]);

    let expected = format!(
        "no toml grammar in {} or treesitter.hume {} for {}",
        folder.display(),
        release_tag(),
        platform()
    );
    assert!(log(&ed).contains(&expected), "{}", log(&ed));
    assert!(!installed(data_tmp.path(), "json"));
    assert_eq!(release.requests(), [index_request()]);
}

/// A configured folder that is not a directory is a mistake to report, not
/// a reason to download from the release instead.
#[test]
fn a_grammar_source_that_is_not_a_directory_is_an_error_and_reaches_no_release() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let file = archive_path(work.path(), "json");
    std::fs::copy(archive_path(&fixture_archive_dir(), "json"), &file).unwrap();
    for source in [work.path().join("missing"), file] {
        let release = ReleaseShim::serving(&fixture_bundle_dir());
        let data_tmp = tempfile::tempdir().unwrap();
        let mut ed = plum_from_folder(data_tmp.path(), &source);
        release.install(&mut ed);

        type_cmd(&mut ed, ":plum-install-grammar json");

        let expected = format!("'grammar-source {} is not a directory", source.display());
        assert!(log(&ed).contains(&expected), "{}", log(&ed));
        assert!(!installed(data_tmp.path(), "json"));
        assert_eq!(release.requests(), Vec::<String>::new());
    }
}

/// A configured folder that holds no grammar archive is a wrong path, such
/// as the parent of the folder the archives were built into.
#[test]
fn a_grammar_source_holding_no_archive_is_an_error_and_reaches_no_release() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let parent = work.path().join("parent");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::rename(folder_copy(work.path()), parent.join("grammars")).unwrap();
    let release = ReleaseShim::serving(&fixture_bundle_dir());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &parent);
    release.install(&mut ed);

    type_cmd(&mut ed, ":plum-install-grammar json");

    let expected = format!(
        "'grammar-source {} holds no grammar archive",
        parent.display()
    );
    assert!(log(&ed).contains(&expected), "{}", log(&ed));
    assert!(!installed(data_tmp.path(), "json"));
    assert_eq!(release.requests(), Vec::<String>::new());
}

/// A grammar the folder lacks, when the release cannot be reached, fails
/// naming the folder it is not in as well as the release failure.
#[test]
fn a_grammar_the_folder_lacks_names_the_folder_when_the_release_fails() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| manifest);
    let release = ReleaseShim::running("exit 22\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    release.install(&mut ed);

    type_cmd(&mut ed, ":plum-install-grammar yaml");

    let log = log(&ed);
    assert!(
        log.contains(&format!("no yaml grammar in {}", folder.display())),
        "{log}"
    );
    assert!(log.contains("cannot fetch the treesitter.hume"), "{log}");
    assert!(!installed(data_tmp.path(), "yaml"));
}

/// A configured folder inside a folder PLUM installs into or cleans up is
/// refused, and its archives are left where they are.
#[test]
fn a_grammar_source_inside_a_folder_plum_manages_is_refused() {
    require_grammars(&["json"]);
    for (managed, inner) in [("plum-staging", "grammars"), ("grammars", "mine")] {
        let data_tmp = tempfile::tempdir().unwrap();
        let managed = canonical_data_dir(data_tmp.path()).join(managed);
        let folder = managed.join(inner);
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::rename(folder_copy(data_tmp.path()), &folder).unwrap();
        let mut ed = plum_from_folder(data_tmp.path(), &folder);

        type_cmd(&mut ed, ":plum-install-grammar json");

        let expected = format!(
            "'grammar-source {} is inside {}, which PLUM manages",
            folder.display(),
            managed.display()
        );
        assert!(log(&ed).contains(&expected), "{}", log(&ed));
        assert!(archive_path(&folder, "json").exists());
        assert!(!installed(data_tmp.path(), "json"));
    }
}

#[test]
fn a_digest_mismatch_fails_and_leaves_nothing_behind() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let bundle = bundle_copy(work.path());
    edit_index_line(&bundle, "json ", |line| {
        let mut fields: Vec<&str> = line.split(' ').collect();
        fields[3] = "0000000000000000000000000000000000000000000000000000000000000000";
        fields.join(" ")
    });
    let release = ReleaseShim::serving(&bundle);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_release(data_tmp.path(), &release);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(log(&ed).contains("sha256 mismatch"), "{}", log(&ed));
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    assert!(!ed.state.config.languages.has_grammar("json"));
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

#[test]
fn a_release_index_for_another_platform_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let bundle = bundle_copy(work.path());
    edit_index_line(&bundle, "treesitter.hume ", |_| {
        format!("treesitter.hume {} plan9-mips", release_tag())
    });
    let release = ReleaseShim::serving(&bundle);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_release(data_tmp.path(), &release);

    type_cmd(&mut ed, ":plum-install-grammar json");

    let expected = format!("is for plan9-mips, not {}", platform());
    assert!(log(&ed).contains(&expected), "{}", log(&ed));
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
}

#[test]
fn an_archive_built_for_another_platform_is_refused() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| {
        manifest.replace(
            &format!("(platform . \"{}\")", platform()),
            "(platform . \"plan9-mips\")",
        )
    });
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(
        log(&ed).contains("the json grammar is built for plan9-mips"),
        "{}",
        log(&ed)
    );
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// An archive named for one grammar that holds another installs nothing:
/// the unpacked manifest names that other grammar.
#[test]
fn an_archive_holding_another_grammar_is_refused() {
    require_grammars(&["json", "yaml"]);
    let work = tempfile::tempdir().unwrap();
    let folder = folder_copy(work.path());
    std::fs::copy(archive_path(&folder, "yaml"), archive_path(&folder, "json")).unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(
        log(&ed).contains("the json grammar archive is the yaml grammar"),
        "{}",
        log(&ed)
    );
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

#[test]
fn a_failed_reinstall_keeps_the_installed_grammar() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = folder_copy(work.path());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    type_cmd(&mut ed, ":plum-install-grammar json");
    assert!(installed(data_tmp.path(), "json"), "{}", log(&ed));

    std::fs::write(archive_path(&folder, "json"), b"truncated").unwrap();
    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(log(&ed).contains("install failed"), "{}", log(&ed));
    assert!(installed(data_tmp.path(), "json"));
    assert!(
        current_install(data_tmp.path(), "json")
            .unwrap()
            .join("queries/highlights.scm")
            .exists()
    );
    assert!(ed.state.config.languages.has_grammar("json"));
}

#[test]
fn a_reinstall_whose_library_cannot_load_keeps_the_installed_grammar() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = folder_copy(work.path());
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    type_cmd(&mut ed, ":plum-install-grammar json");
    let before = installs(data_tmp.path(), "json");
    assert_eq!(before.len(), 1, "{}", log(&ed));
    let broken = work.path().join("broken");
    replace_archive(
        &folder,
        &one_archive_folder(&broken, "json", wrong_symbol),
        "json",
    );

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(log(&ed).contains("install failed"), "{}", log(&ed));
    assert_eq!(installs(data_tmp.path(), "json"), before);
    let manifest = current_install(data_tmp.path(), "json")
        .unwrap()
        .join("manifest.scm");
    assert!(
        std::fs::read_to_string(manifest)
            .unwrap()
            .contains("\"tree_sitter_json\""),
        "the current install is the one whose library loads"
    );
    assert!(ed.state.config.languages.has_grammar("json"));
    assert_eq!(
        entry_names(&canonical_data_dir(data_tmp.path()).join("grammars")),
        ["json"]
    );
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// A reinstall takes effect in the session that runs it, though the grammar
/// it replaces is already loaded.
#[test]
fn a_reinstall_of_a_loaded_grammar_loads_the_new_library() {
    require_grammars(&["json", "yaml"]);
    let work = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let folder = folder_copy(work.path());
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    type_cmd(&mut ed, ":plum-install-grammar json");
    ed.reparse_stale_buffers();
    assert!(!json_runs_the_yaml_library(&mut ed), "{}", log(&ed));
    let renamed = work.path().join("renamed");
    replace_archive(
        &folder,
        &archive_folder(&renamed, "json", yaml_as_json),
        "json",
    );

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(json_runs_the_yaml_library(&mut ed));
    assert_eq!(installs(data_tmp.path(), "json").len(), 1);
}

/// An older install that cannot be deleted is reported as an error naming
/// it, and the reinstall that replaced it stays in place.
#[test]
fn an_older_install_that_cannot_be_deleted_is_reported_and_the_reinstall_stands() {
    use std::os::unix::fs::PermissionsExt;
    require_grammars(&["json"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &fixture_archive_dir());
    type_cmd(&mut ed, ":plum-install-grammar json");
    let older = current_install(data_tmp.path(), "json").unwrap();
    std::fs::set_permissions(&older, std::fs::Permissions::from_mode(0o555)).unwrap();

    type_cmd(&mut ed, ":plum-install-grammar json");

    std::fs::set_permissions(&older, std::fs::Permissions::from_mode(0o755)).unwrap();
    let errors = error_log(&ed);
    assert_eq!(errors.len(), 1, "{}", log(&ed));
    assert!(
        errors[0].contains(&older.display().to_string()),
        "{errors:?}"
    );
    assert!(!log(&ed).contains("install failed"), "{}", log(&ed));
    let installs = installs(data_tmp.path(), "json");
    assert_eq!(installs.len(), 2);
    assert_ne!(current_install(data_tmp.path(), "json").unwrap(), older);
    assert!(ed.state.config.languages.has_grammar("json"));
}

/// A failed load check leaves nothing a retry in the same session would
/// load in place of the corrected grammar.
#[test]
fn a_retry_after_a_failed_install_loads_the_corrected_library() {
    require_grammars(&["json", "yaml"]);
    let work = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let folder = archive_folder(work.path(), "json", |grammar| {
        std::fs::write(
            grammar.join("queries/highlights.scm"),
            "(no_such_node) @keyword",
        )
        .unwrap();
    });
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    type_cmd(&mut ed, ":plum-install-grammar json");
    assert!(log(&ed).contains("install failed"), "{}", log(&ed));
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    let fixed = work.path().join("fixed");
    replace_archive(
        &folder,
        &archive_folder(&fixed, "json", yaml_as_json),
        "json",
    );

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed).len(), 1, "{}", log(&ed));
    assert!(json_runs_the_yaml_library(&mut ed));
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

#[test]
fn a_grammar_whose_library_cannot_load_installs_nothing() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", wrong_symbol);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(log(&ed).contains("install failed"), "{}", log(&ed));
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    assert!(!ed.state.config.languages.has_grammar("json"));
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

#[test]
fn a_manifest_without_a_symbol_is_refused() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| {
        manifest.replace(" (symbol . \"tree_sitter_json\")\n", "")
    });
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(
        log(&ed).contains("grammar manifest has no symbol"),
        "{}",
        log(&ed)
    );
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

#[test]
fn a_relative_grammar_source_is_refused() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[{]>\"x\": 1}\n");

    load_plum_with(
        &mut ed,
        data_tmp.path(),
        "",
        Some("(hash 'grammar-source \"bundle\")"),
    );

    let errors = error_log(&ed);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("'grammar-source must be an absolute path: bundle")),
        "{errors:?}"
    );
}

/// The batch's upfront check covers the dependencies the batch would
/// install, so a dependency no source serves installs nothing.
#[test]
fn a_dependency_in_neither_the_folder_nor_the_release_installs_nothing() {
    require_grammars(&["json", "markdown"]);
    let work = tempfile::tempdir().unwrap();
    let folder = folder_copy(work.path());
    std::fs::remove_file(archive_path(&folder, "markdown.inline")).unwrap();
    let bundle = bundle_copy(work.path());
    edit_index_line(&bundle, "markdown.inline ", |_| String::new());
    let release = ReleaseShim::serving(&bundle);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);
    release.install(&mut ed);

    ensure_grammars(&mut ed, data_tmp.path(), &["json", "markdown"]);

    let expected = format!(
        "no markdown.inline grammar in {} or treesitter.hume {} for {}",
        folder.display(),
        release_tag(),
        platform()
    );
    assert!(log(&ed).contains(&expected), "{}", log(&ed));
    for name in ["json", "markdown"] {
        assert!(!installed(data_tmp.path(), name), "{name}: {}", log(&ed));
    }
}

/// A release index download that writes part of its output and then fails
/// fails the install and leaves no temp file.
#[test]
fn a_failed_index_download_leaves_no_temp_file() {
    let release = ReleaseShim::running("printf 'partial' > \"$out\"\nexit 22\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_release(data_tmp.path(), &release);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert!(log(&ed).contains("curl"), "{}", log(&ed));
    assert!(!installed(data_tmp.path(), "json"));
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// An index that is not UTF-8 fails the install cleanly: no temp file, and a
/// later command still runs.
#[test]
fn an_unreadable_index_fails_cleanly() {
    let release = ReleaseShim::running("printf '\\377\\376' > \"$out\"\n");
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_release(data_tmp.path(), &release);

    type_cmd(&mut ed, ":plum-install-grammar json");

    let expected = format!(
        "cannot fetch the treesitter.hume {} grammar index",
        release_tag()
    );
    assert!(log(&ed).contains(&expected), "{}", log(&ed));
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"no language "nosuchlang""#)),
        "a later command must still run: {:?}",
        ed.state.status_msg
    );
}

/// A local archive built by another release of the build tool still
/// installs, with a warning naming both tags.
#[test]
fn a_grammar_from_another_tag_installs_with_a_warning() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", |manifest| {
        manifest.replace(
            &format!("(tag . \"{}\")", release_tag()),
            "(tag . \"v9.9.9\")",
        )
    });
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = plum_from_folder(data_tmp.path(), &folder);

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(installed(data_tmp.path(), "json"));
    let expected = format!("treesitter.hume v9.9.9; HUME pins {}", release_tag());
    assert!(
        warning_texts(&ed).iter().any(|w| w.contains(&expected)),
        "{}",
        log(&ed)
    );
}

#[test]
fn cleanup_removes_everything_but_the_current_installs() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    let ext = grammar_platform_ext();
    let grammars = canonical_data_dir(data_tmp.path()).join("grammars");
    for (name, install) in [("json", "5"), ("json", "40"), ("proto", "1")] {
        let dir = grammars.join(name).join(install);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.{ext}")), b"").unwrap();
    }
    std::fs::write(grammars.join(format!("json/json.{ext}")), b"").unwrap();
    std::fs::create_dir_all(grammars.join("sources/json")).unwrap();
    std::fs::write(grammars.join("sources/json/grammar.js"), b"").unwrap();
    std::fs::write(grammars.join(format!("json.{ext}")), b"").unwrap();
    let staging = staging_dir(data_tmp.path());
    std::fs::create_dir_all(staging.join("json")).unwrap();
    std::fs::write(staging.join("json.tar.gz"), b"").unwrap();
    std::fs::write(staging.join(format!("json-7.{ext}")), b"").unwrap();
    std::fs::write(staging.join("index"), b"").unwrap();

    type_cmd(&mut ed, ":plum-cleanup-grammars");

    assert_eq!(entry_names(&grammars), ["json"]);
    assert_eq!(entry_names(&grammars.join("json")), ["40"]);
    assert_eq!(staging_leftovers(data_tmp.path()), Vec::<String>::new());
}

/// Two installs left side by side, as on Windows when the older one's
/// library was loaded: startup registers the newer one.
#[test]
fn startup_registers_the_newest_install() {
    require_grammars(&["json"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let grammars = canonical_data_dir(data_tmp.path()).join("grammars");
    let older = test_fixtures::install_fixture_grammar(&grammars, "json");
    let manifest = older.join("manifest.scm");
    std::fs::write(
        &manifest,
        wrong_symbol(std::fs::read_to_string(&manifest).unwrap()),
    )
    .unwrap();
    test_fixtures::copy_fixture_grammar(&grammars.join("json/20"), "json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":set buffer language=json");
    let bid = ed.focused_buffer_id();

    ed.reparse_stale_buffers();

    assert!(ed.state.buffers.get(bid).syntax.is_some(), "{}", log(&ed));
    assert_eq!(warning_texts(&ed), Vec::<String>::new());
}

/// A language folder with no install folder holds no grammar, whatever
/// else it holds.
#[test]
fn a_language_folder_without_an_install_is_not_installed() {
    require_grammars(&["json"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let grammars = canonical_data_dir(data_tmp.path()).join("grammars");
    test_fixtures::copy_fixture_grammar(&grammars.join("json"), "json");
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// The load check does not depend on registration, which `init.scm` only
/// queues.
#[test]
fn ensure_grammars_from_init_installs_nothing_that_cannot_load() {
    require_grammars(&["json"]);
    let work = tempfile::tempdir().unwrap();
    let folder = one_archive_folder(work.path(), "json", wrong_symbol);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    let config = format!("(hash 'grammar-source {})", steel_path(&folder));

    load_with_init(
        &mut ed,
        data_tmp.path(),
        &format!(
            "(load-plugin! \"core:stdlib\")\n{}\n\
             (%activate-plugin-inline! \"core:plum\" \"grammars.scm\")\n\
             (call! \"plum/ensure-grammars\" '(\"json\"))",
            hume_scripting::eager_load_scm("core:plum", Some(&config))
        ),
    );

    assert!(log(&ed).contains("grammar load failed"), "{}", log(&ed));
    assert!(!grammar_dir(data_tmp.path(), "json").exists());
}

/// Installs json from the pinned treesitter.hume release. Hits the network;
/// requires `curl` and `tar` on `PATH`.
#[test]
fn install_json_from_the_pinned_release_e2e() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":set buffer language=json");
    let bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":plum-install-grammar json");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(installed(data_tmp.path(), "json"), "{}", log(&ed));
    ed.reparse_stale_buffers();
    assert!(ed.state.buffers.get(bid).syntax.is_some());
}
