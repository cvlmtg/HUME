// ── mock_host self-tests ─────────────────────────────────────────────────

mod mock_host {
    use super::super::mock_host::MockHost;
    use hume_scripting::host::{CommandHost, LanguageHost};

    fn cmd_def(name: &str) -> hume_scripting::SteelCmdDef {
        hume_scripting::SteelCmdDef {
            name: name.to_string(),
            doc: String::new(),
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
        }
    }

    #[test]
    fn register_command_rejects_duplicate_name() {
        let mut mock = MockHost::new();
        mock.register_command(cmd_def("dup"))
            .expect("first registration must succeed");

        let err = mock
            .register_command(cmd_def("dup"))
            .expect_err("second registration of the same name must be rejected");
        assert!(
            err.contains("dup") && err.contains("conflicts with existing command"),
            "unexpected message: {err}"
        );
        assert_eq!(
            mock.registered_cmds.len(),
            1,
            "the rejected redefinition must not be recorded"
        );
    }

    #[test]
    fn register_command_overwrites_lazy_stub() {
        let mut mock = MockHost::new();
        let plugin = hume_scripting::attribution::PluginId::parse("core:test").unwrap();
        mock.register_lazy_command("bar", &plugin).unwrap();
        assert!(mock.lazy_command_owner("bar").is_some());

        mock.register_command(cmd_def("bar"))
            .expect("defining over a Lazy stub must succeed");

        assert!(
            mock.lazy_command_owner("bar").is_none(),
            "the Lazy stub must be cleared once the real command is defined"
        );
    }

    #[test]
    fn attach_grammar_rejects_missing_grammar_path() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let highlights = dir.path().join("highlights.scm");
        std::fs::write(&highlights, "").unwrap();

        let err = mock
            .attach_grammar(
                "rust",
                std::path::Path::new("/no/such/lib.dylib"),
                "rust_language",
                &highlights,
                None,
            )
            .expect_err("missing grammar path must be rejected");
        assert!(
            err.contains("grammar library not found"),
            "unexpected message: {err}"
        );
        assert!(
            !mock.has_grammar("rust"),
            "a failed attach must not be recorded"
        );
    }

    #[test]
    fn attach_grammar_rejects_missing_highlights_path() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let grammar = dir.path().join("lib.dylib");
        std::fs::write(&grammar, "").unwrap();

        let err = mock
            .attach_grammar(
                "rust",
                &grammar,
                "rust_language",
                std::path::Path::new("/no/such/highlights.scm"),
                None,
            )
            .expect_err("missing highlights path must be rejected");
        assert!(
            err.contains("highlights query not found"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn attach_grammar_succeeds_when_both_paths_exist() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let grammar = dir.path().join("lib.dylib");
        let highlights = dir.path().join("highlights.scm");
        std::fs::write(&grammar, "").unwrap();
        std::fs::write(&highlights, "").unwrap();

        mock.attach_grammar("rust", &grammar, "rust_language", &highlights, None)
            .expect("both paths existing must succeed");
        assert!(mock.has_grammar("rust"));
    }
}
