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
- `src-tauri/src/ssh/mod.rs` uses and zeroizes a prompted key passphrase without
  saving it. WinSCP conversion reads only saved passphrases, so a key unlocked by
  that prompt can fail conversion. Check upstream reuse and consent behavior
  before changing passphrase persistence.

## Pending — full-repo lint fails on `tabby-local/src/session.ts`

- `yarn lint --format unix` fails with 1 problem:
  `tabby-local/src/session.ts:67:24: Unnecessary parentheses around expression. [Error/@typescript-eslint/no-extra-parens]`
- `HEAD` and the working tree match for this file (`git diff HEAD -- tabby-local/src/session.ts`
  is empty), so the failure is pre-existing.
- Unrelated to the current WinSCP/SSH changes; still pending.

## P2 — SSH agent forwarding and jump authentication still have parity gaps

- `src-tauri/src/ssh/mod.rs` builds `SshHandler.agent_socket` only from agent entries
  in `request.auth`. Password, private-key-only, and keyboard-interactive profiles
  therefore ignore configured agent paths when `agentForward` is enabled.
- Before SSH forwarding acceptance: resolve the forwarding agent independently from
  the login method and test a custom socket with password and private-key login.
- The Windows `connect_agent(None)` helper used by jump authentication and forwarding
  tries `SSH_AUTH_SOCK` before Pageant, while direct authentication routes `None` to
  Pageant. Verify explicit Pageant mode with `SSH_AUTH_SOCK` set and align both paths
  with upstream `14e2d60` before claiming full agent parity.

## Pending — X11 transport parity and desktop acceptance

- Forwarding `ssh.x11Display` into the native request does not complete X11 parity.
  `server_channel_open_x11` still closes the channel on non-Unix hosts.
- `connect_x11_display` always adds 6000 to numeric display values; upstream
  `X11Socket.resolveDisplaySpec` treats values of 100 or greater as raw TCP ports.
- Verify the SSH close confirmation in the real desktop UI and validate agent/X11
  behavior on the supported operating systems before accepting complete parity.
