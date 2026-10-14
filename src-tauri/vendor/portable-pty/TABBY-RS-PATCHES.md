# Tabby RS portable-pty patch

This directory contains the published portable-pty 0.9.0 crate under MIT.
The crate archive SHA-256 is b4a596a2b3d2752d94f51fac2d4a96737b8705dddd311a32b9af47211f08671e.
Its upstream source commit is f8921727a11b9f8b073e8c24821d72fd41283500
in https://github.com/wezterm/wezterm (pty directory).
LICENSE.md is retained. Cargo's .cargo-ok cache marker is omitted.

The source modifications are:

- src/win/psuedocon.rs omits cursor inheritance when creating a new Windows
  pseudoconsole and removes the unused flag constant. The resize quirk and
  Win32 input flags remain enabled. A new Tabby terminal has its own cursor;
  failed renderer startup must be able to close it before any cursor-position
  query is answered.
- src/win/mod.rs corrects both termination-result branches in WinChild::do_kill
  and WinChildKiller::kill. A nonzero TerminateProcess result succeeds; zero
  returns the native error. Read last_os_error only for a failed call, so stale
  errors cannot turn successful termination into an error. WinChild::kill keeps
  its existing best-effort result. Unix code and the public API are unchanged.

When updating portable-pty, compare the native CreatePseudoConsole flags and run
the bounded native discard, retained-session, panic-cleanup, and production-reader
regressions on Windows, plus the bounded ordinary-kill tests in
src-tauri/src/pty/kill_tests.rs and the existing cross-platform suites. Remove
this local Cargo patch only after equivalent fresh-terminal initialization,
startup-failure cleanup, and ordinary termination results are verified in the
replacement source. A version number alone is not acceptance evidence.
