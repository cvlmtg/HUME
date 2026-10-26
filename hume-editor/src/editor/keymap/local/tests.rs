use super::*;
use hume_scripting::ConfigOrigin;
use termina::event::{KeyCode, KeyEvent, Modifiers};

fn buffers() -> (BufferId, BufferId) {
    let mut ids: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    (ids.insert(()), ids.insert(()))
}

fn global_with(mode: BindMode, keys: &[KeyEvent], command: &'static str) -> Keymap {
    let mut global = Keymap::empty();
    global.bind(
        mode,
        keys,
        Cow::Borrowed(command),
        false,
        ConfigOrigin::User,
    );
    global
}

fn bind(
    local: &mut LocalKeymaps,
    name: &str,
    mode: BindMode,
    keys: &[KeyEvent],
    command: &'static str,
) {
    local.bind(
        name.into(),
        mode,
        keys,
        Cow::Borrowed(command),
        false,
        ConfigOrigin::User,
    );
}

/// `LocalKeymaps` with one binding in the keymap "k", attached to `buffer`.
fn attached_with(
    buffer: BufferId,
    mode: BindMode,
    keys: &[KeyEvent],
    command: &'static str,
) -> LocalKeymaps {
    let mut local = LocalKeymaps::default();
    bind(&mut local, "k", mode, keys, command);
    local.attach(buffer, "k").expect("k exists");
    local
}

/// The command `keys` run in `buffer`, `"<prefix>"` when more keys are
/// needed, or `"<none>"`.
fn resolved(
    local: &LocalKeymaps,
    global: &Keymap,
    buffer: BufferId,
    mode: BindMode,
    keys: &[KeyEvent],
) -> String {
    match local.resolve(global, buffer, mode, keys).walk {
        WalkResult::Leaf(cmd) => cmd.name.into_owned(),
        WalkResult::Interior => "<prefix>".to_owned(),
        WalkResult::NoMatch => "<none>".to_owned(),
    }
}

#[test]
fn a_new_keymap_has_no_bindings_of_its_own() {
    let (buf, _) = buffers();
    let local = attached_with(buf, BindMode::Normal, &[key!('z')], "zed");
    let global = Keymap::empty();
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('h')]),
        "<none>"
    );
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Insert, &[key!(Tab)]),
        "<none>"
    );
}

#[test]
fn an_attached_binding_hides_the_global_one() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('h')], "move-left");
    let local = attached_with(buf, BindMode::Normal, &[key!('h')], "local-h");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('h')]),
        "local-h"
    );
}

#[test]
fn an_attached_binding_hides_a_global_prefix() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('g'), key!('g')], "goto-file-start");
    let local = attached_with(buf, BindMode::Normal, &[key!('g')], "local-g");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('g')]),
        "local-g"
    );
}

#[test]
fn an_attached_prefix_hides_a_global_binding() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('Z')], "global-z");
    let local = attached_with(buf, BindMode::Normal, &[key!('Z'), key!('a')], "local-za");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('Z')]),
        "<prefix>"
    );
    assert_eq!(
        resolved(
            &local,
            &global,
            buf,
            BindMode::Normal,
            &[key!('Z'), key!('b')]
        ),
        "<none>"
    );
}

#[test]
fn local_and_global_prefixes_combine() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('G'), key!('G')], "toggle-comment");
    let local = attached_with(buf, BindMode::Normal, &[key!('G'), key!('H')], "heading");
    assert_eq!(
        resolved(
            &local,
            &global,
            buf,
            BindMode::Normal,
            &[key!('G'), key!('H')]
        ),
        "heading"
    );
    assert_eq!(
        resolved(
            &local,
            &global,
            buf,
            BindMode::Normal,
            &[key!('G'), key!('G')]
        ),
        "toggle-comment"
    );
}

#[test]
fn the_newest_attached_keymap_wins() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = LocalKeymaps::default();
    bind(&mut local, "old", BindMode::Normal, &[key!('z')], "old-z");
    bind(&mut local, "new", BindMode::Normal, &[key!('z')], "new-z");
    local.attach(buf, "old").expect("old exists");
    local.attach(buf, "new").expect("new exists");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "new-z"
    );
}

#[test]
fn attaching_again_keeps_the_keymap_in_place() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = LocalKeymaps::default();
    bind(&mut local, "old", BindMode::Normal, &[key!('z')], "old-z");
    bind(&mut local, "new", BindMode::Normal, &[key!('z')], "new-z");
    local.attach(buf, "old").expect("old exists");
    local.attach(buf, "new").expect("new exists");
    local.attach(buf, "old").expect("old exists");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "new-z"
    );
}

#[test]
fn attaching_an_unknown_name_changes_nothing_even_after_it_is_bound() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = LocalKeymaps::default();
    assert_eq!(local.attach(buf, "later"), Err(UnknownKeymap));
    bind(
        &mut local,
        "later",
        BindMode::Normal,
        &[key!('z')],
        "later-z",
    );
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "<none>"
    );
}

#[test]
fn detaching_an_unknown_name_is_reported() {
    let (buf, _) = buffers();
    let mut local = LocalKeymaps::default();
    assert_eq!(local.detach(buf, "nothing"), Err(UnknownKeymap));
}

#[test]
fn detaching_a_keymap_that_is_not_attached_is_fine() {
    let (buf, _) = buffers();
    let mut local = LocalKeymaps::default();
    bind(&mut local, "k", BindMode::Normal, &[key!('z')], "zed");
    assert_eq!(local.detach(buf, "k"), Ok(()));
}

#[test]
fn unbinding_never_creates_a_keymap() {
    let (buf, _) = buffers();
    let mut local = LocalKeymaps::default();
    local.unbind("k", BindMode::Normal, &[key!('z')], ConfigOrigin::User);
    assert_eq!(local.attach(buf, "k"), Err(UnknownKeymap));
}

#[test]
fn a_plugins_bind_into_a_named_keymap_stays_below_the_users() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = attached_with(buf, BindMode::Normal, &[key!('z')], "user-z");
    local.bind(
        "k".into(),
        BindMode::Normal,
        &[key!('z')],
        Cow::Borrowed("plugin-z"),
        false,
        ConfigOrigin::Plugin,
    );
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "user-z"
    );
}

#[test]
fn unbinding_in_a_named_keymap_lets_the_global_binding_apply() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('z')], "global-z");
    let mut local = attached_with(buf, BindMode::Normal, &[key!('z')], "local-z");
    local.unbind("k", BindMode::Normal, &[key!('z')], ConfigOrigin::User);
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "global-z"
    );
}

#[test]
fn detaching_one_keymap_keeps_the_others() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('z')], "global-z");
    let mut local = LocalKeymaps::default();
    bind(&mut local, "a", BindMode::Normal, &[key!('a')], "local-a");
    bind(&mut local, "z", BindMode::Normal, &[key!('z')], "local-z");
    local.attach(buf, "a").expect("a exists");
    local.attach(buf, "z").expect("z exists");
    local.detach(buf, "z").expect("z exists");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "global-z"
    );
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('a')]),
        "local-a"
    );
}

#[test]
fn a_forgotten_buffer_has_only_the_global_keys() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('z')], "global-z");
    let mut local = attached_with(buf, BindMode::Normal, &[key!('z')], "local-z");
    local.forget_buffer(buf);
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "global-z"
    );
}

#[test]
fn another_buffer_has_only_the_global_keys() {
    let (buf, other) = buffers();
    let global = global_with(BindMode::Normal, &[key!('z')], "global-z");
    let local = attached_with(buf, BindMode::Normal, &[key!('z')], "local-z");
    assert_eq!(
        resolved(&local, &global, other, BindMode::Normal, &[key!('z')]),
        "global-z"
    );
}

#[test]
fn a_binding_added_after_attaching_applies() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = attached_with(buf, BindMode::Normal, &[key!('a')], "local-a");
    bind(&mut local, "k", BindMode::Normal, &[key!('z')], "local-z");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('z')]),
        "local-z"
    );
}

#[test]
fn extend_mode_tries_each_keymap_whole_before_the_next() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Extend, &[key!('o')], "global-extend-o");
    let local = attached_with(buf, BindMode::Normal, &[key!('o')], "local-o");
    let found = local.resolve(&global, buf, BindMode::Extend, &[key!('o')]);
    assert!(matches!(found.walk, WalkResult::Leaf(ref c) if c.name == "local-o"));
    assert!(!found.extend_override);
}

#[test]
fn an_extend_override_wins_over_normal_in_its_own_keymap() {
    let (buf, _) = buffers();
    let global = Keymap::empty();
    let mut local = LocalKeymaps::default();
    bind(&mut local, "k", BindMode::Normal, &[key!('o')], "local-o");
    bind(
        &mut local,
        "k",
        BindMode::Extend,
        &[key!('o')],
        "local-extend-o",
    );
    local.attach(buf, "k").expect("k exists");
    let found = local.resolve(&global, buf, BindMode::Extend, &[key!('o')]);
    assert!(matches!(found.walk, WalkResult::Leaf(ref c) if c.name == "local-extend-o"));
    assert!(found.extend_override);
}

#[test]
fn normal_mode_ignores_extend_overrides() {
    let (buf, _) = buffers();
    let global = global_with(BindMode::Normal, &[key!('o')], "global-o");
    let local = attached_with(buf, BindMode::Extend, &[key!('o')], "local-extend-o");
    assert_eq!(
        resolved(&local, &global, buf, BindMode::Normal, &[key!('o')]),
        "global-o"
    );
}
