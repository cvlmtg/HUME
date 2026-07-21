// Both tests spawn `/bin/cat` as a stand-in server, so the whole module is
// unix-only.
#[cfg(unix)]
mod unix;
