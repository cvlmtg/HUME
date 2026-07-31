//! # `EditorState`/`Editor` field classification drift
//!
//! `Editor::reset_config_state` resets `EditorState.config: ConfigState`
//! wholesale (a field added there is reset by construction — see
//! `ConfigState`'s own doc), but every *other* field on `EditorState` — and
//! on `Editor` itself, which `reset_config_state` also reaches directly for
//! `lsp`/`timer_wheel`/`timer_payloads` — needs a human decision: does
//! `:reload-config` reset it too (like `settings`, via
//! `settings_ops::reset_globals`), or does it survive untouched (buffers,
//! panes, undo history, registers, …)? Nothing enforced that decision get
//! made — a field added directly to `EditorState` instead of nested inside
//! `ConfigState`, or a field added to `Editor` itself, would silently
//! default to "survives", correct for most fields but wrong for one that
//! should have reset.
//!
//! `editor_state_fields_are_classified`/`editor_fields_are_classified`
//! extract every top-level field name from each struct's body and diff it
//! against `EDITOR_STATE_FIELD_CLASSIFICATION`/`EDITOR_FIELD_CLASSIFICATION`
//! in both directions: a new field with no entry fails naming it (forcing a
//! classification decision at the point it's added, not silently); a stale
//! entry for a since-removed field also fails, so the list can't rot into a
//! document nobody trusts.

use super::{assert_fields_classified, struct_field_names, struct_fields_excluding};

/// `(field name, classification)` for every `EditorState` field other
/// than `config` (exempt — `ConfigState`'s wholesale rebuild classifies
/// itself; see its own doc). Three buckets, by how `:reload-config`'s
/// reset treats the field:
/// - `"config: …"` — reset outside `ConfigState`'s own rebuild, by a
///   named mechanism in `reset_config_state`.
/// - `"accounting: …"` — deliberately read, not reset, to judge the
///   reload itself.
/// - `"preserved"` (optionally with a one-clause reason where it isn't
///   obvious) — untouched: buffer content/undo/panes/registers/macros/
///   search/mode, every transient per-dispatch or per-frame flag, and
///   every `Arc` overlay view (self-healing per frame regardless of
///   `config`, so resetting the model they mirror is enough).
const EDITOR_STATE_FIELD_CLASSIFICATION: &[(&str, &str)] = &[
    (
        "buffers",
        "config: clear_languages_all/clear_overrides_all reset language + \
         overrides; content, undo history, and everything else survive",
    ),
    (
        "settings",
        "config: settings_ops::reset_globals rebuilds EditorSettings wholesale",
    ),
    (
        "message_log",
        "accounting: typed_reload_config diffs this before/after the reset \
         to decide whether to report success — resetting it would defeat that",
    ),
    ("mode", "preserved"),
    ("pending_keys", "preserved"),
    ("count", "preserved"),
    ("wait_char", "preserved"),
    ("pending_char", "preserved"),
    ("registers", "preserved"),
    ("kill_ring", "preserved"),
    ("clipboard", "preserved"),
    ("register_prefix", "preserved"),
    ("last_command", "preserved"),
    ("last_paste", "preserved"),
    ("should_quit", "preserved"),
    ("terminate_exit_code", "preserved"),
    ("minibuf", "preserved"),
    ("minibuf_completion", "preserved"),
    ("status_msg", "preserved"),
    ("summary_ttl", "preserved"),
    ("last_find", "preserved"),
    ("search", "preserved"),
    ("focused_pane_id", "preserved"),
    ("panes", "preserved"),
    ("history", "preserved"),
    ("force_full_redraw", "preserved"),
    ("inline_output", "preserved"),
    ("inline_output_entered", "preserved: test-only seam"),
    ("motion_format_scratch", "preserved"),
    ("visual_move_target_cols", "preserved"),
    ("last_repeatable_action", "preserved"),
    ("selection_recipe", "preserved"),
    ("pending_repeat", "preserved"),
    ("insert_session", "preserved"),
    ("autoindent_pending", "preserved"),
    ("explicit_count", "preserved"),
    ("pending_ctrl_extend", "preserved"),
    ("macro_recording", "preserved"),
    ("macro_pending", "preserved"),
    ("replay_queue", "preserved"),
    ("skip_macro_record", "preserved"),
    ("dispatching_typed_command", "preserved"),
    ("is_replaying", "preserved"),
    ("mouse_drag_anchor", "preserved"),
    ("cwd", "preserved"),
    ("lsp_completion_dismiss_pending", "preserved"),
    (
        "completion_menu_view",
        "preserved: Arc view, self-healing per-frame regardless of config",
    ),
    (
        "minibuf_completion_view",
        "preserved: Arc view, self-healing per-frame",
    ),
    (
        "diagnostic_scopes",
        "preserved: ScopeIds are registry-relative, not theme-relative — \
         survive a theme reset",
    ),
    ("inlay_hint_scope", "preserved: registry-relative ScopeId"),
    (
        "virtual_text_fallback_scope",
        "preserved: registry-relative ScopeId",
    ),
    (
        "runtime_scope_cache",
        "preserved: registry-relative ScopeIds",
    ),
    ("popup_view", "preserved: Arc view, self-healing per-frame"),
    (
        "popup_band_view",
        "preserved: Arc view, self-healing per-frame",
    ),
    ("menu_view", "preserved: Arc view, self-healing per-frame"),
    ("drawer_view", "preserved: Arc view, self-healing per-frame"),
    ("picker_view", "preserved: Arc view, self-healing per-frame"),
    ("wake", "preserved: cross-thread waker infra, not config"),
];

/// `(field name, classification)` for every `Editor` field other than
/// `state` and `view` — both exempt: `state: EditorState` is governed by
/// `EDITOR_STATE_FIELD_CLASSIFICATION` above, and `view: EngineView` is a
/// whole rendering-state struct from another crate whose own
/// config-relevant piece (`view.theme`) is already covered by
/// `settings_ops::reset_globals`'s doc. Same three buckets as
/// `EDITOR_STATE_FIELD_CLASSIFICATION`.
const EDITOR_FIELD_CLASSIFICATION: &[(&str, &str)] = &[
    (
        "kitty_enabled",
        "preserved: the probe result reset_config_state itself reads to \
         rebuild ConfigState's keymap with the same kitty defaults",
    ),
    (
        "scripting",
        "config: typed_reload_config drops this to None directly (not via \
         reset_config_state) right before init_scripting rebuilds it",
    ),
    (
        "builtin_cmd_names",
        "config: overwritten wholesale by init_scripting from the fresh \
         registry, every call including a reload's",
    ),
    ("parse_worker", "preserved"),
    ("parse_worker_disconnect_logged", "preserved"),
    (
        "timer_wheel",
        "config: reset_config_state cancels only the Steel-thunk-payload \
         entries (paired with timer_payloads below); native \
         ViewportDebounce timers survive",
    ),
    (
        "timer_payloads",
        "config: reset_config_state removes only the Steel-thunk-payload \
         entries, paired 1:1 with the timer_wheel cancellations above",
    ),
    (
        "viewport_debounce",
        "preserved: indexes the native ViewportDebounce timers that \
         themselves survive the reset",
    ),
    ("last_viewport_key", "preserved"),
    (
        "virtual_lines_synced",
        "preserved: staleness after a reload is forced by \
         DecorationStores::reset bumping the generation counter, not by \
         resetting this map directly",
    ),
    ("lsp", "config: LspState::reset_config()"),
    ("tui_active", "preserved"),
    ("terminal", "preserved"),
    (
        "applied_mouse_mode",
        "preserved: prepare_frame reconciles it lazily against \
         state.settings after a reload, same as any runtime \
         :set mouse-enabled/mouse-select change",
    ),
];

/// Exercises the tricky patterns actually present in `EditorState`: a
/// doc comment, an own-line attribute, a field whose type wraps onto a
/// second line, a generic with an internal tuple (nested commas), and
/// `pub(in crate::editor)` visibility (a `::` path separator inside the
/// parens, before the field's own `:`).
#[test]
fn struct_field_names_handles_wrapped_generics_and_attributes() {
    let body = r#"
        /// doc comment: mentions a colon, must not be read as a field
        pub(crate) buffers: BufferStore,
        #[cfg(test)]
        pub(crate) inline_output_entered: bool,
        pub(crate) minibuf_completion_view:
            Arc<RwLock<Option<crate::ui::completion_overlay::MinibufCompletionView>>>,
        pub(super) pending_hooks: Vec<(hume_scripting::hooks::HookId, Vec<steel::rvals::SteelVal>)>,
        pub(in crate::editor) completion: Option<completion::CompletionSession>,
    "#;
    assert_eq!(
        struct_field_names(body),
        vec![
            "buffers",
            "inline_output_entered",
            "minibuf_completion_view",
            "pending_hooks",
            "completion",
        ]
    );
}

/// Fail oracle: add a field directly to `EditorState` (outside
/// `config: ConfigState`) without adding a matching entry to
/// `EDITOR_STATE_FIELD_CLASSIFICATION` — this test fails naming the
/// field, in either direction (new unclassified field, or a stale
/// entry for a field that no longer exists).
#[test]
fn editor_state_fields_are_classified() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let path = std::path::Path::new(&manifest).join("src/editor/mod.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let fields = struct_fields_excluding(&src, "pub(crate) struct EditorState {", &["config"]);
    assert_fields_classified(
        &fields,
        EDITOR_STATE_FIELD_CLASSIFICATION,
        "EditorState",
        "EDITOR_STATE_FIELD_CLASSIFICATION",
    );
}

/// Fail oracle: add a field directly to `Editor` (outside `state` and
/// `view`, both separately governed) without a matching entry in
/// `EDITOR_FIELD_CLASSIFICATION` — same two-directional check as
/// `editor_state_fields_are_classified`, for the fields
/// `reset_config_state` reaches through `&mut self` directly rather than
/// through `self.state.config`.
#[test]
fn editor_fields_are_classified() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let path = std::path::Path::new(&manifest).join("src/editor/mod.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let fields = struct_fields_excluding(&src, "pub(crate) struct Editor {", &["state", "view"]);
    assert_fields_classified(
        &fields,
        EDITOR_FIELD_CLASSIFICATION,
        "Editor",
        "EDITOR_FIELD_CLASSIFICATION",
    );
}
