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

## Pending — full-repo lint fails on `tabby-local/src/session.ts`

- `yarn lint --format unix` fails with 1 problem:
  `tabby-local/src/session.ts:67:24: Unnecessary parentheses around expression. [Error/@typescript-eslint/no-extra-parens]`
- `HEAD` and the working tree match for this file (`git diff HEAD -- tabby-local/src/session.ts`
  is empty), so the failure is pre-existing.
- Unrelated to the current WinSCP/SSH changes; still pending.

## Pending — SSH forwarding desktop and Windows acceptance

- Renderer and native request tests cover selecting a forwarding socket independently
  of password, private-key, and keyboard-interactive login.
- `src-tauri/src/ssh/engine_integration.rs` verifies unsolicited agent and X11 channels
  against a loopback SSH server, with password/private-key login and forwarding on/off.
  The production forwarding helpers reject disabled channels before connecting to the
  local socket. Actual Tauri handler wiring, desktop controls, and multi-hop forwarding
  still need end-to-end acceptance.
- `src-tauri/src/ssh/engine.rs` now shares `connect_agent` with jump authentication and
  forwarding. On Windows, `None` selects Pageant and an explicit path selects a named pipe.
  Verify Pageant with `SSH_AUTH_SOCK` set and verify named-pipe connections on Windows.
- Local `cargo check --target x86_64-pc-windows-msvc` stops in the `ring` dependency
  because the host lacks Windows C headers (`assert.h`). It does not establish that the
  Windows application code compiles; Windows compilation and runtime checks are pending.

## Pending — X11 transport parity and desktop acceptance

- Forwarding `ssh.x11Display` into the native request does not complete X11 parity.
  `server_channel_open_x11` still closes the channel on non-Unix hosts.
- `connect_x11_display` always adds 6000 to numeric display values; upstream
  `X11Socket.resolveDisplaySpec` treats values of 100 or greater as raw TCP ports.
- Verify the SSH close confirmation in the real desktop UI and validate agent/X11
  behavior on the supported operating systems before accepting complete parity.

## Pending — SSH password prompts and automatic authentication parity

- Configured profile passwords are now tried in automatic and explicit password modes.
  Native tests verify rejection falls through to stored passwords and Debug redacts the
  supplied value. Repeated configured/stored passwords are tried only once. Renderer
  tests cover ordering and separate jump-host passwords; manual macOS desktop loopback
  checks verify both modes authenticate with the configured password.
  Keyboard-interactive prefill from configured and stored passwords remains pending.
- Keyboard-interactive challenges with no prompts now receive an empty response
  automatically, matching `14e2d60:tabby-ssh/src/session/ssh.ts`. Unit tests cover
  repeated empty rounds, rejection/fallback, non-empty prompts, and transport errors.
  Loopback integration tests cover acceptance and rejection after two empty rounds.
  A direct macOS desktop connection verifies two automatic rounds followed by a
  masked password prompt and successful login. Jump-host desktop acceptance remains
  pending; the latest attempt expired while waiting for host-key confirmation.
- Keyboard-interactive UI parity remains pending: the current modal does not submit
  on Enter in the direct macOS check. Compare the upstream inline panel's Enter,
  Previous/Next/Finish, and Save password behavior before accepting this flow.
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
- Upstream `14e2d60:tabby-ssh/src/session/ssh.ts` deletes saved passwords after rejected
  authentication. Compare and implement the matching resolved-account behavior.
- Direct authentication shares a 120-second deadline across username, password, and key
  prompts; jump authentication does not share that deadline. Verify slow interactive
  login against upstream before accepting timeout parity.
