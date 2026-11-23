//! Following the link under the cursor in a Markdown buffer.

use super::*;

/// The plugin's editor on `a.md` holding `marked`, selected as it marks,
/// with each of `files` written as `(name, text)` beside it.
fn linking(marked: &str, files: &[(&str, &str)]) -> Preview {
    let mut p = preview();
    for (name, text) in files {
        std::fs::write(p.tmp.path().join(name), text).unwrap();
    }
    p.open_marked("a.md", marked);
    p
}

/// The state of `b.md`, holding `b`, once `[see](b.md#{fragment})` is
/// followed into it and it parses.
fn landed(fragment: &str, b: &str) -> String {
    let mut p = linking(&format!("[-[s]>ee](b.md#{fragment})\n"), &[("b.md", b)]);
    assert_eq!(follow(&mut p), "t", "{b:?}");
    p.parse();
    state(&p.ed)
}

/// What `markdown/follow-link` returned in `p`, `t` or `f`, or the error it
/// raised. The result is read by a later command, since a warning the call
/// logs keeps the status line.
fn follow(p: &mut Preview) -> String {
    run(
        &mut p.ed,
        p.tmp.path(),
        r#"(define follow-result "raised")
           (define-command! "follow-probe" "" (lambda (pane) (set! follow-result (if (call! "markdown/follow-link" pane) "t" "f"))))"#,
    );
    call(&mut p.ed, "follow-probe");
    let raised = status(&p.ed);
    match log_probe(&mut p.ed, p.tmp.path(), "follow-result").as_str() {
        "raised" => raised,
        result => result.to_owned(),
    }
}

/// Defines `lsp/goto-definition` as a command writing `?` at the start of
/// the buffer, so the text shows the language server was asked.
fn stub_server(p: &mut Preview) {
    run(
        &mut p.ed,
        p.tmp.path(),
        r#"(define-command! "lsp/goto-definition" "" (lambda (pane) (edit-text! pane (list (hash 'start-char 0 'end-char 0 'text "?")))))"#,
    );
}

const OPEN_FAILED: &str = "markdown: could not open ";

/// What `markdown/follow-link` handed the system's opener in `p`. The
/// editor's search path holds only `open` and `xdg-open`, each a script that
/// prints its argument and fails, so the warning that failure logs names the
/// argument.
fn opened(p: &mut Preview) -> String {
    let bin = tempfile::tempdir().unwrap();
    for name in ["open", "xdg-open"] {
        hume_platform::path::test_support::write_script(
            bin.path(),
            name,
            "printf '%s' \"$1\" >&2\nexit 1\n",
            0o755,
        );
    }
    p.ed.state.dirs.search_path = Some(bin.path().as_os_str().to_owned());
    assert_eq!(follow(p), "t");
    drain_until(&mut p.ed, |ed| {
        warning_log(ed).iter().any(|w| w.contains(OPEN_FAILED))
    });
    let warning = warning_log(&p.ed)
        .into_iter()
        .find(|w| w.contains(OPEN_FAILED))
        .unwrap();
    let after = warning.split_once(OPEN_FAILED).unwrap().1;
    after.split(": ").next().unwrap().to_owned()
}

#[test]
fn a_link_to_a_markdown_file_opens_it() {
    let mut p = linking("see [-[b]>](b.md)\n", &[("b.md", "# B\n")]);
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[#]> B\n");
}

#[test]
fn a_link_to_a_heading_moves_to_it() {
    let mut p = linking(
        "[-[s]>ee](#second-part)\n\n# First\n\n## Second part\n",
        &[],
    );
    assert_eq!(follow(&mut p), "t");
    assert_eq!(
        state(&p.ed),
        "[see](#second-part)\n\n# First\n\n-[#]># Second part\n"
    );
}

#[test]
fn a_link_to_a_heading_of_another_file_skips_code_blocks() {
    let b = "# B\n\n```\n## Part two\n```\n\n## Part two ##\n";
    assert_eq!(
        landed("part-two", b),
        b.replacen("## Part two ##", "-[#]># Part two ##", 1)
    );
}

#[test]
fn a_link_to_an_underlined_heading_moves_to_it() {
    let b = "Intro\n\nPart Two\n--------\n";
    assert_eq!(landed("part-two", b), "Intro\n\n-[P]>art Two\n--------\n");
}

#[test]
fn a_link_to_a_missing_heading_opens_the_file_at_its_start() {
    let mut p = linking("[-[s]>ee](b.md#nowhere)\n", &[("b.md", "# B\n")]);
    assert_eq!(follow(&mut p), "t");
    p.parse();
    assert_eq!(state(&p.ed), "-[#]> B\n");
    assert!(
        warning_log(&p.ed)
            .iter()
            .any(|w| w.contains("no heading #nowhere")),
        "{:?}",
        warning_log(&p.ed)
    );
}

#[test]
fn a_fence_closes_only_on_a_run_as_long_as_its_opener() {
    let b = "# B\n\n````\n```rust\n## Install\n```\n````\n\n## Install\n";
    assert_eq!(
        landed("install", b),
        "# B\n\n````\n```rust\n## Install\n```\n````\n\n-[#]># Install\n"
    );
}

#[test]
fn front_matter_holds_no_heading() {
    let b = "---\ntitle: Intro\n---\n\n## Title: Intro\n";
    assert_eq!(
        landed("title-intro", b),
        "---\ntitle: Intro\n---\n\n-[#]># Title: Intro\n"
    );
}

#[test]
fn a_heading_in_a_quote_or_a_list_item_is_found() {
    for (b, shown) in [
        ("# B\n\n> ## Setup\n", "# B\n\n> -[#]># Setup\n"),
        ("# B\n\n- ## Setup\n", "# B\n\n- -[#]># Setup\n"),
    ] {
        assert_eq!(landed("setup", b), shown, "{b:?}");
    }
}

#[test]
fn a_heading_is_named_by_the_text_the_preview_shows() {
    for (b, fragment, shown) in [
        (
            "# B\n\n## [API](api.md)\n",
            "api",
            "# B\n\n-[#]># [API](api.md)\n",
        ),
        (
            "# B\n\n## [1.2](https://x.io/v1.2) - Q&amp;A\n",
            "12---qa",
            "# B\n\n-[#]># [1.2](https://x.io/v1.2) - Q&amp;A\n",
        ),
    ] {
        assert_eq!(landed(fragment, b), shown, "{b:?}");
    }
}

#[test]
fn a_repeated_heading_is_named_with_its_count() {
    let b = "## Setup\n\nx\n\n## Setup\n\ny\n\n## Setup\n";
    assert_eq!(
        landed("setup-2", b),
        "## Setup\n\nx\n\n## Setup\n\ny\n\n-[#]># Setup\n"
    );
}

#[test]
fn a_bare_hash_moves_to_the_start_with_no_warning() {
    let mut p = linking("# A\n\n[-[t]>op](#)\n", &[]);
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[#]> A\n\n[top](#)\n");
    assert_eq!(warning_log(&p.ed), Vec::<String>::new());
}

#[test]
fn a_move_before_the_file_is_parsed_keeps_the_cursor() {
    let b = "# B\nmore\n\n## Part two\n";
    let mut p = linking("[-[s]>ee](b.md#part-two)\n", &[("b.md", b)]);
    assert_eq!(follow(&mut p), "t");
    call(&mut p.ed, "move-down");
    p.parse();
    assert_eq!(state(&p.ed), "# B\n-[m]>ore\n\n## Part two\n");
}

#[test]
fn a_fragment_into_a_file_that_is_not_markdown_opens_it_at_its_start() {
    let mut p = linking("[-[n]>otes](n.txt#a)\n", &[("n.txt", "x\n# A\n")]);
    assert_eq!(follow(&mut p), "t");
    p.wait();
    assert_eq!(state(&p.ed), "-[x]>\n# A\n");
}

/// Draws frames until the focused buffer's tree matches its text, with the
/// held clock standing still.
fn parse_now(p: &mut Preview) {
    let bid = p.ed.focused_buffer_id();
    let (width, height) = p.size;
    poll_until(
        &mut p.ed,
        |ed| {
            frame(ed, width, height);
            ed.settle();
        },
        |ed| ed.state.buffers.get(bid).current_layers().is_some(),
    );
}

/// Moves `p`'s held clock on by `ms` and settles.
fn advance(p: &mut Preview, ms: u64) {
    p.ed.state.clock.advance(Duration::from_millis(ms));
    p.ed.settle();
}

/// The warnings in `p`'s log that say no heading matches `#fragment`.
fn missed(p: &Preview, fragment: &str) -> usize {
    let warning = format!("markdown: no heading #{fragment}");
    warning_log(&p.ed)
        .iter()
        .filter(|w| w.contains(&warning))
        .count()
}

#[test]
fn a_file_that_is_not_markdown_warns_after_a_second() {
    let mut p = linking("[-[n]>otes](n.txt#a)\n", &[("n.txt", "x\n# A\n")]);
    assert_eq!(follow(&mut p), "t");
    advance(&mut p, 900);
    assert_eq!(missed(&p, "a"), 0);
    advance(&mut p, 100);
    assert_eq!(missed(&p, "a"), 1);
    assert_eq!(state(&p.ed), "-[x]>\n# A\n");
}

#[test]
fn a_heading_found_after_the_warning_still_moves_to_it() {
    let mut p = linking("[-[n]>otes](n.txt#a)\n", &[("n.txt", "x\n# A\n")]);
    assert_eq!(follow(&mut p), "t");
    advance(&mut p, 1000);
    assert_eq!(missed(&p, "a"), 1);
    run(
        &mut p.ed,
        p.tmp.path(),
        r#"(define-command! "to-markdown" "" (lambda (pane) (set-buffer-option! pane 'language "markdown")))"#,
    );
    call(&mut p.ed, "to-markdown");
    parse_now(&mut p);
    assert_eq!(state(&p.ed), "x\n-[#]> A\n");
}

#[test]
fn a_heading_found_by_the_parse_logs_no_warning() {
    let b = "# B\n\n## Part two\n";
    let mut p = linking("[-[s]>ee](b.md#part-two)\n", &[("b.md", b)]);
    assert_eq!(follow(&mut p), "t");
    parse_now(&mut p);
    advance(&mut p, 2000);
    assert_eq!(state(&p.ed), "# B\n\n-[#]># Part two\n");
    assert_eq!(warning_log(&p.ed), Vec::<String>::new());
}

#[test]
fn the_next_follow_drops_the_wait_and_its_warning() {
    let mut p = linking("[-[n]>otes](n.txt#a)\n", &[("n.txt", "x\n# A\n")]);
    assert_eq!(follow(&mut p), "t");
    call(&mut p.ed, "goto-alternate-buffer");
    assert_eq!(state(&p.ed), "[-[n]>otes](n.txt#a)\n");
    call(&mut p.ed, "markdown/follow-link");
    assert_eq!(state(&p.ed), "-[x]>\n# A\n");
    advance(&mut p, 2000);
    assert_eq!(missed(&p, "a"), 1);
}

#[test]
fn a_percent_encoded_path_names_the_decoded_file() {
    let mut p = linking(
        "[-[n]>otes](my%20notes.md)\n",
        &[("my notes.md", "hello\n")],
    );
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[h]>ello\n");
}

#[test]
fn an_encoded_hash_in_a_path_names_a_file_with_a_hash() {
    let mut p = linking("[-[n]>otes](a%23b.md)\n", &[("a#b.md", "hello\n")]);
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[h]>ello\n");
}

#[test]
fn a_percent_escape_that_is_not_utf_8_raises() {
    let mut p = linking("[-[x]>](%FF.md)\n", &[]);
    let raised = follow(&mut p);
    assert!(
        raised.contains("%FF.md is not valid percent-encoded UTF-8"),
        "{raised}"
    );
}

#[test]
fn a_percent_encoded_fragment_names_the_decoded_heading() {
    let mut p = linking("[-[c]>](#caf%C3%A9)\n\n## Café\n", &[]);
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "[c](#caf%C3%A9)\n\n-[#]># Café\n");
}

#[test]
fn a_backslash_escape_in_a_path_names_the_escaped_character() {
    let mut p = linking(
        "[-[n]>otes](notes\\(1\\).md)\n",
        &[("notes(1).md", "hello\n")],
    );
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[h]>ello\n");
}

#[test]
fn an_entity_in_a_web_address_names_its_character() {
    let mut p = linking("[-[s]>ite](https://a.com/?a=1&amp;b=2)\n", &[]);
    assert_eq!(opened(&mut p), "https://a.com/?a=1&b=2");
}

#[test]
fn a_link_to_a_text_file_opens_it() {
    let mut p = linking("[-[n]>otes](n.txt)\n", &[("n.txt", "hello\n")]);
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[h]>ello\n");
}

#[test]
fn a_link_to_a_missing_file_raises() {
    let mut p = linking("[-[g]>one](gone.md)\n", &[]);
    assert!(follow(&mut p).contains("no file"), "{}", status(&p.ed));
    assert_eq!(state(&p.ed), "[-[g]>one](gone.md)\n");
}

#[test]
fn off_a_link_nothing_happens_even_with_a_language_server() {
    for marked in ["[see](b.md) -[t]>ext\n", "a -[[]> [x\n"] {
        let mut p = linking(marked, &[("b.md", "# B\n")]);
        stub_server(&mut p);
        assert_eq!(follow(&mut p), "f", "{marked:?}");
        assert_eq!(state(&p.ed), marked, "{marked:?}");
    }
}

#[test]
fn a_wiki_or_reference_link_asks_the_language_server() {
    for (marked, asked) in [
        ("a [-[T]>ODO] b\n", "?a [-[T]>ODO] b\n"),
        ("see [[-[b]>]]\n", "?see [[-[b]>]]\n"),
        ("see -[[]>[b]]\n", "?see -[[]>[b]]\n"),
        ("see [[b]-[]]>\n", "?see [[b]-[]]>\n"),
        ("see [[b|-[c]>]]\n", "?see [[b|-[c]>]]\n"),
        ("[-[x]>][y]\n\n[y]: b.md\n", "?[-[x]>][y]\n\n[y]: b.md\n"),
        ("[-[x]>][]\n\n[x]: b.md\n", "?[-[x]>][]\n\n[x]: b.md\n"),
    ] {
        let mut p = linking(marked, &[("b.md", "# B\n")]);
        stub_server(&mut p);
        assert_eq!(follow(&mut p), "t", "{marked:?}");
        assert_eq!(state(&p.ed), asked, "{marked:?}");
    }
}

#[test]
fn without_the_inline_grammar_nothing_happens() {
    let mut p = preview_with(None, false);
    std::fs::write(p.tmp.path().join("b.md"), "# B\n").unwrap();
    p.open_marked("a.md", "see [-[b]>](b.md)\n");
    stub_server(&mut p);
    assert_eq!(follow(&mut p), "f");
    assert_eq!(state(&p.ed), "see [-[b]>](b.md)\n");
}

#[test]
fn a_web_address_opens_in_the_system_app() {
    let mut p = linking("[-[s]>ite](https://example.com/a?q=1&b=2)\n", &[]);
    assert_eq!(opened(&mut p), "https://example.com/a?q=1&b=2");
}

#[test]
fn an_autolink_opens_without_its_angle_brackets() {
    let mut p = linking("<-[h]>ttps://example.com/a>\n", &[]);
    assert_eq!(opened(&mut p), "https://example.com/a");
}

#[test]
fn an_image_opens_its_file_in_the_system_app() {
    let mut p = linking("![-[l]>ogo](pic.png)\n", &[("pic.png", "")]);
    let opened = opened(&mut p);
    assert!(opened.ends_with("/pic.png"), "{opened}");
}

#[test]
fn a_mail_link_opens_in_the_system_app() {
    let mut p = linking("[-[m]>ail](mailto:a@b.c)\n", &[]);
    assert_eq!(opened(&mut p), "mailto:a@b.c");
}

#[test]
fn a_mail_autolink_opens_in_the_system_app() {
    let mut p = linking("<-[a]>@b.c>\n", &[]);
    assert_eq!(opened(&mut p), "mailto:a@b.c");
}

#[test]
fn a_scheme_in_capitals_opens_in_the_system_app() {
    let mut p = linking("[-[s]>ite](HTTPS://example.com)\n", &[]);
    assert_eq!(opened(&mut p), "HTTPS://example.com");
}

#[test]
fn a_link_with_another_scheme_raises() {
    for (marked, scheme) in [
        ("[-[x]>](file:///tmp/x.exe)\n", "file:"),
        ("[-[x]>](ftp://example.com/a)\n", "ftp:"),
        ("<-[m]>s-msdt:x>\n", "ms-msdt:"),
    ] {
        let mut p = linking(marked, &[]);
        let raised = follow(&mut p);
        assert!(
            raised.contains("opens only http, https and mailto links")
                && raised.contains(&format!("not {scheme}")),
            "{marked:?}: {raised}"
        );
    }
}

#[test]
fn an_image_link_to_another_file_opens_it() {
    let mut p = linking(
        "![-[s]>etup](setup.command)\n",
        &[("setup.command", "echo hi\n")],
    );
    assert_eq!(follow(&mut p), "t");
    assert_eq!(state(&p.ed), "-[e]>cho hi\n");
}

#[test]
fn g_d_follows_a_link_in_a_markdown_buffer() {
    let mut p = linking("see [-[b]>](b.md)\n", &[("b.md", "# B\n")]);
    bind_global_probe(&mut p, &[("normal", "g d")]);
    p.ed.feed_keys([key('g'), key('d')]);
    assert_eq!(state(&p.ed), "-[#]> B\n");
}
