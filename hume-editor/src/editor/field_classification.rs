//! Guards that force a compile error when a field is added to
//! `EditorState`/`Editor` without a reset classification for it — see the
//! two tests' own doc for how.

// `Editor::reset_config_state` resets `EditorState.config: ConfigState`
// wholesale (a field added there is reset by construction — see
// `ConfigState`'s own doc), but every *other* field on `EditorState` — and
// on `Editor` itself, which `reset_config_state` also reaches directly for
// `lsp`/`timer_wheel`/`timer_payloads` — needs a human decision: does
// `:reload-config` reset it too (like `settings`, via
// `settings::ops::reset_globals`), or does it survive untouched (buffers,
// panes, undo history, registers, …)?
//
// The two functions below are never called — each declares a local fn taking
// the struct by value and exhaustively destructures it with no `..`, so a
// field added or removed anywhere in `EditorState`/`Editor` is a *compile
// error* right here (an unknown or unmentioned field name) instead of
// silently defaulting to "survives", correct for most fields but wrong for
// one that should have reset. The classification itself — does the reset
// reach this field, and how — lives in the comment beside each one.

use super::*;

/// See the module doc above. `config` is exempt: `ConfigState`'s own
/// wholesale rebuild classifies itself (see that type's doc).
#[test]
fn editor_state_fields_are_classified() {
    #[allow(dead_code, unused_variables)]
    fn assert_exhaustive(e: EditorState) {
        let EditorState {
            // config: clear_languages_all/clear_overrides_all reset
            // language + overrides; content, undo history, and
            // everything else survive
            buffers: _,
            config: _, // exempt — see ConfigState's own doc
            // config: reset_config_state → input.truncate_to_base()
            input: _,
            mode: _,                // preserved
            pending_keys: _,        // preserved
            count: _,               // preserved
            wait_char: _,           // preserved
            pending_char: _,        // preserved
            registers: _,           // preserved
            kill_ring: _,           // preserved
            clipboard: _,           // preserved
            register_prefix: _,     // preserved
            paste_stamp: _,         // preserved
            should_quit: _,         // preserved
            terminate_exit_code: _, // preserved
            minibuf: _,             // preserved
            minibuf_completion: _,  // preserved
            status_msg: _,          // preserved
            summary_ttl: _,         // preserved
            // accounting: typed_reload_config diffs this before/after
            // the reset to decide whether to report success —
            // resetting it would defeat that
            message_log: _,
            // config: settings::ops::reset_globals rebuilds
            // EditorSettings wholesale
            settings: _,
            last_find: _,                       // preserved
            search: _,                          // preserved
            focus: _,                           // preserved
            tabs: _,                            // preserved
            panes: _,                           // preserved
            history: _,                         // preserved
            force_full_redraw: _,               // preserved
            inline_output: _,                   // preserved
            visual_move_target_display_cols: _, // preserved
            last_repeatable_action: _,          // preserved
            selection_recipe: _,                // preserved
            selection_recipe_writes: _,         // preserved
            command_refused: _,                 // preserved
            pending_repeat: _,                  // preserved
            insert_session: _,                  // preserved
            explicit_count: _,                  // preserved
            pending_ctrl_extend: _,             // preserved
            macro_recording: _,                 // preserved
            macro_pending: _,                   // preserved
            replay_queue: _,                    // preserved
            skip_macro_record: _,               // preserved
            dispatching_typed_command: _,       // preserved
            is_replaying: _,                    // preserved
            message_logged_this_input: _,       // preserved
            // config: resync_config_state clears this so
            // detect_buffer_enter's diff re-raises OnBufferEnter for
            // the focused buffer
            last_entered_buffer: _,
            mouse_drag_anchor: _,              // preserved
            cwd: _,                            // preserved
            lsp_completion_dismiss_pending: _, // preserved
            views: _, // preserved: Arc views, self-healing per-frame regardless of config
            tabline_view: _, // preserved: self-healing per-frame regardless of config
            wake: _,  // preserved: cross-thread waker infra, not config
        } = e;
    }
}

/// See the module doc above. `state`/`view` are exempt: `state:
/// EditorState` is governed by `editor_state_fields_are_classified`
/// above, and `view: EngineView` is a whole rendering-state struct from
/// another crate whose own config-relevant piece (`view.theme`) is
/// already covered by `settings::ops::reset_globals`'s doc.
#[test]
fn editor_fields_are_classified() {
    #[allow(dead_code, unused_variables)]
    fn assert_exhaustive(e: Editor) {
        let Editor {
            state: _, // exempt — see editor_state_fields_are_classified
            view: _,  // exempt — see this test's own doc
            // preserved: the probe result reset_config_state itself
            // reads to rebuild ConfigState's keymap with the same
            // kitty defaults
            kitty_enabled: _,
            // config: typed_reload_config drops this to None directly
            // (not via reset_config_state) right before
            // init_scripting rebuilds it
            scripting: _,
            // preserved: :reload-config must re-evaluate the source
            // the session booted from, not the default init.scm
            config_source: _,
            // config: overwritten wholesale by init_scripting from
            // the fresh registry, every call including a reload's
            builtin_cmd_names: _,
            parse_worker: _,                   // preserved
            parse_worker_disconnect_logged: _, // preserved
            // config: reset_config_state cancels only the
            // Steel-thunk-payload entries (paired with
            // timer_payloads below); native ViewportDebounce timers
            // survive
            timer_wheel: _,
            // config: reset_config_state removes only the
            // Steel-thunk-payload entries, paired 1:1 with the
            // timer_wheel cancellations above
            timer_payloads: _,
            // preserved: indexes the native ViewportDebounce timers
            // that themselves survive the reset
            viewport_debounce: _,
            last_viewport_key: _,      // preserved
            last_tabline_signature: _, // preserved
            // preserved: staleness after a reload is forced by
            // DecorationStores::reset bumping the generation
            // counter, not by resetting this map directly
            virtual_lines_synced: _,
            lsp: _, // config: LspState::reset_config()
            // preserved: Editor::run sets it On on entry and Off on
            // exit, and a :reload-config can only run from inside
            // that loop
            tui: _,
            // preserved: prepare_frame reconciles it lazily against
            // state.settings after a reload, same as any runtime
            // :set mouse-enabled/mouse-select change
            applied_mouse_mode: _,
            // preserved: drained by apply_startup_positions at the
            // first settle, long before any :reload-config could run
            startup_positions: _,
        } = e;
    }
}
