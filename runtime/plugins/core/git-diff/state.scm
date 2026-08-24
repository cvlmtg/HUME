;;; core:git-diff — state.scm (see README.md "File layout"). Per-buffer
;;; state, one `(box (hash))` keyed by buffer id — the same per-key
;;; mutable-table idiom `debounce-by` uses
;;; (hume-scripting/src/builtins/bootstrap.scm). Steel's `hash` is
;;; persistent, so mutation is swap-the-box, not in-place update.
;;;
;;; Each entry holds the two independent enable flags (`signs?`/`inline?`)
;;; plus the git-fetch/diff pipeline's working state: `ref-text` — a string
;;; (the cached `git show` blob), `#f` (needs (re-)fetching), or
;;; `'unavailable` (the last fetch failed; a sticky negative cache so a
;;; doomed fetch isn't retried every debounce fire — see diff.scm's
;;; `handle-fetch-result!`/`refresh!`/`force-refresh!`) — `hunks` (the
;;; verbatim tuples `diff-buffer-lines` last returned, and always kept in
;;; sync with what's actually painted, never a signs-derived shape — the
;;; additivity invariant: every renderer in `render.scm` stays a pure
;;; function over this one shared hunk set, so adding a new rendering is one
;;; function and one setter call, touching neither this file, the fetch
;;; pipeline, nor the lifecycle hooks in plugin.scm), `job` (the in-flight
;;; `spawn-async!` id, or `#f`, so a superseded fetch can be cancelled), and
;;; `ref` — `#f` (use the config default) or a string (a runtime override
;;; set via `:toggle-git-signs <ref>`/`:toggle-inline-diff <ref>`, shared by
;;; both renderers — see plugin.scm's `git-diff/buffer-ref`).

(provide git-diff/init-buffer! git-diff/remove-buffer!
         git-diff/buffer-entry git-diff/entry-set! git-diff/ensure-entry!
         git-diff/toggle-flag!)

(define git-diff/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape — `init-buffer!`, `ensure-entry!`, and
;;; `toggle-flag!`'s untracked-buffer fallback all need it.
(define (git-diff/fresh-entry signs? inline?)
  (hash "signs?" signs? "inline?" inline?
        "ref-text" #f "hunks" '() "job" #f "ref" #f))

(define (git-diff/buffer-entry bid)
  (let ([table (unbox git-diff/*buffers*)])
    (and (hash-contains? table bid) (hash-ref table bid))))

(define (git-diff/init-buffer! bid signs? inline?)
  (set-box! git-diff/*buffers*
            (hash-insert (unbox git-diff/*buffers*) bid
                         (git-diff/fresh-entry signs? inline?))))

(define (git-diff/remove-buffer! bid)
  (set-box! git-diff/*buffers* (hash-remove (unbox git-diff/*buffers*) bid)))

;;; Generic field write, keyed on an existing entry. A no-op when `bid` has
;;; no tracked entry — a late `spawn-async!` callback for a buffer closed
;;; while its fetch was in flight must not resurrect state for it.
(define (git-diff/entry-set! bid key value)
  (let ([entry (git-diff/buffer-entry bid)])
    (when entry
      (set-box! git-diff/*buffers*
                (hash-insert (unbox git-diff/*buffers*) bid (hash-insert entry key value))))))

;;; Unlike `entry-set!`, resurrects a missing entry from `fresh-entry` rather
;;; than no-opping — for a write path (an explicit-ref or bare toggle
;;; invocation, see `toggle-flag!` below) that must succeed even for a
;;; buffer whose `on-buffer-open` never fired (user overrode the manifest's
;;; #:events with a #:commands-only activation list, so the plugin only
;;; loaded on the first toggle command).
(define (git-diff/ensure-entry! bid)
  (unless (git-diff/buffer-entry bid)
    (set-box! git-diff/*buffers*
              (hash-insert (unbox git-diff/*buffers*) bid (git-diff/fresh-entry #f #f)))))

;;; Flip `key` (one of "signs?"/"inline?") and return the new value. Built
;;; from `ensure-entry!` (same untracked-buffer resurrection) and
;;; `entry-set!` (the actual write) rather than its own box/hash-insert
;;; pair — the caller needing the flipped value back is just an extra
;;; `hash-ref` around the two, not a reason to duplicate them.
(define (git-diff/toggle-flag! bid key)
  (git-diff/ensure-entry! bid)
  (let ([new? (not (hash-ref (git-diff/buffer-entry bid) key))])
    (git-diff/entry-set! bid key new?)
    new?))
