//! Every store that ranks a write by its `ConfigOrigin`, driven through the
//! editor's own apply paths. The expected winner comes from the rule alone:
//! the higher origin is in force whichever write came first, and a tie goes
//! to the first or the later write as the store's contract says.

use super::*;

use super::language_config::boot;
use hume_editing::text::LineEnding;
use hume_scripting::host::{BindMode as ScriptBindMode, KeymapTarget, OptionInput, OptionValue};
use hume_scripting::{
    ConfigOrigin, Effect, FeatureFilter, ListEntry, ListTier, PendingLanguageReg,
    PendingLspServerOp, PendingLspServerReg, ServerName,
};
use hume_treesitter::registry::LanguageFields;

use crate::editor::keymap::BindMode;
use crate::editor::settings::ops;

const ORIGINS: [ConfigOrigin; 3] = [
    ConfigOrigin::Bundled,
    ConfigOrigin::Plugin,
    ConfigOrigin::User,
];

/// Which of two equal-origin writes a store keeps.
#[derive(Clone, Copy)]
enum OnTie {
    FirstWins,
    LaterWins,
}

/// Which of two writes, in arrival order, is in force: `1` or `2`.
fn winner(first: ConfigOrigin, second: ConfigOrigin, on_tie: OnTie) -> u8 {
    match first.cmp(&second) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => 2,
        std::cmp::Ordering::Equal => match on_tie {
            OnTie::FirstWins => 1,
            OnTie::LaterWins => 2,
        },
    }
}

/// One ranked store: a write of label `1` or `2` from an origin, and a read
/// of the label in force.
struct Store {
    name: &'static str,
    on_tie: OnTie,
    prepare: fn(&mut Editor),
    write: fn(&mut Editor, ConfigOrigin, u8),
    read: fn(&Editor) -> Option<u8>,
}

fn nothing(_: &mut Editor) {}

/// A fresh editor with a scripting host attached.
fn with_host(ed: &mut Editor) {
    let tmp = tempfile::tempdir().unwrap();
    run(ed, tmp.path(), "");
}

fn label_of(texts: [&str; 2], now: &str) -> Option<u8> {
    texts
        .iter()
        .position(|text| *text == now)
        .map(|index| index as u8 + 1)
}

// ── Option writes ────────────────────────────────────────────────────────

const INDENT_WIDTHS: [&str; 2] = ["2", "3"];

fn indent_width_label(value: Option<OptionValue>) -> Option<u8> {
    match value {
        Some(OptionValue::Int(width)) => label_of(INDENT_WIDTHS, &width.to_string()),
        _ => None,
    }
}

fn write_global_option(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let value = OptionInput::Text(INDENT_WIDTHS[label as usize - 1].to_string());
    ops::apply_global(&mut ed.state, &mut ed.view, "indent-width", &value, by).unwrap();
}

fn read_global_option(ed: &Editor) -> Option<u8> {
    indent_width_label(crate::editor::settings::setting_value(
        "indent-width",
        &ed.state.settings,
        None,
    ))
}

fn write_buffer_option(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let value = OptionInput::Text(INDENT_WIDTHS[label as usize - 1].to_string());
    let bid = ed.focused_buffer_id();
    ops::apply_buffer(&mut ed.state, bid, "indent-width", &value, by).unwrap();
}

fn read_buffer_option(ed: &Editor) -> Option<u8> {
    let overrides = &ed.state.buffers.get(ed.focused_buffer_id()).overrides;
    indent_width_label(crate::editor::settings::setting_value(
        "indent-width",
        &ed.state.settings,
        Some(overrides),
    ))
}

// ── Buffer line ending and language ──────────────────────────────────────

const LINE_ENDINGS: [&str; 2] = ["crlf", "lf"];

fn write_line_ending(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let bid = ed.focused_buffer_id();
    ops::set_line_ending(&mut ed.state, bid, LINE_ENDINGS[label as usize - 1], by).unwrap();
}

fn read_line_ending(ed: &Editor) -> Option<u8> {
    match ed.doc().text().line_ending() {
        LineEnding::CrLf => Some(1),
        LineEnding::Lf => Some(2),
    }
}

const LANGUAGES: [&str; 2] = ["la", "lb"];

fn write_buffer_language(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some(LANGUAGES[label as usize - 1]), by);
}

fn read_buffer_language(ed: &Editor) -> Option<u8> {
    let id = ed.doc().language?;
    label_of(LANGUAGES, ed.state.config.languages.name_of(id))
}

// ── Key bindings ─────────────────────────────────────────────────────────

fn bound_key() -> termina::event::KeyEvent {
    key('ø')
}

fn bind_effect(by: ConfigOrigin, command: &str) -> Effect {
    Effect::BindKey {
        mode: ScriptBindMode::Normal,
        keys: vec![bound_key()],
        cmd: command.to_string(),
        force_extend: false,
        keymap: KeymapTarget::Global,
        origin: by,
    }
}

fn write_binding(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    ed.apply_script_effects(vec![bind_effect(by, &format!("cmd-{label}"))]);
}

/// Label `2` unbinds the key.
fn write_binding_or_unbinding(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let effect = if label == 1 {
        bind_effect(by, "cmd-1")
    } else {
        Effect::UnbindKey {
            mode: ScriptBindMode::Normal,
            keys: vec![bound_key()],
            keymap: KeymapTarget::Global,
            origin: by,
        }
    };
    ed.apply_script_effects(vec![effect]);
}

fn read_binding(ed: &Editor) -> Option<u8> {
    let (command, _) = ed
        .state
        .config
        .keymap
        .lookup_command(BindMode::Normal, &[bound_key()])?;
    label_of(["cmd-1", "cmd-2"], &command)
}

fn read_binding_or_unbinding(ed: &Editor) -> Option<u8> {
    Some(read_binding(ed).unwrap_or(2))
}

// ── Language server registrations and lists ──────────────────────────────

fn server(name: &str) -> ServerName {
    ServerName::parse(name).unwrap()
}

fn registration(by: ConfigOrigin, name: &str, command: String) -> PendingLspServerReg {
    PendingLspServerReg {
        name: server(name),
        command,
        args: Vec::new(),
        init_options: None,
        settings: None,
        env: Vec::new(),
        origin: by,
    }
}

fn write_registration(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let reg = registration(by, "srv", format!("cmd-{label}"));
    ed.apply_script_effects(vec![Effect::LspServerOp(PendingLspServerOp::Register(reg))]);
}

/// Label `2` unregisters the server.
fn write_registration_or_unregistration(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let op = if label == 1 {
        PendingLspServerOp::Register(registration(by, "srv", "cmd-1".to_string()))
    } else {
        PendingLspServerOp::Unregister {
            name: server("srv"),
            origin: by,
        }
    };
    ed.apply_script_effects(vec![Effect::LspServerOp(op)]);
}

fn read_registration(ed: &Editor) -> Option<u8> {
    let command = ed.state.lsp.registered_command_for_test("srv")?;
    label_of(["cmd-1", "cmd-2"], &command)
}

fn read_registration_or_unregistration(ed: &Editor) -> Option<u8> {
    Some(read_registration(ed).unwrap_or(2))
}

fn register_list_servers(ed: &mut Editor) {
    let regs = ["s1", "s2"].map(|name| {
        Effect::LspServerOp(PendingLspServerOp::Register(registration(
            ConfigOrigin::User,
            name,
            name.to_string(),
        )))
    });
    ed.apply_script_effects(regs.into());
}

fn write_server_list(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    let entry = ListEntry {
        name: server(&format!("s{label}")),
        filter: FeatureFilter::All,
    };
    ed.apply_script_effects(vec![Effect::LspServerOp(
        PendingLspServerOp::SetLanguageServers {
            language: "mx".to_string(),
            tier: ListTier::Explicit,
            origin: by,
            entries: Some(vec![entry]),
        },
    )]);
}

fn read_server_list(ed: &Editor) -> Option<u8> {
    let first = crate::editor::lsp::introspect::language_servers(&ed.state.lsp, "mx")
        .into_iter()
        .next()?;
    label_of(["s1", "s2"], &first.name.to_string())
}

// ── Per-language configuration ───────────────────────────────────────────

const LANGUAGE_IDS: [&str; 2] = ["id-1", "id-2"];

fn define_matrix_language(ed: &mut Editor) {
    ed.apply_script_effects(vec![Effect::LanguageReg(PendingLanguageReg::Identity {
        name: "mx".to_string(),
        extensions: Vec::new(),
        globs: Vec::new(),
        shebangs: Vec::new(),
        origin: ConfigOrigin::Bundled,
    })]);
}

fn write_language_field(ed: &mut Editor, by: ConfigOrigin, label: u8) {
    ed.apply_script_effects(vec![Effect::LanguageReg(PendingLanguageReg::Configure {
        name: "mx".to_string(),
        fields: LanguageFields {
            lsp_language_id: Some(LANGUAGE_IDS[label as usize - 1].to_string()),
            ..Default::default()
        },
        origin: by,
    })]);
}

fn read_language_field(ed: &Editor) -> Option<u8> {
    let languages = &ed.state.config.languages;
    let id = languages.defined_id("mx")?;
    label_of(LANGUAGE_IDS, languages.lsp_language_id_of(id))
}

const STORES: &[Store] = &[
    Store {
        name: "global option",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_global_option,
        read: read_global_option,
    },
    Store {
        name: "buffer option",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_buffer_option,
        read: read_buffer_option,
    },
    Store {
        name: "line ending",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_line_ending,
        read: read_line_ending,
    },
    Store {
        name: "buffer language",
        on_tie: OnTie::LaterWins,
        prepare: with_host,
        write: write_buffer_language,
        read: read_buffer_language,
    },
    Store {
        name: "key binding",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_binding,
        read: read_binding,
    },
    Store {
        name: "key binding against unbinding",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_binding_or_unbinding,
        read: read_binding_or_unbinding,
    },
    Store {
        name: "server registration",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_registration,
        read: read_registration,
    },
    Store {
        name: "server registration against unregistration",
        on_tie: OnTie::LaterWins,
        prepare: nothing,
        write: write_registration_or_unregistration,
        read: read_registration_or_unregistration,
    },
    Store {
        name: "language server list",
        on_tie: OnTie::LaterWins,
        prepare: register_list_servers,
        write: write_server_list,
        read: read_server_list,
    },
    Store {
        name: "language field",
        on_tie: OnTie::FirstWins,
        prepare: define_matrix_language,
        write: write_language_field,
        read: read_language_field,
    },
];

fn traced_as_shadowed(ed: &Editor) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.severity == Severity::Trace && e.text.contains("a higher origin"))
}

#[test]
fn every_ranked_store_keeps_the_higher_origin_whichever_write_came_first() {
    for store in STORES {
        for first in ORIGINS {
            for second in ORIGINS {
                let mut ed = editor_from("-[a]>b\n");
                (store.prepare)(&mut ed);

                (store.write)(&mut ed, first, 1);
                (store.write)(&mut ed, second, 2);

                let expected = winner(first, second, store.on_tie);
                assert_eq!(
                    (store.read)(&ed),
                    Some(expected),
                    "{}: {first:?} then {second:?}",
                    store.name
                );
            }
        }
    }
}

#[test]
fn a_write_a_higher_origin_shadows_is_traced() {
    for store in STORES {
        for first in ORIGINS {
            for second in ORIGINS.into_iter().filter(|second| *second < first) {
                let mut ed = editor_from("-[a]>b\n");
                (store.prepare)(&mut ed);

                (store.write)(&mut ed, first, 1);
                (store.write)(&mut ed, second, 2);

                assert!(
                    traced_as_shadowed(&ed),
                    "{}: {second:?} after {first:?} left no trace",
                    store.name
                );
            }
        }
    }
}

// ── Reads a script makes before its own writes apply ─────────────────────

fn language_name(ed: &Editor) -> Option<String> {
    let id = ed.doc().language?;
    Some(ed.state.config.languages.name_of(id).to_owned())
}

/// The user asserts the language a plugin already asserted: the assertion
/// still records the user's origin, so a later plugin write does not apply.
#[test]
fn a_users_equal_language_write_records_its_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "assert-la" ""
             (lambda (pane) (set-buffer-option! pane "language" "la")))"#,
    );
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("la"), ConfigOrigin::Plugin);

    call(&mut ed, "assert-la");
    ed.set_buffer_language_by_name(bid, Some("lb"), ConfigOrigin::Plugin);

    assert_eq!(language_name(&ed).as_deref(), Some("la"));
}

const PROBING_HOOK: &str = r#"(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "la")
      (set-buffer-option! pane "language" "lb")
      (log! 'warn (string-append "probe:" (get-buffer-option pane "language"))))))"#;

/// A plugin hook writes a language the user's assertion shadows, then reads
/// the buffer's language back: the read agrees with what the editor applies.
#[test]
fn a_hooks_language_read_back_ignores_a_write_the_editor_will_shadow() {
    let (mut ed, _fixture) = boot("", &[], r#"(load-plugin! "user/tp")"#, PROBING_HOOK);
    let bid = ed.focused_buffer_id();

    ed.set_buffer_language_by_name(bid, Some("la"), ConfigOrigin::User);
    ed.settle();

    assert_eq!(language_name(&ed).as_deref(), Some("la"));
    let warnings: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(warnings, ["probe:la"]);
}

// ── A default list ranks below every explicit list ───────────────────────

const EXPLICIT: &str = r#"(set-language-servers! "mx" '("sa"))"#;
const DEFAULT: &str = r#"(set-default-language-servers! "mx" '("sb"))"#;
const REGISTER_SERVERS: &str = r#"(register-lsp-server! "sa" #:command "sa")
                                  (register-lsp-server! "sb" #:command "sb")"#;

/// `init.scm` and the plugin source that run `writes` in order: each write
/// names the origin whose file holds it.
fn sources_for(writes: [(ConfigOrigin, &str); 2]) -> (String, String) {
    let load = r#"(load-plugin! "user/tp")"#;
    let [(first_by, first), (second_by, second)] = writes;
    let init = REGISTER_SERVERS.to_string();
    match (first_by, second_by) {
        (ConfigOrigin::User, ConfigOrigin::User) => {
            (format!("{init}\n{first}\n{second}"), String::new())
        }
        (ConfigOrigin::Plugin, ConfigOrigin::Plugin) => {
            (format!("{init}\n{load}"), format!("{first}\n{second}"))
        }
        (ConfigOrigin::User, ConfigOrigin::Plugin) => {
            (format!("{init}\n{first}\n{load}"), second.to_string())
        }
        (ConfigOrigin::Plugin, ConfigOrigin::User) => {
            (format!("{init}\n{load}\n{second}"), first.to_string())
        }
        _ => unreachable!("a file's code ranks as the user or a plugin"),
    }
}

#[test]
fn an_explicit_server_list_beats_a_default_from_any_origin_in_any_order() {
    let file_origins = [ConfigOrigin::Plugin, ConfigOrigin::User];
    for explicit_by in file_origins {
        for default_by in file_origins {
            for explicit_first in [true, false] {
                let (explicit, default) = ((explicit_by, EXPLICIT), (default_by, DEFAULT));
                let writes = if explicit_first {
                    [explicit, default]
                } else {
                    [default, explicit]
                };
                let (init, plugin) = sources_for(writes);

                let (ed, _fixture) = boot("", &[], &init, &plugin);

                let servers: Vec<String> =
                    crate::editor::lsp::introspect::language_servers(&ed.state.lsp, "mx")
                        .into_iter()
                        .map(|entry| entry.name.to_string())
                        .collect();
                assert_eq!(
                    servers,
                    ["sa"],
                    "explicit from {explicit_by:?}, default from {default_by:?}, explicit first: {explicit_first}"
                );
            }
        }
    }
}
