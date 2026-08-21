;;; core:git-diff — diff.scm
;;;
;;; Ref-content fetch (`spawn-async!` + `git show`) and the native line-diff
;;; call (`diff-buffer-lines`), debounced per buffer. Word diff
;;; (`diff-words`) is not called here — it's called from `render.scm`,
;;; where the records it feeds are built, rather than threading a parallel
;;; word-diff structure between the two files.

(require "state.scm")
(require "render.scm")

(provide git-diff/schedule-refresh! git-diff/force-refresh! git-diff/cancel-fetch!)

;;; `hunks` -> apply to state + render, but only when they actually differ
;;; from what's already rendered (nvim's `_hunks_equal`) — skips a no-op
;;; setter call, e.g. when a debounced refresh's diff comes back identical
;;; to the last one. This equality check is exactly why state's `hunks`
;;; must always equal what's currently painted — see `force-refresh!`'s
;;; comment. Re-reads the entry rather than trusting a caller-held one:
;;; both call sites below (a live local diff and a `spawn-async!` callback)
;;; can run after the buffer closed out from under them. Each rendering is
;;; gated on its own flag, independently — a user can have signs on and
;;; inline off, or the reverse, and a refresh triggered for either must not
;;; touch the other.
(define (git-diff/apply-hunks! bid hunks)
  (let ([entry (git-diff/buffer-entry bid)])
    (when (and entry (not (equal? (hash-ref entry "hunks") hunks)))
      (git-diff/entry-set! bid "hunks" hunks)
      (when (hash-ref entry "signs?") (git-diff/render-for! "signs?" bid hunks))
      (when (hash-ref entry "inline?") (git-diff/render-for! "inline?" bid hunks)))))

;;; `spawn-async!` callback for the `git show` below. `stdout` is only
;;; trusted on `exit-code 0`. Otherwise, three severities: `-1` (contract at
;;; hume-scripting/src/builtins/process.rs:18-27) means `git` couldn't even
;;; run — an environment fault, not a fact about this file — and is logged
;;; `'error` so it reaches the status line and the unseen-count indicator.
;;; A real nonzero exit against a buffer's runtime ref override (`entry`'s
;;; "ref" field, set by an explicit-ref toggle invocation) is
;;; logged `'warn` — the failure is a direct answer to a command the user
;;; just typed, so it belongs on the status line, unlike the config-default
;;; case below. Every other nonzero exit (untracked file, brand-new file,
;;; buffer outside any repo, bad `ref` config — indistinguishable without
;;; parsing `stderr` further, so all share this branch) is expected and
;;; logged `'trace`: visible in `:messages` for diagnosis, but silent
;;; otherwise. Either way `ref-text` becomes `'unavailable`, not `#f` — `#f`
;;; means "not fetched yet" and `refresh!` would re-spawn a `git show` that
;;; fails identically on every debounced keystroke; `'unavailable` is a
;;; sticky negative cache, cleared by `on-buffer-save` or `force-refresh!`.
(define (git-diff/handle-fetch-result! bid stdout stderr exit-code)
  (git-diff/entry-set! bid "job" #f)
  (if (= exit-code 0)
      (begin
        (git-diff/entry-set! bid "ref-text" stdout)
        (git-diff/apply-hunks! bid (diff-buffer-lines bid stdout)))
      (begin
        (let ([entry (git-diff/buffer-entry bid)])
          (log! (cond [(= exit-code -1) 'error]
                      [(and entry (hash-ref entry "ref")) 'warn]
                      [else 'trace])
                (string-append "git-diff: `git show` failed: " (trim stderr))))
        (git-diff/entry-set! bid "ref-text" 'unavailable)
        (git-diff/apply-hunks! bid '()))))

;;; `git show <ref>:./<name>`, cwd = `path`'s directory. `./`-prefixing the
;;; name resolves it relative to cwd, so no `git rev-parse --show-toplevel`
;;; call (and no cached repo root in state) is needed to locate the blob.
(define (git-diff/fetch-ref! bid path ref)
  (git-diff/cancel-fetch! bid)
  (let ([job (spawn-async! "git"
                           (list "show" (string-append ref ":./" (file-name path)))
                           (parent-name path)
                           (lambda (stdout stderr exit-code)
                             (git-diff/handle-fetch-result! bid stdout stderr exit-code)))])
    (git-diff/entry-set! bid "job" job)))

;;; Immediate (non-debounced) refresh — `schedule-refresh!` below is the
;;; debounced entry point every hook actually calls. Re-reads `bid`'s live
;;; entry/path rather than trusting stale arguments, same reasoning as
;;; `inlay.scm`'s `refresh-hints`: a debounced fire happens later, after
;;; state may have moved.
(define (git-diff/refresh! bid ref)
  (let ([entry (git-diff/buffer-entry bid)])
    ;; Either rendering alone still needs a fetch — signs and inline are
    ;; two independent consumers of the same hunk store.
    (when (and entry (or (hash-ref entry "signs?") (hash-ref entry "inline?")))
      (let ([path (buffer-path bid)])
        ;; A pathless buffer (`:messages`, `:ls`) still fires
        ;; `on-text-changed` — nothing to diff against, so skip it rather
        ;; than erroring.
        (when path
          (let ([ref-text (hash-ref entry "ref-text")])
            (if (string? ref-text)
                ;; Cached: a local diff, no git process on the keystroke path.
                (git-diff/apply-hunks! bid (diff-buffer-lines bid ref-text))
                ;; `#f` = never fetched, or invalidated by a save → fetch.
                ;; `'unavailable` = the last fetch failed → don't re-spawn a
                ;; `git show` that will fail identically on every debounced
                ;; keystroke; a save or `force-refresh!` clears it.
                (unless ref-text
                  (git-diff/fetch-ref! bid path ref)))))))))

;;; Forces a fetch even through a sticky `'unavailable` cache — used by both
;;; toggle commands so turning signs/inline back on always re-tries rather
;;; than staying silent because a previous fetch failed. Deliberately does
;;; *not* touch `hunks`: it must keep tracking whatever is actually
;;; painted, so that when the fetch reproduces a result different from the
;;; last one (e.g. `'()`, the buffer now matches the ref exactly)
;;; `apply-hunks!`'s equality check sees a real change and re-renders. The
;;; caller (`plugin.scm`'s toggle command) is what paints an instant
;;; preview from the already-stored `hunks` before this runs.
(define (git-diff/force-refresh! bid ref)
  (let ([entry (git-diff/buffer-entry bid)])
    (when entry
      (unless (string? (hash-ref entry "ref-text"))
        (git-diff/entry-set! bid "ref-text" #f))
      (git-diff/refresh! bid ref))))

;;; Cancels any in-flight fetch for `bid` without firing its callback. Used
;;; both here (a newer refresh supersedes an older fetch) and from
;;; `on-buffer-close` (state.scm has no async awareness of its own).
(define (git-diff/cancel-fetch! bid)
  (let ([entry (git-diff/buffer-entry bid)])
    (when entry
      (let ([job (hash-ref entry "job")])
        (when job (cancel-async! job)))
      (git-diff/entry-set! bid "job" #f))))

;;; `debounce-by`, not `debounce`: keyed per `bid`, so one buffer's edits
;;; never cancel another's pending refresh (`inlay.scm`'s same rationale).
;;; 150ms rather than `inlay.scm`'s 200ms LSP round-trip budget — once the
;;; ref is cached, a refresh is a local diff, not a network request.
(define git-diff/schedule-refresh! (debounce-by 150 git-diff/refresh!))
