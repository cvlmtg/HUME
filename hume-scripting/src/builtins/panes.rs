use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

type SteelResult = Result<SteelVal, SteelErr>;

fn pane_stub(builtin_name: &str) -> SteelResult {
    steel::stop!(Generic => "{}: pane operations require :split, deferred to M9+", builtin_name)
}

/// `(open-pane! bid)` — reserved; pane split operations land in M9+.
pub(crate) fn open_pane(_bid: SteelVal) -> SteelResult {
    pane_stub("open-pane!")
}

/// `(close-pane! pid)` — reserved; pane split operations land in M9+.
pub(crate) fn close_pane(_pid: SteelVal) -> SteelResult {
    pane_stub("close-pane!")
}

/// `(focus-pane! pid)` — reserved; pane split operations land in M9+.
pub(crate) fn focus_pane(_pid: SteelVal) -> SteelResult {
    pane_stub("focus-pane!")
}

/// `(pane-buffer pid)` — reserved; pane split operations land in M9+.
pub(crate) fn pane_buffer(_pid: SteelVal) -> SteelResult {
    pane_stub("pane-buffer")
}

/// `(pane-set-buffer! pid bid)` — reserved; pane split operations land in M9+.
pub(crate) fn pane_set_buffer(_pid: SteelVal, _bid: SteelVal) -> SteelResult {
    pane_stub("pane-set-buffer!")
}

#[cfg(test)]
mod tests {
    use crate::ScriptingHost;
    use crate::null_host::NullHost;

    fn host() -> ScriptingHost {
        ScriptingHost::new()
    }

    #[test]
    fn open_pane_returns_deferred_error() {
        let mut h = host();
        let mut mock = NullHost;
        let err = h.eval_source("(open-pane! #f)", &mut mock).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn close_pane_returns_deferred_error() {
        let mut h = host();
        let mut mock = NullHost;
        let err = h.eval_source("(close-pane! #f)", &mut mock).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn focus_pane_returns_deferred_error() {
        let mut h = host();
        let mut mock = NullHost;
        let err = h.eval_source("(focus-pane! #f)", &mut mock).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn pane_buffer_returns_deferred_error() {
        let mut h = host();
        let mut mock = NullHost;
        let err = h.eval_source("(pane-buffer #f)", &mut mock).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn pane_set_buffer_returns_deferred_error() {
        let mut h = host();
        let mut mock = NullHost;
        let err = h
            .eval_source("(pane-set-buffer! #f #f)", &mut mock)
            .unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }
}
