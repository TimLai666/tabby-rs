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
- Also open: the existing-instance hand-off is unverified. `launch.rs:4-9` notes that a
  running WinSCP may take the session over, so the only process lifetime relied on is the one
  this module waits for. That path needs Windows evidence.

## Pending — full-repo lint fails on `tabby-local/src/session.ts`

- `yarn lint --format unix` fails with 1 problem:
  `tabby-local/src/session.ts:67:24: Unnecessary parentheses around expression. [Error/@typescript-eslint/no-extra-parens]`
- `HEAD` and the working tree match for this file (`git diff HEAD -- tabby-local/src/session.ts`
  is empty), so the failure is pre-existing.
- Unrelated to the current WinSCP/SSH changes; still pending.
