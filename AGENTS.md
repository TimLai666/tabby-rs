# Follow-ups

## P2 — WinSCP temporary key files are only cleaned up by `TempPath` drop

- `src-tauri/src/winscp/key.rs:24` — `ConvertedKey::path` is a `tempfile::TempPath`;
  the file is deleted only when the handle is dropped.
- `src-tauri/src/winscp/launch.rs:57-62` — `launch_with` holds both converted keys for
  the duration of the call, and `run_process` (`launch.rs:108-117`) uses `Command::status`,
  which blocks until the spawned WinSCP process exits. Cleanup therefore depends on that
  wait returning.
- `src-tauri/src/commands/app.rs:383-392` — `app_quit` calls `app.exit(0)` immediately.
  Exiting Tabby RS before WinSCP closes kills the pending `spawn_blocking` task, so the
  `TempPath` never drops and the converted keys stay in the temp directory.
- Before WinSCP acceptance is signed off: implement and verify cleanup on application exit
  and on crash recovery, without deleting key files that another process is still using.
- `src-tauri/src/winscp/prepare.rs` also stages a source key while waiting for
  `WinSCP.com /keygen`; include that conversion phase in the cleanup verification.
- Also open: the existing-instance hand-off is unverified. `launch.rs:4-9` notes that a
  running WinSCP may take the session over, so the only process lifetime relied on is the one
  this module waits for. That path needs Windows evidence.

## Pending — WinSCP reuse of SSH keys and prompted passphrases

- `tabby-tauri/src/ssh/session.ts` can authenticate with keys from
  `ssh.listPrivateKeys` when the profile has no configured keys.
- `tabby-tauri/src/services/winscp.service.ts` converts only configured keys, so
  such sessions open WinSCP without a key. Verify this flow against the fixed
  upstream baseline before claiming WinSCP parity.
- Private-key prompts now offer Remember and save the passphrase after the native
  decoder unlocks the key. WinSCP conversion reads saved passphrases; verify reuse
  on Windows and compare unchecked Remember behavior with upstream before acceptance.

## Pending — full-repo lint and Rust formatting

- `yarn lint --format unix` fails with 1 problem:
  `tabby-local/src/session.ts:67:24: Unnecessary parentheses around expression. [Error/@typescript-eslint/no-extra-parens]`
- `HEAD` and the working tree match for this file (`git diff HEAD -- tabby-local/src/session.ts`
  is empty), so the failure is pre-existing.
- Unrelated to the current WinSCP/SSH changes; still pending.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` also fails on existing
  Rust formatting, including `commands/app.rs`, `commands/ssh.rs`, and SSH modules.
  Comparing the changed SSH module with HEAD shows no new formatting differences;
  the X11 cookie change removes one old difference. Keep this gate pending.

## Pending — SSH forwarding desktop and Windows acceptance

- Renderer and native request tests cover selecting a forwarding socket independently
  of password, private-key, and keyboard-interactive login.
- `src-tauri/src/ssh/engine_integration.rs` verifies unsolicited agent and X11 channels
  against a loopback SSH server, with password/private-key login and forwarding on/off.
  The production forwarding helpers reject disabled channels before connecting to the
  local socket. macOS desktop checks through two SSH jump hosts cover all four target
  agent/X11 setting combinations, an exact synthetic agent identities exchange, and a
  16 KiB X11 TCP round trip. Enabled jump-profile flags do not enable a disabled target.
  Closing a tab closes its target and both hops without disrupting another tab's forwarding.
  These checks use synthetic endpoints, not agent signing or a graphical multi-hop X server.
  Forwarding controls, real-agent signing, graphical multi-hop X11, and other platforms
  still need acceptance.
- Shell setup sends X11 before agent forwarding, matching `14e2d60`. Forwarding and shell
  requests do not ask for replies or wait for approval. Rejected or unanswered requests
  leave the channel available for subsequent data and input. Send errors still tear down
  the connection. Real two-hop macOS desktop checks verify usable terminal input after
  X11 or agent rejection, matching the exact upstream shell method and locked russh binding.
  The npm archive matches the upstream lock integrity; installed native and JS bindings
  match the archive. `shell_start_tests.rs` exercises the helper called by production
  `SshManager::connect_inner` over real loopback SSH channels; `RusshConnection::open_shell`
  also uses it. Its 42 cases cover all four forwarding flag combinations, server policies
  that accept/reject/ignore forwarding and shell requests, ordered output, terminal input,
  and subsequent EOF/close. With `want_reply=false`, the fixture suppresses request replies
  as required by SSH; unsolicited replies from a noncompliant server are not covered.
  The tests fail when setup waits for shell approval. This does not replace desktop or
  cross-platform acceptance.
  macOS desktop checks cover accepted, rejected, and unanswered shell requests: all show
  the fixture's diagnostic and round-trip terminal input, matching the original methods.
  The rejection check previously displayed a shell-channel error instead. These synthetic
  exchanges verify the channel, not execution of a remote shell after rejection.
- Shell lifecycle follows the original shell method: EOF or transport disconnection ends
  the shell; standalone CLOSE, exit status, and exit signal do not. The native event loop
  drains queued output before observing transport completion, retains tab controls after
  channel CLOSE, and registers sessions before spawning their reader to avoid stale entries
  when EOF arrives immediately. Real loopback tests cover these events, buffered output,
  a second channel after CLOSE, tab controls, and disconnection with or without prior CLOSE.
  Queued closure is processed before controls. CLOSE cancels shell input and rejects
  resizes without ending the transport or blocking tab closure.
  The exact upstream methods with the locked binding verify seven reference scenarios;
  these are method-level probes, not original desktop UI acceptance.
  macOS desktop checks verify EOF termination, continued input after status/signal,
  retained output and reconnect prompts on transport disconnect with or without prior
  CLOSE, and closing a CLOSE-only tab without disrupting another SSH tab. SFTP and
  forwarding teardown, transport backpressure, and other platforms still need acceptance.
- PTY setup shares `request_shell_pty` between the manager and engine. It does not request
  approval or wait for a reply, so rejecting or ignoring PTY cannot prevent later shell
  requests. The exact `14e2d60` shell method with the locked binding also proceeds under
  accept/reject/silent policies and round-trips input. Native real-SSH tests exercise the
  engine caller with all three policies, with and without an environment variable, and
  verify terminal parameters, diagnostic order, and input. The production manager path
  has macOS desktop evidence for all three policies without environment variables,
  including visible diagnostics, keyboard input, and tab closure. These fixtures echo
  data; they do not demonstrate execution of a remote shell after PTY rejection.
  Environment confirmation and send-error cleanup remain unchanged. Original wire-level
  `want_reply` and noncompliant unsolicited PTY replies have not been verified.
  Initial shell requests now use `xterm-256color` at 80x24 with zero pixel dimensions,
  matching the exact original method and locked binding. The renderer previously sent
  80x30. Both tab components resize after shell startup. macOS desktop and real SSH
  evidence verify the initial 80x24 request, then 132x40, zoom to 235x55, and restoration
  to 132x40, with input after resizing and clean tab closure. The native request API
  still preserves supplied terminal parameters; the fixed default belongs to the renderer.
  Do not claim full SSH parity until environment behavior, teardown,
  and supported-platform acceptance have been compared and verified.
- Shell input uses a separate bounded worker so waiting for the peer's channel window
  cannot block output or session controls. Keep ordered `ChannelWriteHalf` writes:
  `Handle::data` can leave pending data that prevents russh 0.54.4 from acknowledging CLOSE.
  Cancel the worker on remote CLOSE, EOF, disconnection, local close, or owner drop.
  Await worker termination before sending local CLOSE so no write can race behind it.
  Loopback tests verify a stalled one-byte window with a full 32-entry input queue,
  output delivery, all four close paths, release of pending callers, the peer's CLOSE
  acknowledgement, and exact binary data/order across window adjustments. The locked
  upstream binding retains pending write promises while delivering output and lifecycle
  events; its disconnect call completes for live transports. Native cancellation releases
  write callers with Closed; the renderer only logs write errors and does not close the tab
  or display a service message for them. macOS desktop checks with a one-byte peer window
  verify visible output during stalled input, local tab closure, retained output after
  remote CLOSE, EOF/disconnect reconnect prompts, and successful reconnection. The test
  server observes each transport closing. Two 8 KiB paste attempts reached the server
  and displayed its one-byte-received diagnostic, but the UI tool timed out observing
  clipboard reads; exact desktop paste byte counts remain unverified. Native tests
  verify the full 8 KiB write and blocked queue explicitly.
  Sustained input against a stalled peer and transport-level backpressure need separate
  resource and responsiveness checks before accepting full SSH parity.
- Security-sensitive parity decision pending: the exact upstream incoming-channel handlers
  with the locked russh binding forward unsolicited agent and X11 channels even when both
  target options are false. A real two-hop fixture confirms both local synthetic endpoints
  are reached. Native handlers close disabled channels before opening local endpoints.
  Preserve that protection until the user explicitly decides whether this is an accepted
  safety exception; do not mark this behavior as full parity or an approved exception yet.
- `src-tauri/src/ssh/engine.rs` now shares `connect_agent` with jump authentication and
  forwarding. On Windows, `None` selects Pageant and an explicit path selects a named pipe.
  Verify Pageant with `SSH_AUTH_SOCK` set and verify named-pipe connections on Windows.
- Local `cargo check --target x86_64-pc-windows-msvc` stops in the `ring` dependency
  because the host lacks Windows C headers (`assert.h`). It does not establish that the
  Windows application code compiles; Windows compilation and runtime checks are pending.

## Pending — X11 transport parity and desktop acceptance

- X11 TCP forwarding now shares the same implementation on Windows and Unix, with
  the existing consent check before connecting to the local display. Windows defaults
  to `localhost:6000`; Unix defaults to `/tmp/.X11-unix/X0`. Unix sockets remain available
  only on Unix. The portable real-SSH fixture checks TCP traffic and disabled-channel
  rejection with password and private-key login; it runs in the ordinary Rust test suite.
  macOS tests pass. Windows compilation and runtime acceptance remain pending: the local
  cross-check stops in native dependencies because Windows headers, including `windows.h`,
  are unavailable. It does not establish that the Windows application code compiles.
- X11 TCP display numbers below 100 now add 6000; values of 100 or greater use the
  supplied port, matching `14e2d60:tabby-ssh/src/session/x11.ts`. Named Unix displays
  such as `unix:0.0` resolve to `/tmp/.X11-unix/X0`. Tests cover the 99/100 boundary,
  port 65535, socket paths, invalid numbers, and a real TCP exchange on an ephemeral
  port. The existing real SSH forwarding fixture also passes with forwarding enabled
  and disabled under password and private-key login. macOS desktop TCP forwarding displays
  xclock on a disposable Xvfb server that allows connections without cookie validation.
  Windows TCP forwarding acceptance and graphical multi-hop desktop acceptance remain pending.
- DISPLAY parsing follows the fixed upstream regex, including fallback for screenless
  values such as `host:12`, the wildcard separator, and the greedy `host:100` result
  (`host:6001`). Hostless `:N` and `:N.screen` also fall back to the platform default.
  Keep these behaviors when editing the parser. A checked-in fixture
  contains 332 results generated by `14e2d60:tabby-ssh/src/session/x11.ts` for Unix and
  Windows, covering absolute paths, Unicode, invalid syntax, and large numeric values.
  TCP port range validation happens after resolution; Unix display numbers are not
  limited to 16 bits. Configured display, environment, and default selection share one helper.
  Invalid UTF-8 in a Unix DISPLAY environment value is decoded with replacement characters
  like Node, rather than treated as unset and silently redirected to the default display.
  A macOS desktop check confirms the replacement-character path is retained in the
  connection diagnostic instead of connecting to `/tmp/.X11-unix/X0`.
  macOS desktop verification includes a real SSH X11 channel carrying a TCP round trip
  with a screenless display (`127.0.0.1:5822400` resolves to port 58224), then a refused
  connection to that same endpoint after the local listener exits. The SSH terminal remains usable.
  Failed local-display connections now emit upstream-style terminal diagnostics through
  `ssh:message`, retaining Tabby RS branding, before closing the X11 channel. Tests cover
  the resolved endpoint, Windows X-server guidance, terminal-control escaping, session
  isolation, teardown, and actual channel closure on an unavailable Unix socket.
  macOS desktop verification covers the SSH and X badges, upstream line spacing,
  missing-socket diagnostics, continued SSH input after failure, and silent rejection
  when X11 is disabled. Windows guidance and other platforms still need desktop acceptance.
- Verify the SSH close confirmation in the real desktop UI and validate agent/X11
  behavior on the supported operating systems before accepting complete parity.
- Each X11-enabled shell requests a fresh 16-byte random cookie, matching
  `14e2d60:tabby-ssh/src/session/ssh.ts`; it no longer queries local xauth entries.
  Isolated child-process tests verify that an available xauth executable is not called.
  macOS desktop SSH fixtures verify distinct cookies across two shells, the upstream
  protocol/flags/screen number, and no request when X11 is disabled. The exact upstream
  shell method also passes the fixture. Both implementations relay X11 data unchanged.
  The fixed upstream shell/relay methods and the macOS Tauri app both display xclock
  through real SSH channels on Xvfb with access control disabled. With a fixed-cookie
  Xvfb server, both reject the random cookie with `Invalid MIT-MAGIC-COOKIE-1 key`, while
  a direct connection using the server's cookie succeeds. The native SSH shell stays usable.
  This matches the fixed upstream behavior, but changes the prior native local-cookie path;
  do not claim support for cookie-protected displays from the successful xclock check.
  XQuartz, other X servers, other client platforms, and graphical multi-hop X11 still need acceptance.

## Pending — SSH password prompts and automatic authentication parity

- Configured profile passwords are now tried in automatic and explicit password modes.
  Native tests verify rejection falls through to stored passwords and Debug redacts the
  supplied value. Repeated configured/stored passwords are tried only once. Renderer
  tests cover ordering and separate jump-host passwords; manual macOS desktop loopback
  checks verify both modes authenticate with the configured password.
  Keyboard-interactive now tries a configured-password candidate before the stored
  or bare candidate. Only masked password fields receive prefill. Native code snapshots
  saved passwords after resolving the account, preserving the retry values if the panel
  saves a different password. Tests cover Keychain/Vault target isolation, equal/different
  configured and stored values, empty credentials, wire compatibility, and Debug redaction.
  macOS desktop direct and single-hop target connections verify a rejected configured
  password followed by a successful manual second candidate, with blank verification-code
  fields. Saved-password retry still needs desktop acceptance: this run's synthetic
  Keychain credential triggered SecurityAgent access approval, which desktop tools cannot
  operate. The test credential was removed and its absence verified.
- Automatic authentication now probes none and follows the server's non-empty allowed
  method lists. Unused candidates remain available if a later response enables them.
  Direct and jump connections share the same transport authentication adapter. Real SSH
  loopback tests cover password-only servers, method changes, and successful none auth.
  The password-retry fixture explicitly advertises password after rejection; its previous
  default russh rejection removed that method despite expecting another password attempt.
  Empty method lists preserve the last advertised list, but russh 0.54.4 closes the
  connection on an empty list; compare that transport behavior with upstream before
  accepting the empty-list edge case as complete parity.
- Keyboard-interactive challenges with no prompts now receive an empty response
  automatically, matching `14e2d60:tabby-ssh/src/session/ssh.ts`. Unit tests cover
  repeated empty rounds, rejection/fallback, non-empty prompts, and transport errors.
  Loopback integration tests cover acceptance and rejection after two empty rounds.
  Direct and single-hop macOS desktop connections verify two automatic rounds
  followed by a two-field challenge and successful login.
- Tauri keyboard-interactive authentication now uses the upstream inline panel with
  unchanged template and styles. Native metadata supplies the actual host, port, and
  resolved account for stored-password lookup and saving. Renderer tests cover field
  navigation, consent, storage errors, duplicate events, cancellation, stale sessions,
  and initial connection failure. An independent review's failure-cleanup finding is
  fixed and regression-tested. macOS desktop checks verify Enter, previous/next/finish,
  masked and echoed fields, Keychain saving, restart prefill, and reuse through a jump
  host. The synthetic Keychain entry and test servers were cleaned up afterward.
  Vault, keyboard-interactive authentication on the jump host itself, other supported
  platforms, and saved-password retry behavior still need acceptance. Preserve
  upstream's save-on-consent behavior when aligning rejected-password deletion below.
- Empty usernames now prompt after host-key verification; `$VAR` expands before
  authentication and saved-password lookup. Native tests cover cancellation, invalid
  responses, environment fallback, and Keychain/Vault identity isolation. Manual macOS
  desktop checks verify visible username input, Enter, Esc without an auth attempt,
  environment expansion, and Keychain password reuse under the resolved username.
  A single-hop desktop check verifies distinct hop/target usernames and target Keychain
  reuse. A four-hop macOS desktop check verifies each hop's username prompt and configured
  password, second-hop cancellation, and cleanup without disturbing another connection.
  Vault desktop behavior and other platforms still need acceptance.
- Private-key passphrases now use the shared masked prompt with Remember and retry.
  Renderer tests cover cancellation, consent, retries, stale events, and storage errors.
  A manual macOS desktop loopback check verifies wrong-then-correct passphrase entry and
  successful key authentication. Reading the isolated synthetic Keychain entry verifies
  the remembered value. A second manual macOS desktop check verifies reuse after restarting
  the app and Esc cancellation followed by successful configured-password fallback.
  Vault, jump-host flows, and other supported platforms still need acceptance.
- Missing/unreadable and malformed private keys now fall through to later methods.
  Unit and loopback tests cover this behavior, including successful password fallback.
  Desktop jump-host acceptance still needs verification.
- Automatic and explicit password modes now use the shared `PromptModalComponent`.
  Browser checks cover masked input, Remember, Enter, OK, and Esc with simulated native
  events. A manual macOS desktop loopback SSH check verifies actual prompt delivery, Enter
  authentication, terminal output, and Esc cancellation without a password attempt.
  A manual macOS desktop check also verifies Remember creates a Keychain credential that
  a subsequent resolved-username connection can reuse. Password prompts on jump hosts,
  Vault persistence, and other platforms still need end-to-end acceptance.
- Independent review covers private-key loading/passphrase prompts, configured-password
  reuse, and resolved-username prompts/lookups. Broader agent/stored-password and prompted-
  password review remains pending. The native passphrase retry loop has manual desktop
  evidence but no automated regression test. Complete these checks before accepting
  SSH authentication parity.

## Pending — SSH credential and username follow-through

- Jump chains now connect outermost first, without the former three-hop limit, and use
  effective profile settings including group defaults and built-in profiles. Renderer
  tests cover 0/1/2/3/4/8 hops, credential association, cycles, missing profiles,
  and profile resolution. Native validation checks every hop. A four-hop macOS desktop
  connection using inherited group settings succeeds; closing its tab leaves no open
  connections on the five loopback servers. Built-in profile desktop behavior, mixed
  authentication methods, forwarding, and other platforms still need acceptance.

- `tabby-tauri/src/services/winscp.service.ts` uses the original profile username for
  jump options, matching `14e2d60:tabby-ssh/src/services/ssh.service.ts`. Verify prompted
  and `$VAR` accounts on Windows against that baseline before classifying a parity gap.
- `tabby-tauri/src/services/passwordStorage.service.ts` omits the port from the Keychain
  service when the profile has no port, while native SSH lookup defaults to port 22.
  Verify legacy no-port profiles and preserve existing credentials when aligning the keys.
- Final SSH authentication-candidate exhaustion now deletes the saved password for the
  failing host, port, and resolved account. The connect error preserves code/details and
  carries that identity only on exhaustion, avoiding event/listener teardown races.
  Tests cover ordinary propagated errors, identity isolation, Keychain/Vault selectors,
  private-key preservation, unavailable storage, and non-blocking deletion failures.
  macOS desktop direct and first-hop rejection checks verify that Save password creates
  a Keychain item, one rejection preserves it, and final rejection removes only that item.
  A separate destination credential survives hop rejection. Synthetic items and servers
  were cleaned up. Vault persistence and other platforms still need desktop acceptance.
- Password lookup must distinguish a missing item from an unreadable store. If a candidate's
  saved-password lookup fails, exhaustion currently omits the deletion target and allows
  other authentication candidates to run. This prevents deleting an untried saved password;
  it does not complete upstream parity. Upstream loads passwords after resolving the account
  and before authentication. Align Vault unlocking and candidate preparation, including
  explicit key-only modes, before accepting this flow.
- Password prompt dismissal now skips that candidate, matching upstream, and can delete
  the stored password on final exhaustion. An explicit abort response distinguishes prompt
  setup failure and session teardown from dismissal; abort closes the native waiter without
  carrying a password-deletion target. Tests cover missing/false/true abort fields, duplicate
  replies, empty-password submission, modal failure, and teardown during credential loading.
  macOS desktop direct and first-hop checks verify saved-password rejection followed by Esc,
  no extra password attempt, clean transport closure, and deletion of only the failed account's
  Keychain item. The destination credential survives hop cancellation; synthetic items and the
  test server were cleaned up. Vault and other platforms still need desktop acceptance.
  Keyboard-interactive cancellation still needs exact baseline comparison and acceptance.
  Closing a pending SSH tab now cancels native setup and aborts its registered prompts.
  A registration acknowledgement repeats cancellation if tab closure preceded native setup.
  Each direct/jump transport observes cancellation during reads and writes, including russh's
  background key-exchange task; failed setup also signals cancellation. Waiter guards remove
  pending host-key/auth replies. Renderer tests cover closure during listener installation,
  request preparation, registration, prompts, and the successful-connect race. Native tests
  verify cancellation isolation, waiter removal, transport I/O, and socket EOF during stalled
  key exchange. macOS desktop checks verify tab closure before the server identification and
  during key exchange, with socket closure while another authenticated SSH session remains
  connected. Normal password login and closing the established session also pass. Modal tab closure,
  multi-hop cancellation, Vault behavior, and other platforms still need desktop acceptance.
  Agent errors continue to fall through as in upstream; a disconnect swallowed by that path
  can reach exhaustion, unlike an error propagated out of authentication.
  Upstream also catches failures inside prompted-password and private-key attempts, whereas
  some corresponding native errors propagate. Compare disconnects in those methods before
  accepting their fallback and saved-password deletion behavior.
- Upstream's keyboard-interactive panel saves on consent without awaiting storage. Verify
  pending saves cannot restore a rejected password after deletion, especially during Vault
  unlock or expiration. This ordering remains unverified, not an accepted parity result.
- Direct authentication shares a 120-second deadline across username, password, and key
  prompts; jump authentication does not share that deadline. Verify slow interactive
  login against upstream before accepting timeout parity.

## Pending — Telnet and serial login scripts and desktop connection startup

- Tauri Telnet and serial profiles now expose the shared Login scripts editor and
  configure the existing session processor. Successful connection runs unconditional
  scripts before draining early output, matching `14e2d60`. The new
  `scripts/test-connector-login-scripts.mjs` runs real session and middleware code
  with a simulated native bridge. It verifies ordered initial/prompt scripts,
  early output, once-only execution, preserved profile data, failed/cancelled
  connections, and legacy profiles without scripts for both protocols.
- `tabby-tauri/src/index.ts` imports the core and terminal Angular modules so shared
  settings components render. macOS desktop checks show the login-script rows,
  input/stream controls, and serial reconnect toggle. Search and terminal-toolbar
  interactions still need desktop acceptance.
- Desktop connection verification is blocked by the existing `readline: false`
  fallback in `app/webpack.config.tauri.mjs:99`. Telnet startup throws
  `readline.clearLine is not a function` in cli-spinner before native connection.
  Serial uses the same spinner, and stream processing also requires readline.
  The Node session tests use real Node readline and cannot detect this bundle gap.
  Fix browser readline support and verify actual Telnet/serial script exchanges
  before accepting connector parity. The macOS fixture has no successful exchange.
- Script deletion persistence, serial automatic-reconnect script behavior, original
  serial slow-send support, and other platforms remain unaccepted. Do not infer
  full connector parity from the session tests or rendered settings controls.
