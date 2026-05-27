use super::super::{ops, Severity};
use super::super::Editor;
use crate::core::error::CommandError;

// ── Message log ──────────────────────────────────────────────────────────────

/// `:messages` — open the message log in a read-only buffer.
///
/// Displays all logged warnings, errors, and trace entries accumulated during
/// the session. Cursor starts at the last entry (most recent). Dismiss with
/// `:bd` or switch away with `:b#`.
pub fn typed_messages(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let content = ed.message_log.format_for_display();
    if content.is_empty() {
        ed.report(Severity::Info, "No messages".to_string());
        return Ok(());
    }
    // Position cursor at last content line (most recent entry).
    let last_line = content.lines().count().saturating_sub(1);
    ed.message_log.mark_all_seen();
    ed.open_read_only_view("[messages]", &content, last_line);
    Ok(())
}

/// `:ls` / `:list-buffers` — open a read-only buffer listing every open buffer.
///
/// Each row shows: 1-based index, current (`%`) / alternate (`#`) marker,
/// dirty (`+`) flag, short name, and home-shortened absolute path.
/// Cursor is placed on the row corresponding to the currently focused buffer.
pub fn typed_list_buffers(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let current = ed.focused_buffer_id();
    let alternate = ed.alternate_buffer();

    let header = format!("{:>4}    {:<32}  {}\n", "buf", "name", "path");
    let mut out = header;
    // The header occupies rope line 0; each buffer occupies rope line `idx + 1`.
    // `current_rope_line` tracks that index so the cursor opens on the right row.
    let mut current_rope_line: usize = 1;

    for (idx, (id, buf)) in ed.buffers.iter().enumerate() {
        let display_num = idx + 1;
        let rope_line = idx + 1; // 1 header line before buffer rows

        let cur_marker = if id == current {
            '%'
        } else if matches!(alternate, Some(alt) if id == alt) {
            '#'
        } else {
            ' '
        };
        let dirty_marker = if buf.is_dirty() { '+' } else { ' ' };

        let path_ref = buf.path();
        let name = path_ref
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(buf.label.as_deref().unwrap_or("*scratch*"));
        let path = path_ref
            .map(crate::os::path::shorten_home)
            .unwrap_or_default();

        out.push_str(&format!(
            "{:>4}  {}{}  {:<32}  {}\n",
            display_num, cur_marker, dirty_marker, name, path
        ));

        if id == current {
            current_rope_line = rope_line;
        }
    }

    ed.open_read_only_view("[buffers]", &out, current_rope_line);
    Ok(())
}

/// `:plugin-status` / `:plugins` — show all declared plugins, their load
/// state, and (for still-waiting plugins) which triggers they are waiting on.
pub fn typed_plugin_status(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let out = if let Some(host) = ed.scripting.as_ref() {
        host.lazy_registry.format_status()
    } else {
        ed.report(Severity::Info, "Scripting disabled".to_string());
        return Ok(());
    };
    if out.is_empty() {
        ed.report(Severity::Info, "No plugins declared".to_string());
        return Ok(());
    }
    ed.open_read_only_view("[plugin-status]", &out, 0);
    Ok(())
}

/// `:reload-config` — drop the scripting engine and re-evaluate `init.scm`
/// from scratch, restoring a clean slate.
///
/// Stale `SteelBacked` entries from the previous init must be removed from the
/// registry before `init_scripting()` runs: otherwise the new `builtin_names`
/// set (built from `registry.names()`) would contain every Steel command from
/// the prior load, and every `(define-command!)` in the re-evaluated
/// `init.scm` would fail the builtin-conflict check in
/// `editor/src/scripting/builtins/commands.rs` with "conflicts with a built-in
/// command and cannot be redefined".
pub fn typed_reload_config(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.scripting = None;
    ed.registry.unregister_dynamic_commands();
    ed.init_scripting();
    ed.report(Severity::Info, "Config reloaded".to_string());
    Ok(())
}

// ── :split / :vsplit typed stubs ──────────────────────────────────────────────

pub fn typed_split(
    _ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub fn typed_vsplit(
    _ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    Err(CommandError(":vsplit not yet implemented".into()))
}

/// `:theme <name>` — load a theme by name from the theme search path.
///
/// On success the engine view's theme is replaced and re-baked.
/// On failure a warning is shown and the current theme is left unchanged.
pub fn typed_theme(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        let current: &str = if ed.settings.theme.is_empty() {
            super::DEFAULT_THEME_LABEL
        } else {
            &ed.settings.theme
        };
        // NLL: `current` borrow of ed.settings.theme ends inside format!(), before report().
        ed.report(Severity::Info, format!("Current theme: {current}"));
        return Ok(());
    };
    if ops::load_theme_by_name(
        &mut ed.engine_view,
        &mut ed.message_log,
        &mut ed.status_msg,
        name,
    ) {
        ed.settings.theme = name.to_owned();
    }
    Ok(())
}

/// `:theme-debug` — print the resolved style chain for key UI scopes.
///
/// Reports the scope name, resolution chain, and final fg/bg/modifiers for
/// the cursor, selection, and cursorline scopes from the active theme.
pub fn typed_theme_debug(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use ratatui::style::Color;

    fn color_str(c: Option<Color>) -> String {
        match c {
            Some(Color::Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            Some(other) => format!("{other:?}"),
            None => "-".to_owned(),
        }
    }

    fn scope_chain(theme: &engine::theme::Theme, scope: &str) -> String {
        // Walk the dot-notation prefix chain and collect names that have entries.
        let mut chain: Vec<&str> = Vec::new();
        let mut cur = scope;
        loop {
            if theme.raw_contains(cur) {
                chain.push(cur);
            }
            match cur.rfind('.') {
                Some(dot) => cur = &cur[..dot],
                None => break,
            }
        }
        if chain.is_empty() {
            format!("{scope} → default")
        } else {
            chain.join(" → ")
        }
    }

    let theme = &ed.engine_view.theme;
    let name = if ed.settings.theme.is_empty() {
        super::DEFAULT_THEME_LABEL
    } else {
        &ed.settings.theme
    };

    let scopes = [
        "ui.cursor.primary",
        "ui.cursor",
        "ui.cursor.insert",
        "ui.selection",
        "ui.cursorline",
        "ui.statusline",
    ];

    let mut lines = vec![format!("Theme: {name}")];
    for scope in scopes {
        let style = theme.resolve_by_name(engine::types::Scope(scope));
        let chain = scope_chain(theme, scope);
        lines.push(format!(
            "  {scope}: chain={chain} fg={} bg={}{}",
            color_str(style.fg),
            color_str(style.bg),
            if style.modifiers.is_empty() {
                String::new()
            } else {
                format!(" modifiers={:?}", style.modifiers)
            },
        ));
    }

    ed.report(Severity::Info, lines.join("\n"));
    Ok(())
}

pub fn typed_version(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.report(Severity::Info, format!("hume {}", crate::VERSION));
    Ok(())
}

pub fn typed_tutor(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    // Resolve the install source. Fail fast on missing runtime or file.
    let Some(runtime) = crate::os::dirs::runtime_dir() else {
        return Err(CommandError(
            "runtime directory not found (set HUME_RUNTIME to override)".into(),
        ));
    };
    let source_path = runtime.join("tutor.txt");
    let source = std::fs::canonicalize(&source_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CommandError(format!(
                "tutor.txt not found at {} (set HUME_RUNTIME to override)",
                source_path.display()
            ))
        } else {
            CommandError(format!("could not access tutor.txt: {e}"))
        }
    })?;

    // Compute a per-process tmp path so `:w` never touches the install source.
    // Canonicalize the parent dir (which we create) so the path matches what
    // BufferStore stores on macOS (/private/var/... vs /var/...).
    let tmp_dir = std::env::temp_dir().join(format!("hume-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| CommandError(format!("could not create tutor tmp dir: {e}")))?;
    let canonical_tmp = std::fs::canonicalize(&tmp_dir)
        .map_err(|e| CommandError(format!("could not canonicalize tutor tmp dir: {e}")))?
        .join("tutor.txt");

    // If a buffer is already open at the tmp path, switch — no re-copy so that
    // unsaved in-memory edits are preserved.
    if let Some(bid) = ed.buffers.find_by_path(&canonical_tmp) {
        ed.switch_to_buffer_with_jump(bid);
        return Ok(());
    }

    // No live buffer at tmp. Copy fresh source content (overwrites any stale
    // file from a prior `:bd!`), then open the copy.
    std::fs::copy(&source, &canonical_tmp)
        .map_err(|e| CommandError(format!("could not copy tutor.txt to tmp: {e}")))?;
    let (bid, _) = ed
        .open_or_dedup(&canonical_tmp)
        .map_err(|e| CommandError(format!("could not open tutor copy: {e}")))?;
    ed.switch_to_buffer_with_jump(bid);
    Ok(())
}
