# Follow-ups

## Pending — Windows runtime and packaging acceptance

- CI run `36341774362` on `4aa71925` fails before Windows plugin lifecycle tests
  can run. The macOS-only `title_bar_style` and `hidden_title` window builder
  calls now have a macOS platform guard. A standalone macOS compile probe
  reproduced the Windows `authenticate_agent` Send lifetime error with the same
  boxed AgentStream type. `ssh/agent_transport.rs` supplies a concrete forwarding
  wrapper, shared by Unix sockets, Windows named pipes, and Pageant, which passes
  that probe and the macOS application tests. The separate real SSH agent signing
  test also passes.
  The wrapper tests cover partial binary reads/writes, EOF, and peer closure.
  CI run `36343754549` on `0f03ba33` subsequently passes both the Windows system
  npm lifecycle and Rust host test steps, confirming compilation for that commit.
  Its later SSH source-contract step failed; the corrected contract is in
  `dc3ad915`. Windows packaging, named-pipe/Pageant runtime behavior, and changes
  after that commit still require their own acceptance. Original failure evidence:
  https://github.com/TimLai666/tabby-rs/actions/runs/36341774362
  Windows Rust test evidence:
  https://github.com/TimLai666/tabby-rs/actions/runs/36343754549/job/108688735753

## Pending — Linux desktop and packaging acceptance

- `cargo test --locked --manifest-path src-tauri/Cargo.toml` passes on Linux x64
  in an isolated Debian Bookworm container on `ubuntu-1`: 428 passed, none failed,
  and 12 ignored. The run uses Rust/Cargo 1.97.1, Node 22.22.0, and npm 10.9.4
  under a non-root account. All 1,419 source and renderer file fingerprints match
  before and after the run; the temporary container is removed afterward.
- The run includes the native SSH Close/SFTP completion, EOF reply readiness,
  output failure, and resize race regressions. The separate
  `node scripts/test-plugin-npm-lifecycle.mjs` run passes the ignored system npm
  lifecycle case with actual install, upgrade, and removal. Its 1,419 file
  fingerprints also match before and after the run, and its container is removed.
- Linux desktop rendering, interaction, packaging, and installation remain pending.
  Verify those independently before accepting Linux parity; compilation and
  native tests do not establish supported-platform runtime acceptance.

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
- With no configured keys, both the exact `14e2d60` SSH service and
  `tabby-tauri/src/services/winscp.service.ts` launch without a key even when the
  session records private-key authentication. Their actual launch methods pass
  the same empty-key check with simulated process/IPC boundaries and preserve
  the resolved target username. Verify this flow on Windows before acceptance;
  the method comparison does not verify a WinSCP process.
- Private-key prompts now offer Remember and save the passphrase after the native
  decoder unlocks the key. WinSCP conversion reads saved passphrases; verify reuse
  on Windows and compare unchecked Remember behavior with upstream before acceptance.

## Pending — WinSCP launch from ended SSH tabs on Windows

- Native SSH tabs retain the last authenticated connection before shared shell
  teardown. Both WinSCP entry points can use it after the shell ends, while a
  pending, failed, or cancelled reconnect preserves the previous connection.
  A newer authenticated connection replaces it; an old awaited initialization
  cannot overwrite that newer identity.
- `scripts/test-tauri-ssh-actions.mjs` exercises the actual native tab and session
  start/destroy methods with simulated UI/IPC boundaries and fixed `14e2d60`
  method controls. Its 11 retention cases cover both entry points, reconnect
  outcomes, early EOF during pending forwarding, duplicate destruction, and
  stale completion. The existing lifecycle fixture loads the real retention
  field/helper and preserves its original assertions. `test:ssh-reconnect`
  includes these checks and the forwarding fixture's authentication-metadata
  readiness and retention checks.
- Verify both entry points in Windows, actual WinSCP process startup, and the
  selected username and key after the shell ends or reconnects. The passing
  method checks do not establish desktop or WinSCP-process acceptance. Temporary
  key cleanup and prompted-passphrase reuse retain their separate pending checks.

## P1 — SFTP panel differs from the fixed upstream

- The native SSH tab uses the exact `14e2d60` toolbar and Ports modal templates,
  with the shared forwarding editor and native add/remove adapter. Tauri startup
  loads the exact original icon styles, including solid, regular, and brand icons.
  Tests cover diagnostics, separate row identities, saved-profile isolation, failed-stop
  retries, pending requests, and owner destruction. A rendered Angular fixture
  verifies the toolbar, help dropdown, Local/Remote/Dynamic additions, removal
  with form restoration, focused-editor Escape, and reconnect at 1100x720 and
  640x480. Its native
  IPC is simulated; actual desktop traffic and supported-platform acceptance
  remain pending.
- The fixed upstream SFTP Pug and SCSS provide breadcrumb/path editing, filtering,
  translated controls, file permissions, a context menu, and folder drag-and-drop.
  Both hosts use those templates and the shared `SFTPPanelController`, with a
  display-label hook that preserves Native display-only markers without changing
  remote names. The Native transport adapter keeps Unix-second timestamps,
  followed attributes, streaming transfers, and readable native errors compatible
  with the shared controller. The original session/type exports stay available.
  Native module registration must provide `NgxFilesizeModule`; Core imports that
  module without exporting its pipe.
- `test:sftp-panel` checks the actual shared/Native controllers, transport adapter,
  delete modal, and directory completion state. `test:ssh-reconnect` includes it.
  Rendered Angular checks at 1100x720 and 640x480 cover path editing, filtering,
  no results, navigation failure, create-directory cancellation/completion,
  context menus, copying paths, deletion progress, file-link download, binary
  upload, display-only labels, and cancellation during an awaited child deletion.
  Keep IPC/file-system fixture results separate from desktop/platform acceptance.
- Native Edit locally uses an owned temporary copy, the existing streaming
  transfers, a one-second watch grace period and save debounce, serialized
  overwrite uploads, and remote mode restoration. Session closure stops watching
  and queued saves; completed editor copies remain available to the editor.
  File watching is scoped to that copy and distinguishes in-place saves from
  replacement. Replacement triggers one final upload and ends watching, matching
  the fixed upstream; later saves require reopening Edit locally. Native SFTP
  menu extensions share the original injection token;
  legacy providers retain their concrete-panel default type.
- Native folder download uses the shared controller's size/status flow, followed
  file-link sizes, and relative child statuses. `test:sftp-panel` includes local
  editing, transport, menu-provider, and folder-progress regressions. Native tests
  exercise actual file watching and a binary save through a real SFTP protocol
  peer. Renderer/IPC fixtures do not establish an external editor process or
  supported-platform desktop behavior.
- Native file drops on a shared SFTP drop zone use the existing streaming upload
  tree, preserve mixed top-level file/folder names and empty directories, and do not
  paste local paths into the SSH terminal. Bootstrap awaits one native file-drop
  listener; panels and terminals subscribe synchronously and release their own
  subscriptions. Windows/Linux coordinates use physical pixels; the pinned Wry
  macOS implementation supplies AppKit points despite its PhysicalPosition type.
  Preparation errors cancel opened handles; a removed panel cancels late
  preparations. Upload errors cancel remaining tree leaves without cancelling
  completed files. `test:sftp-panel` includes `scripts/test-sftp-native-drop.mjs`.
  Rendered Angular checks at 1100x720 and 640x480 cover mixed file/folder drops,
  exact binary bytes, nested/empty directories, the unchanged folder chooser,
  delayed startup registration, panel reopening, and partial-error cleanup.
  Keep IPC/file-system fixture results separate from native OS drag gestures.
- Still pending: native drag-over hints, original desktop comparison, and supported
  desktop acceptance of native file drops, editing, menu extensions, and folder
  progress.
  Cross-session upload temporary-name collisions retain their separate P1 entry.
  Verify these before accepting full SFTP parity.
- Native SFTP resolves remote symbolic links with `readlink` and followed `stat`,
  retaining the alias name/path for downloads and directory navigation. Resolve
  relative targets against the listed item's parent, including after path editing
  or failed navigation. Folder downloads use followed file-link sizes and retain
  the original link modes. Renderer checks compare the exact fixed methods;
  native tests use the actual SFTP backend with an in-process SSH/SFTP peer.
  A macOS real OpenSSH fixture verifies relative and absolute file/directory
  links, a dangling link, followed attributes, alias names, exact binary reads,
  EOF, close, and directory/broken-link download rejection. Run the isolated
  `ssh::sftp::integration::link_acceptance` ignored test with
  `TABBY_RS_SSH_INTEGRATION=1`. Rendered downloads and other supported platforms
  remain pending.
- Unix local downloads apply the requested mode at creation, limited by the process
  umask. Hold the staged file inside an owned 0700 directory until completion,
  cancellation, error, or owner drop. Tests exercise actual TransferManager size
  guards, destination preservation, permissions, privacy, and cleanup. The shared
  terminal export receives the same creation rule. Existing-destination permission
  parity and rendered/platform acceptance remain pending.
- Directory-delete cancellation returns before sending `sftp.remove` or refreshing
  the listing. `scripts/test-sftp-delete-cancellation.mjs` compares the exact
  upstream cancellation method and exercises the native method with simulated
  UI/SFTP boundaries. It also covers confirmed deletion, files, dismissed action
  prompts, and display-only names. Verify cancellation in the supported desktops;
  these method checks do not establish supported-desktop acceptance or full SFTP parity.
- Cancellation during deletion stops subsequent children and parent removal.
  An already sent removal may finish; retain the modal operation after its UI
  closes and await its settlement before refreshing the unchanged current path.
  Refresh after partial errors as well. Component destruction only stops further
  operations and must not close an already closed modal again.

## P1 — SFTP concurrent uploads share temporary names

- Each SftpManager starts its transfer counter at zero and opens
  `{final_path}.tabby-upload-{id}` with CREATE, TRUNCATE, and WRITE. Two SSH
  sessions uploading the same destination can truncate or mix the same temporary
  file; completion or cancellation can remove the other session's file. This
  behavior already exists in the base commit.
- Give each upload a unique temporary path and atomically create it without
  truncating an existing upload. Verify simultaneous sessions, independent
  cancellation, rename failure, and destination preservation with an isolated
  SSH/SFTP peer before accepting concurrent-upload behavior.

## P2 — Folder file-link downloads inherit the upstream link mode

- Fixed `14e2d60` folder downloads pass each listed item's mode to createFile,
  including a symbolic link's 0777 bits. Under umask 022, the completed local
  file can be readable by other local users even when the remote target is 0600.
  The native flow retains this original behavior; its staged file stays inside
  an owned 0700 directory before completion.
- Verify completed-file permissions separately from staging privacy. Changing
  them to the followed target's mode requires a decision on the fixed-baseline
  parity contract; keep the existing exact-original mode assertions until then.

## P2 — Ports modal focus after removing a forwarding row

- Removing the focused row button moves focus to the body in the Angular
  fixture. Escape then does not reach the shared NgbModalWindow listener;
  focusing the forwarding editor restores dismissal. Compare this flow in the
  original desktop before changing shared focus behavior or accepting Escape
  handling after row removal.

## P2 — SFTP peers that omit file size are treated as empty files

- `src-tauri/src/ssh/sftp/backend.rs` maps missing size attributes to zero in
  RemoteFileEntry, while native download descriptors preserve an optional size.
  Renderer downloads send the entry's zero as an advertised size, so a nonempty
  file from such a peer is rejected by the existing transfer size guard.
- Compare the fixed upstream unknown-size flow and preserve a missing size across
  listing, stat, renderer transfer setup, and IPC. Keep known-size overflow and
  incomplete-close guards; link support does not resolve this earlier gap.

## Pending — SSH tab lifecycle platform acceptance

- Tauri SSH tabs use the fixed `14e2d60` session-end policy: the colored host
  session-closed message precedes the shared policy, automatic reconnect is
  immediate, and `exit` plus Enter or a final Ctrl+D count as explicit termination.
  Do not restore the custom five-attempt delayed tab reconnect.
- `scripts/test-ssh-tab-lifecycle.mjs` executes the extracted upstream/current tab
  and shared lifecycle methods. It covers keep/reconnect/close/auto, manual
  disconnect, absent frontends, explicit and near-miss inputs, clearing the ended
  session, one reconnection prompt, and reconnecting once on the next key.
  Native inline-auth state is cleared with or without a frontend.
- The fixture passed for upstream and failed for the former native behavior
  before the fix. Both now pass.
- macOS arm64 desktop checks through real loopback SSH connections verify the
  colored host-closed message, a single next-key reconnection, automatic
  reconnection after EOF, close-on-end, and auto mode retaining an EOF-ended tab
  while closing after `exit` or Ctrl+D. Manual Disconnect does not automatically
  reconnect, and another tab remains usable. Closing the fixture window leaves
  no open server connections. The server echoes data and sends EOF; these checks
  do not establish execution of a remote shell. Original desktop comparison and
  other supported platforms still need acceptance before claiming full SSH parity.

## Pending — SSH port forwarding startup acceptance

- Local, Dynamic, and Remote startup failures are handled per forwarding entry,
  so they do not end the authenticated SSH session or trigger its reconnect policy.
  Later forwards are attempted, successful IDs are retained for teardown, and
  service messages use the fixed upstream badges, arrows, and forwarding descriptions.
  Entries without an exact Local, Remote, or Dynamic type are ignored like `14e2d60`.
- `scripts/test-ssh-forwarding-startup.mjs` executes the real Tauri session and
  BaseSession lifecycle against the verbatim upstream `addPortForward` method and
  entire `ForwardedPort` class, including its field initialization. It covers each
  rejection mode, failure at each
  position in a mixed list, invalid JSON entries, structured native errors,
  successful diagnostic bytes, authentication identity, input/resize, and cleanup.
  `test:ssh-reconnect` includes this fixture. These checks do not verify startup timing.
  Omitted-target cases force transport rejection to compare failure descriptions;
  they do not establish target-validation or successful-startup parity. Native
  validation rejects missing targets before starting Local/Remote forwards, while
  the fixed upstream can establish listeners without those fields. Compare that
  difference before classifying it as parity or an accepted safety exception.
- Native forwarding failures preserve the actual bind or request error through
  the existing `AppError` I/O payload. `src-tauri/src/ssh/forwarding_tests.rs`
  checks a real occupied TCP port, ephemeral-port binary traffic and release,
  and remote forwarding rejection through the production control handler.
  The rejection test verifies retained SSH input/output and no registered route.
  All three tests and the full macOS suite pass. Platform-specific error strings
  and their rendered diagnostics still need desktop acceptance.
- Explicit empty bind hosts are replaced with `127.0.0.1` by the renderer,
  while diagnostics retain the configured empty value. The fixed upstream passes
  the empty value to its listener or remote forwarding request. A real Node 22
  probe binds `::`; RFC 4254 section 7.1 gives empty remote bind addresses all
  supported protocol families. Resolve this behavior and message mismatch with
  an explicit security decision before accepting parity; no exception is approved.
- Startup still awaits each forwarding request; the fixed upstream starts them
  without waiting. Buffered early output is drained after forwarding setup.
  A real loopback probe executes the exact upstream start, addPortForward, and
  openShellChannel methods with the locked russh binding. A silent remote-forward
  request leaves later shell opening pending for the five-second observation;
  an established shell exchanges binary data while the request remains pending.
  Its raw channel-close API closes the channel, but the exact SSHShellSession
  destroy method does not call that API. Shell destruction immediately emits
  closed/destroyed notifications and starts disconnect without awaiting it.
  No peer channel close or transport disconnect is observed before peer shutdown
  during the five-second check; the disconnect call remains pending. All fixture
  connections and listeners close cleanly, with no unhandled rejection. These are
  method-level observations, not original desktop or native Tauri acceptance.
  Compare startup ordering, login scripts, and rendered close behavior against
  these results before changing the waiting policy.
- The production shell loop continues reading ordered output while a control
  request awaits its result. Real loopback SSH tests exercise the shared helper
  with generic pending operations: binary stdout/stderr, queued output before
  EOF, operation cancellation, remote CLOSE acknowledgement with blocked input
  and a full queue, retained transport, status/signal, and operation results.
  Ready control results take priority over queued output and EOF. After stopping
  input on EOF or failed output delivery, preserve an available reply with one
  non-blocking check which ignores cooperative scheduling yields; genuinely pending
  work is cancelled. Retain EOF for the outer loop when returning a ready result.
  Failed output delivery always ends the loop. Native loopback tests verify actual
  SftpList validation and Close replies; deterministic oneshot tests verify readiness
  between polls and exhausted Tokio task budget. These checks establish native reply
  delivery, not remote global-request timing. Renderer tests keep native close pending
  and verify that closing output is neither displayed nor retained in the pre-connect
  buffer. A separate macOS real-SSH probe exercises the production StartRemoteForward
  control and shell-loop helpers with an unanswered global request. Exact binary
  input, stdout, and stderr continue while its reply is pending. EOF retains the
  final output, stops input, and cancels the pending operation with a dropped reply
  sender. Transport disconnection returns the forwarding error and drains through
  transport completion. Both paths close the connection before fixture shutdown;
  no pending replies or connections remain. This is a handler/loop-helper check,
  not the full native manager task, rendered desktop, or original desktop comparison.
  Desktop acceptance remains pending.
  Later native controls remain queued behind the pending request. Compare actual
  wire cleanup, resize, SFTP, and forwarding startup with the exact upstream shell
  before accepting those flows.
- Resize errors are returned to the caller while the shell loop drains queued output.
  A real loopback regression selects an actual Resize control before the peer sends
  binary output and disconnects. It verifies the Closed reply, preserved loop result,
  exact final bytes, and transport completion. Live resize also preserves shell input
  and output. These tests call the production control handler and reader helpers.
  The exact 14e2d60 resize method calls resizePTY without a session-close step. A
  method-level probe with the locked binding verifies live resize, final output before
  one closed/destroyed notification, and a post-close SendError without another
  notification. It does not reproduce the same failed-resize-before-tail interleaving
  or provide original desktop acceptance. The direct fixture also does not establish
  jump-transport shutdown: verify final output and ssh:exit after target disconnection
  while its jump host stays connected. Rendered resize behavior and other supported
  platforms remain pending.
- Renderer destruction completes the logical session while cancellation, SFTP
  shutdown, forwarding stops, and native close finish in the background. The
  exact upstream session classes provide the reference for
  `scripts/test-ssh-close-lifecycle.mjs`. The checks verify immediate lifecycle
  notifications, independent cleanup initiation, duplicate and rejected cleanup,
  pending cancellation with late registration, and replacement-session event and
  native ID isolation before and after old cleanup. They execute the current
  shared disconnect/reconnect methods with a simulated tab host, not the rendered
  tab lifecycle. Native socket/process cleanup and desktop behavior remain pending.
  Current shared reconnect awaits logical destruction and marks a manual restart;
  the upstream method does neither. Compare automatic reconnect and manual restart
  in the actual desktop before accepting that shared-flow difference.
- Renderer closure now stops a forwarding ID returned after destruction;
  regression tests cover the late reply and concurrent pending removal. Native
  cancellation broadcast timing before listener subscription remains unverified.
  Reproduce that timing with a hostname bind and confirm the port is released
  before changing startup to run in the background.
- Failed native forwarding starts retain entries until session closure. Verify
  repeated failures, list consumers, and listener cleanup before accepting modal
  forwarding runtime behavior. An open Ports modal also stays bound to its old
  session through reconnect; compare that transition with the locked upstream
  binding before accepting it.
- macOS desktop failure/success diagnostics and terminal input after all three
  rejection modes remain unverified: the current fixture cannot be inspected while
  the Mac is locked. Original desktop comparison and supported-platform checks
  remain pending.

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
- Preserve the approved SSH forwarding safety exception: native handlers close an
  unsolicited agent or X11 channel before opening a local endpoint when its target
  forwarding option is disabled. The exact `14e2d60` handlers forward those channels.
  This difference is accepted; its scope and approval are recorded in
  `docs/release-acceptance.md#approved-ssh-forwarding-safety-exception`.
  Keep normal forwarding and supported-platform acceptance separate from this exception.
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

## Pending — native window lifecycle and event isolation

- Native `CloseRequested` must call `prevent_close()` before requesting renderer
  confirmation, and send the event only to the owning window. After shared
  `AppService.closeWindow` confirms every tab, `window_close` destroys that window
  without generating another close request. The multi-window source contract
  checks the native wiring; the Telnet fixture executes the real AppService close
  methods and verifies cancellation preserves every tab, while approval closes
  tabs before the window. macOS desktop native-window closure with two Telnet tabs
  displays the actual translated warning. Cancelling keeps both tabs and the open
  connection usable; confirming closes the window and its remaining TCP connection.
  Two-window local PTY checks also verify process cleanup. SSH/serial resources,
  cancellation across multiple live windows, and other platforms need acceptance.
  Separately verify application Quit/Cmd+Q: `app_quit` calls `app.exit(0)` directly,
  so this window-close fix does not establish application-quit parity.
- Window event producers must use `emit_to(label)` and the renderer must listen
  through `getCurrentWebviewWindow().listen`. Tauri's default Any listener still
  receives events sent to another label. Keep both halves for focus, movement,
  resize, close, file drop, theme, and scale-factor events. The multi-window fixture
  checks both labels, delivery isolation, unsubscription, and retained global
  events. A separate check against the actual Tauri 2.11.5 global JS bundle confirms
  these seven registrations use `WebviewWindow` plus the current label, while global
  hotkeys use `Any`. This validates registration and wiring, not desktop behavior.
- `closeAllTabs` invokes tab destruction without awaiting asynchronous session
  cleanup. Verify server connections and PTY processes after closing a secondary
  window before accepting cleanup; the native managers do not handle window
  destruction by owner. Do not assume the passing permission tests prove cleanup.
- Launch requests use a per-window native queue and a scoped `app:launch` wake-up.
  Register the listener before draining `app.initialLaunch`, serialize reads, and
  retain requests until Angular readiness. Preserve the original CLI handlers and
  second-instance fallback; consume routing flags when a request creates a window.
  The incoming FIFO channel exists before plugin setup; its single receiver starts
  after app state initialization. Keep target selection and delivery on the event
  thread with window destruction, and window creation on the receiver thread.
  Mark approved closes before calling `destroy()` and exclude closing labels;
  Tauri can retain a destroyed native window in its map until the delayed
  `Destroyed` notification. Restore eligibility and notify on destroy failure.
  Native tests cover ordering, isolation, concurrent consumption, cleanup, and flag
  normalization. Renderer tests cover registration/readiness timing and duplicate
  notifications. macOS desktop checks on `b3fae75c` verify a second invocation,
  explicit new-window creation, routing to the latest window while the original
  is focused, delivery after minimizing the latest window, and delivery after
  closing the main window. The surviving terminal round-trips keyboard input.
  Closing both isolated test windows leaves none of their five PTY child processes
  running. These checks use a freshly built portable app with isolated settings.
  Concurrent startup, cold deep links, Windows WebView2, other supported platforms,
  and cleanup of SSH, Telnet, and serial sessions still need desktop acceptance.
  See `docs/architecture/identity-and-launch.md` before changing this contract.
- Verify macOS cold deep-link startup event timing against the original app. If
  `RunEvent::Opened` arrives after setup, a second-instance URL may follow the
  ordinary initial request and cause an extra auto-opened tab. The queued delivery
  preserves the event; neither its timing nor exact cold-start UI is established
  by the renderer fixtures. Do not claim cold deep-link parity without that check.

## Pending — Telnet and serial login scripts and desktop connection startup

- Tauri Telnet now uses the fixed upstream close confirmation for open sessions.
  `scripts/test-telnet-tab-controls.mjs` executes both actual component methods
  and compares translated dialog options, inactive sessions, confirm/cancel,
  unknown responses, dialog failure, and waiting for the response. The tab and
  session remain intact while checking permission. Tab close buttons, middle-click,
  close hotkeys, and window closure use the shared `canClose` flow. Verify the
  tab-close buttons, middle-click, and close hotkeys on supported desktops.
  macOS native-window closure verifies the rendered translated dialog, cancellation
  with both tabs retained and continued Telnet input, confirmation, and TCP cleanup.
  Other close entry points and supported platforms still need acceptance.
- Telnet and serial tab termination use the fixed upstream `onSessionDestroyed`
  behavior: a colored session-closed message precedes the shared end-of-session
  policy. Keep the frontend guard and shared immediate reconnect; do not restore
  the custom five-attempt delayed tab reconnect. The device-reconnect test runs
  actual current and `14e2d60` tab methods plus extracted shared lifecycle methods.
  It covers keep/reconnect/close/auto, manual disconnect, explicit quit/close,
  absent frontends, a single key prompt, and reconnection on the next key.
  macOS Telnet desktop checks verify the rendered closed badge, keep plus key
  reconnection, automatic reconnect after peer closure, and manual menu disconnect
  without automatic reconnect. Cancelling the second of two native window-close
  warnings preserves both tabs and both TCP connections with working input.
  Confirmed window closure leaves no test TCP connections or fixture app running.
  Serial desktop/physical-device checks, native device-level automatic reconnect,
  and other supported platforms still need acceptance.
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
- The browser readline adapter restores the spinner and stream editor using the
  fixed upstream lock's Electron 38.8.6 / Node 22.22.0 editing modules. Retain their
  MIT notices and the Unicode License V3 for the ICU-derived width table. The
  checked-in regeneration script and provenance hashes identify the exact source.
  The bundle test reproduces the original missing-clearLine failure and compares
  12 editing scenarios with the pinned Electron runtime, including output bytes,
  history, Unicode input, and cleanup. It also checks all 1,114,112 code point
  widths against that runtime's ICU and verifies Windows/macOS Ctrl+Z remains usable.
  The renderer now maps native `windows`/`macos` to Node `win32`/`darwin` names.
- macOS desktop Telnet checks against a loopback server that processes Telnet
  negotiation verify both login scripts, a visible SCRIPT_OK response, line
  editing with cursor movement/deletion, history resend, and hex input sending
  exact bytes. Closing the isolated app closes all three tested connections.
  A final-build macOS desktop check also sends `ab`, Ctrl+Z, `cd`, Enter and
  receives `ECHO:abcd`; the server records exactly `abcd\n` and clean closure.
  This fixture does not establish complete Telnet protocol or platform parity.
  Serial desktop acceptance remains pending: opening a macOS pseudo-terminal
  fails with `Not a typewriter` in the native serial backend. Telnet and serial
  startup errors now extract native `details`, then JavaScript `message`, with
  string conversion as a fallback. Their error badge and text use the original
  ANSI colors. Component tests cover nine error shapes per connector, spinner
  shutdown, and awaited session cleanup; they reproduce `[object Object]` before
  the fix. Verify the rendered diagnostic and retry flow in the macOS desktop:
  the current check cannot proceed while the Mac is locked. Serial hardware
  exchanges and supported-platform acceptance remain pending.
- Linux Ctrl+Z remains a known adapter gap: Node readline requests renderer
  process suspension, but the WebView process shim has no `kill` API. This can
  throw and stall the input stream. Verify the original Linux desktop
  behavior and implement the corresponding host integration before acceptance;
  do not silently substitute Windows' no-op behavior on Linux. A macOS method-level
  probe in Electron 38.8.6 with the original BrowserWindow preferences confirms
  SIGTSTP is ignored in that launch environment: input after Ctrl+Z completes
  normally without an external SIGCONT. The adapter preserves that observable
  behavior on macOS. Other launch/process-group environments remain unverified.
- Serial Slow feed uses the original middleware position and a renderer Writable
  queue. Preserve `_writev` batching across consecutive inputs and login scripts;
  the native side must not split those batches again. Tests compare the actual
  browser Writable with the pinned upstream SerialPortStream under controlled
  write completion, including binary data, errors, close, and legacy settings.
  They also pass in Electron 38.8.6 / Node 22.22.0. Batches larger than the native
  1 MiB limit are sent in ordered calls and stop when the session closes.
  With automatic reconnect enabled, failed writes discard the old queue without
  destroying the session. Drop input while disconnected; a connected event starts
  a fresh queue. Tests cover write failure before the state event and an old
  callback arriving after reconnection. Verify these flows on physical hardware.
- Native serial reads and writes use separate handles to the same port. Tests
  using real Unix pseudo-terminals verify writes while a read waits, output while
  a 1 MiB write stalls, exact bytes, disconnected/replaced ports, and permanent
  closure. Unix descriptors must remain nonblocking so a large write cannot block
  inside the driver after readiness polling. Close waits for the writer to stop;
  regression tests verify remaining bytes are cancelled before acknowledgement.
  Unix write readiness timeouts must retry: flow control can pause transmission
  longer than the read timeout. The stalled-write test waits across three timeout
  periods, receives output, then verifies the full 1 MiB after the peer resumes.
  Read-side disconnection signals cancellation before waiting for the writer lock;
  a stalled-write regression verifies this releases the handle for reconnection.
  Reject queued work from an earlier port generation after reconnect. These
  fixtures do not establish physical serial or Windows behavior. Toggle
  persistence, automatic reconnect, rendered controls, physical serial exchanges,
  and supported-platform acceptance still need desktop checks. Hardware and
  Windows cancellation/cleanup timing need comparison with the fixed upstream.
  The Windows backend uses synchronous duplicated COM handles, whereas upstream
  uses overlapped I/O. Verify simultaneous read/write and zero-byte timeout returns
  before accepting Windows parity; the Unix pseudo-terminal tests do not cover them.
- Script deletion persistence, serial automatic-reconnect script behavior,
  and other platforms remain unaccepted. Do not infer
  full connector parity from the session tests or rendered settings controls.
- Serial output uses the original UTF8SplitterMiddleware before InputProcessor.
  Session tests verify Chinese, emoji, and accented characters split at every
  byte boundary, plus incomplete output flushed on close. Telnet's existing
  streaming decoder passes the same split-character check. Verify rendered
  serial Unicode output on supported desktop platforms before acceptance.
- Serial tabs retain the fixed upstream Home/End sequences, focus guard, delayed
  profile title, and explicit `close`/`quit` termination detection. The toolbar
  uses the original Pug structure, labels, classes, and connected/disconnected
  button conditions, with `baudRate` adapted to the Tauri profile. The component
  test compares the actual pinned upstream methods and compiled templates;
  rendered layout and real desktop interaction still need acceptance.
- `serial.setBaudRate` updates the live native port on its control thread without
  taking the writer lock. Retain a successful rate in the request used for native
  reconnect, and propagate driver failures without updating that request. The UI
  retains the selected profile rate and closes a failed session with a notification,
  matching upstream's update-error path. Selector cancellation sends no update.
  While native automatic reconnect waits for a device, accept the selected rate
  for the next open without destroying the session. Reset its bounded retry
  count on this explicit setting change so exhausted attempts do not prevent
  recovery with a newly selected rate. Session tests cover bridge
  payloads, reconnect-wait updates, and closed guards. Native tests cover zero
  rates, missing sessions, reconnect-wait updates, and control processing while the
  writer lock is held. macOS pseudo-terminals reject IOSSIOSPEED with `Not a
  typewriter`; the macOS check verifies driver-error propagation, unchanged native
  reconnect settings, and continued port I/O. Successful physical rate changes,
  automatic reconnect at the new rate, and Windows/Linux execution remain pending.

## Pending — lint from the shared Main checkout

- `npm run lint` in `/Volumes/SSD/Developer/TABBY-RS` reaches the Node heap
  limit at about 4 GiB and exits with status 134 before producing lint results.
  That checkout is mounted from `mac-1` over SMB. The full lint command passes
  in the local verification directory with all 1,344 compared file hashes equal
  to Main; the private `.env` is excluded from that comparison.
- Investigate the shared-checkout heap growth before claiming an in-place lint
  pass. Preserve both the failed Main run and passing local run, and verify the
  source hashes when using the local copy for acceptance.
