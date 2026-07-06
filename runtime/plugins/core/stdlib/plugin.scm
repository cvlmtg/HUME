;;; core:stdlib — library of helper functions for plugin authors.
;;;
;;; Scripting layers:
;;;   - BOOTSTRAP (hume-scripting/src/builtins/mod.rs) — core dispatch
;;;     primitives (call!, load-plugin, define-command!); always available.
;;;   - prelude (runtime/scheme/prelude.scm) — convenience macros for
;;;     init.scm; loaded at startup when the runtime dir exists.
;;;   - core:stdlib (this plugin) — functions useful to plugin authors;
;;;     loaded explicitly via (load-plugin "core:stdlib") in init.scm,
;;;     before any plugin that depends on it.
;;;
;;; Currently empty — functions land here as they are decided.
