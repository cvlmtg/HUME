//! Applying the effects a Steel eval queued, and ending a script session.

use hume_core::{BufferId, Severity};
use hume_scripting::Effect;
use hume_scripting::host::KeymapTarget;

use crate::editor::Editor;
use crate::editor::keymap::{LocalKeymaps, UnknownKeymap};

impl Editor {
    /// Apply every effect a Steel eval queued, in the exact order the
    /// script emitted them (`hume_scripting::Effect`): one ordered log, not
    /// separate channels with a hardcoded apply order. Consecutive
    /// `Effect::LanguageReg` entries are grouped into one
    /// `apply_pending_language_regs` call so a large run (e.g.
    /// `languages.scm`'s 342 `define-language!` calls) rebuilds the glob
    /// matcher once, not once per entry. Consecutive `Effect::LspServerOp`
    /// entries are likewise one `apply_lsp_server_ops` call, so buffers
    /// reconcile to a run of registry changes once. Every other effect kind
    /// applies one at a time.
    ///
    /// Finishes by draining `state.config.pending_language_detection`, which covers every
    /// buffer a disjoint-borrow Steel path opened via `buffer::lifecycle::
    /// open_buffer_and_notify` this eval (`(open-buffer! …)`, workspace edits,
    /// goto-definition). `drain_pending_work` would reach them on its next
    /// pass; draining here lets the next item of the same batch see their
    /// language. *After* the effect loop, not before: a script that opens a
    /// buffer and registers its language in the same eval must have the
    /// registration land first.
    pub(in crate::editor) fn apply_script_effects(&mut self, effects: Vec<Effect>) {
        let mut effects: std::collections::VecDeque<Effect> = effects.into();
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::LanguageReg(reg) => {
                    let batch = take_run(&mut effects, reg, |effect| match effect {
                        Effect::LanguageReg(next) => Ok(next),
                        other => Err(Box::new(other)),
                    });
                    self.apply_pending_language_regs(batch);
                }
                Effect::LspServerOp(op) => {
                    let batch = take_run(&mut effects, op, |effect| match effect {
                        Effect::LspServerOp(next) => Ok(next),
                        other => Err(Box::new(other)),
                    });
                    self.apply_lsp_server_ops(batch);
                }
                Effect::SetBufferLanguage {
                    buffer,
                    language,
                    origin,
                } => {
                    // `buffer` may have closed between the Steel call that
                    // queued this effect and this drain (e.g. `(set-buffer-
                    // option! bid "language" "rust") (close-buffer! bid)` in
                    // one eval), the same shape as `LspRequest`/`LspNotify`'s own
                    // `try_get` guard in `bridge.rs`. The language write uses
                    // the panicking `get_mut`, so this must check first rather
                    // than let it panic.
                    if !self.effect_buffer_gone("set-buffer-option!", buffer) {
                        self.set_buffer_language_by_name(buffer, language.as_deref(), origin)
                    }
                }
                Effect::GrammarSweep(name) => {
                    let id = self.state.config.languages.id_of(&name).expect(
                        "GrammarSweep is only emitted right after attach_grammar interns the name",
                    );
                    self.sweep_buffers_for_grammars(vec![id])
                }
                Effect::LspRequest(req) => self.state.lsp_send_request(&self.view, req),
                Effect::LspNotify(notify) => self.state.lsp_send_notify(notify),
                Effect::BindKey {
                    mode,
                    keys,
                    cmd,
                    force_extend,
                    keymap,
                    origin,
                } => {
                    let cmd = std::borrow::Cow::Owned(cmd);
                    let hidden = match keymap {
                        KeymapTarget::Global => {
                            self.state
                                .config
                                .keymap
                                .bind(mode, &keys, cmd, force_extend, origin)
                        }
                        // Bound after init too, so checked here rather than by
                        // the post-init lint, which covers the global keymap.
                        KeymapTarget::Named(name) => {
                            if self.state.config.registry.get_mappable(&cmd).is_none() {
                                self.report_unbound_key_command(&cmd);
                            }
                            self.state.config.local_keymaps.bind(
                                name,
                                mode,
                                &keys,
                                cmd,
                                force_extend,
                                origin,
                            )
                        }
                    };
                    if hidden {
                        self.state.report_shadowed(
                            "bind-key!",
                            format_args!("{mode:?} binding of these keys"),
                        );
                    }
                }
                Effect::UnbindKey {
                    mode,
                    keys,
                    keymap,
                    origin,
                } => {
                    let hidden = match keymap {
                        KeymapTarget::Global => {
                            self.state.config.keymap.unbind(mode, &keys, origin)
                        }
                        KeymapTarget::Named(name) => self
                            .state
                            .config
                            .local_keymaps
                            .unbind(&name, mode, &keys, origin),
                    };
                    if hidden {
                        self.state.report_shadowed(
                            "unbind-key!",
                            format_args!("{mode:?} binding of these keys"),
                        );
                    }
                }
                Effect::AttachKeymap { buffer, name } => {
                    self.change_attachment("attach-keymap!", buffer, &name, LocalKeymaps::attach)
                }
                Effect::DetachKeymap { buffer, name } => {
                    self.change_attachment("detach-keymap!", buffer, &name, LocalKeymaps::detach)
                }
                Effect::RegisterCompletionSource(reg) => self.register_completion_source(reg),
                Effect::SetTriggers {
                    listener,
                    scope,
                    chars,
                } => {
                    // Checked at apply time, not by the builtin that queued
                    // this effect: an earlier effect in the same batch may
                    // have just registered the source, so whether it exists
                    // can't be known any sooner than this.
                    let applied = self.state.set_triggers(listener, scope, chars);
                    if let Err(e) = applied {
                        self.report(Severity::Error, e);
                    }
                }
            }
        }
        self.detect_pending_languages();
    }

    /// Attach or detach the keymap `name` in `buffer`. A name nothing is bound
    /// into warns, since the call then does nothing.
    fn change_attachment(
        &mut self,
        builtin: &str,
        buffer: BufferId,
        name: &str,
        change: fn(&mut LocalKeymaps, BufferId, &str) -> Result<(), UnknownKeymap>,
    ) {
        if !self.effect_buffer_gone(builtin, buffer)
            && change(&mut self.state.config.local_keymaps, buffer, name).is_err()
        {
            self.report(
                Severity::Warning,
                format!("{builtin}: no keymap named '{name}'"),
            );
        }
    }

    /// Whether `buffer`, which an effect from `builtin` targets, closed after
    /// the effect was queued. Traced when it did.
    fn effect_buffer_gone(&mut self, builtin: &str, buffer: BufferId) -> bool {
        let gone = self.state.buffers.try_get(buffer).is_none();
        if gone {
            self.report(
                Severity::Trace,
                format!("{builtin}: buffer {buffer:?} no longer exists"),
            );
        }
        gone
    }

    /// End a Steel session that returned `Result<Vec<Effect>, EvalError>`:
    /// close the `#:inline-output` bracket if a builtin opened it, report the
    /// session's queued `log!` messages, and apply its effects either way (an
    /// `EvalError` still carries whatever committed before the error).
    /// Returns the error's message unreported.
    ///
    /// Every such Steel entry point ends here: a command body, a hook, a
    /// queued-call batch, `init.scm`, a `runtime/` file, and a plugin
    /// activation. The close comes first so a Steel error raised after screen
    /// entry still gets the TUI restored. The flush comes before any error
    /// report, so a body's log lines land ahead of its own error. `init.scm`
    /// and `runtime/` files run before `self.scripting` is installed, so they
    /// come through [`Self::apply_init_result`], which reports their messages
    /// first.
    pub(in crate::editor) fn finish_script_session(
        &mut self,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
    ) -> Result<(), String> {
        self.close_inline_output_bracket();
        self.flush_script_messages();
        match result {
            Ok(effects) => {
                self.apply_script_effects(effects);
                Ok(())
            }
            Err(e) => {
                self.apply_script_effects(e.effects);
                Err(e.message)
            }
        }
    }

    /// [`Self::finish_script_session`], reporting an error as
    /// `{err_prefix}{message}`. `err_prefix` distinguishes which eval failed
    /// (a hook, a queued call, `init.scm`, a `runtime/` file) in the reported
    /// message; pass `""` for a caller with nothing to prefix.
    pub(in crate::editor) fn apply_script_result(
        &mut self,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
        err_prefix: &str,
    ) {
        if let Err(message) = self.finish_script_session(result) {
            self.report(Severity::Error, format!("{err_prefix}{message}"));
        }
    }
}

/// `first` and the effects right behind it that `take` unwraps, in order;
/// the first effect `take` refuses stays at the front of `effects`.
fn take_run<T>(
    effects: &mut std::collections::VecDeque<Effect>,
    first: T,
    take: fn(Effect) -> Result<T, Box<Effect>>,
) -> Vec<T> {
    let mut run = vec![first];
    while let Some(effect) = effects.pop_front() {
        match take(effect) {
            Ok(next) => run.push(next),
            Err(other) => {
                effects.push_front(*other);
                break;
            }
        }
    }
    run
}
