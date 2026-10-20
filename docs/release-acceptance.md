# Tabby RS release acceptance

This checklist records the human acceptance evidence required by
`parity/features.yaml` and `parity/platform-matrix.yaml`. It supplements the
automated evidence collector; it does not turn an automated fixture into a
human platform result.

## Evidence rules

1. Run the checklist against the exact source revision and artifact that will
   be reported. Record the OS version, architecture, Tauri/WebView version,
   toolchain, test time, and artifact SHA-256 before testing.
2. Record one result for every `requiredChecks` entry and every feature's
   `tests.manual` entry: `passed`, `failed`, or `not-run`. A missing, failed, or
   `not-run` check blocks the corresponding manifest entry.
3. A manifest entry may change from `pending` to `passed` only when its evidence
   points to the captured report, screenshot, log, or artifact. Use
   `accepted-difference` only with a written reason, evidence files that exist
   in the release checkout, and an approval object containing
   `decision: accepted-difference`, `approver`, and an RFC 3339 `approvedAt`.
4. Do not put passwords, private keys, tokens, personal data, or full config
   files in evidence. Redact terminal output and screenshots before upload.
5. Keep the original Tabby installation and data directory untouched during
   side-by-side tests. Record both application identities and the paths used.

## Evidence record

Create one JSON record per desktop platform under the release staging directory.
For Web, create `manual-acceptance/web.json` using the same `environment`,
`features`, and `artifacts` fields but with `kind` set to
`tabby-rs-manual-feature-acceptance` and `platform` set to `web`; Web has no
desktop platform-matrix checks. The following shape is the minimum required
desktop record; additional fields may describe the environment or attach
evidence files.

```json
{
  "schemaVersion": 1,
  "kind": "tabby-rs-manual-platform-acceptance",
  "sourceRevision": "<git sha>",
  "platform": "windows-x64",
  "architecture": "x86_64",
  "target": "x86_64-pc-windows-msvc",
  "environment": {
    "os": "<name and version>",
    "webview": "<version>",
    "toolchain": "<versions>",
    "testedAt": "<RFC 3339>"
  },
  "checks": [
    {
      "id": "local-shell",
      "status": "passed",
      "steps": ["<observable action and result>"],
      "evidence": ["manual/windows-x64/local-shell.txt"]
    }
  ],
  "features": [
    {
      "id": "local-shell.profiles-and-pty",
      "status": "passed",
      "steps": ["started a shell, resized the terminal, interrupted it, and verified clean exit"],
      "evidence": ["manual/windows-x64/local-shell.profiles-and-pty.txt"]
    }
  ],
  "artifacts": [
    { "path": "<relative artifact path>", "sha256": "<digest>" }
  ]
}
```

The record must identify the same `sourceRevision`, platform, architecture, and
target as the parity manifest and release artifact reports. Evidence and
artifact paths are relative to the release staging directory, must point to
existing files, and artifact SHA-256 values are recomputed by the validator.
Its `features` array must cover every feature with a manual test for that
platform. A report that only says “smoke passed” is insufficient.

Validate each record before adding it to a release staging directory:

```text
node scripts/check-manual-platform-acceptance.mjs \
  --record release-staging/manual-acceptance/windows-x64.json \
  --evidence-root release-staging \
  --source-revision "$GITHUB_SHA" \
  --architecture x86_64 \
  --target x86_64-pc-windows-msvc
```

The release gate accepts a desktop manual record only when every
platform-matrix check and every platform-scoped manual feature is present,
marked `passed`, backed by observable steps and relative evidence paths, and
bound to the same revision, architecture, and target. The Web record applies
the same evidence rules to every Web-scoped manual feature, without desktop
architecture or target fields. If a platform manifest is changed to `passed`,
the gate requires its record under
`release-staging/manual-acceptance/<platform-id>.json`; a passed Web feature
requires `release-staging/manual-acceptance/web.json`.

## Approved SSH forwarding safety exception

When the target profile disables agent or X11 forwarding, Tabby RS closes the
corresponding unsolicited server channel before connecting to the local agent
or X server. The fixed `14e2d60` incoming-channel handlers forward those channels
without checking the target options. Preserving the native checks prevents a
server from reaching a local endpoint after the user has disabled forwarding.

The user approved this behavior as an exception to complete parity:

```json
{
  "id": "ssh.disabled-forwarding-consent",
  "baseline": "14e2d60b9b6dee84a53c37f05eefeb803787de04",
  "approval": {
    "decision": "accepted-difference",
    "approver": "user",
    "approvedAt": "2026-10-09T15:38:43.497591Z"
  }
}
```

The checks are implemented by `forward_agent_channel` and `forward_x11_channel`
in `src-tauri/src/ssh/mod.rs`. The real SSH fixtures in
`src-tauri/src/ssh/engine_integration.rs` verify channel closure and no local
connection when disabled, plus successful forwarding when enabled, with password
and private-key login. Run them with:

```text
cargo test --manifest-path src-tauri/Cargo.toml --lib forwarding_consent -- --include-ignored
```

Apply this exception only to rejection of disabled incoming channels. Normal
forwarding, jump hosts, platform behavior, and the remaining manual checks keep
their existing acceptance requirements. Keep their feature manifest entries
pending until the complete feature scope has been verified.

## Approved empty SSH bind-host safety exception

For Local, Dynamic, and Remote forwarding, Tabby RS substitutes `127.0.0.1`
when the configured bind host is empty or missing. For Local and Dynamic,
this limits the requested listener to the local computer. For Remote, it
requests a loopback listener on the SSH server; the server controls whether
and where that listener is actually bound. Explicit hosts, including
`0.0.0.0` and `::`, are passed through unchanged. Profile settings are not
rewritten, and diagnostics retain the configured host like the fixed upstream.

The fixed `14e2d60` `addPortForward` and `ForwardedPort` methods pass an
explicit empty host through to the local listener or Remote request. That
can request a wildcard listener. The user approved retaining the native
loopback default as a narrow exception to complete parity. The timestamp
records this approval when entered in the release contract:

```json
{
  "id": "ssh.empty-bind-loopback",
  "baseline": "14e2d60b9b6dee84a53c37f05eefeb803787de04",
  "approval": {
    "decision": "accepted-difference",
    "approver": "user",
    "approvedAt": "2026-10-20T16:07:19Z"
  }
}
```

`TauriSshSession.addPortForward` in `tabby-tauri/src/ssh/session.ts` applies
this default to both profile startup and interactive additions. Run:

```text
node scripts/test-ssh-forwarding-startup.mjs
```

This fixture executes the actual session methods with simulated native IPC.
It verifies Local, Dynamic, and Remote requests through both entry points,
empty/missing defaults, unchanged explicit IPv4/IPv6/hostname values,
unchanged configuration and diagnostics, and forwarding teardown. Its fixed
upstream control verifies that an explicit empty host is passed through.
It does not prove native listener exposure or server policy. Keep native
network checks, supported-desktop acceptance, startup ordering, and all
other forwarding requirements pending until separately verified.

## Approved SFTP overwrite safety exception

Tabby RS refuses to replace an existing destination when the SFTP server does
not advertise version 1 of posix-rename@openssh.com. The error explains that
atomic replacement is unsupported, the existing destination was preserved,
and the upload needs a different name. The check uses LSTAT so a dangling
symbolic link also counts as an existing entry. New destinations can use
ordinary SFTP RENAME after an explicit NoSuchFile response. Other metadata
errors stop the commit.

The fixed 14e2d60 upload in tabby-ssh/src/session/sftp.ts deletes the destination
before renaming the staging file. A failed rename after that deletion can lose
the previous file. The user approved preserving the old file and reporting
failure as a narrow exception to complete parity. The timestamp below records
the approval when it was received by this task:

```json
{
  "id": "ssh.sftp.nonatomic-overwrite",
  "baseline": "14e2d60b9b6dee84a53c37f05eefeb803787de04",
  "approval": {
    "decision": "accepted-difference",
    "approver": "user",
    "approvedAt": "2026-10-14T10:12:58Z"
  }
}
```

RusshSftpBackend::replace_file in src-tauri/src/ssh/sftp/backend.rs implements
this rule. The serialized protocol fixtures in
src-tauri/src/ssh/sftp/overwrite_tests.rs verify unsupported and mismatched
capabilities, preserved binary/empty files and dangling links, metadata
failures, new files, a competing destination, and isolated staging cleanup.
The supported-capability and concurrent-upload checks remain in that file and
src-tauri/src/ssh/sftp/upload_tests.rs. Run them with:

```text
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib ssh::sftp
```

Apply this exception only to existing-destination replacement without the
supported extension. Supported OpenSSH replacement, ordinary file creation,
Skip and Rename policies, and all desktop checks keep their existing acceptance
requirements. A lost connection or reply after a mutation leaves the remote
result uncertain. Keep the SFTP and platform manifest entries pending until
their complete scopes have been verified.

## Approved Remote allocated-port safety exception

Tabby RS rejects a successful port-zero Remote forwarding reply when the
allocated value is zero or exceeds 65535. The forwarding error retains the
original value. Rejection keeps the SSH session and existing routes usable,
removes only this request's pending registration, and closes its early incoming
channels before connecting to a local target. It sends no cancellation for the
invalid value, which could otherwise be truncated to another forwarding port.
Configured nonzero ports and valid allocated ports retain their existing behavior.

The fixed `14e2d60` Remote forwarding method in tabby-ssh/src/session/ssh.ts
awaits the binding request without this allocated-value check. The former native
handler cast the server's `u32` result to `u16`, so 65537 could be returned as 1.
The user approved explicit rejection as a narrow exception to complete parity.
The timestamp records this approval when entered in the release contract:

```json
{
  "id": "ssh.remote-allocated-port-range",
  "baseline": "14e2d60b9b6dee84a53c37f05eefeb803787de04",
  "approval": {
    "decision": "accepted-difference",
    "approver": "user",
    "approvedAt": "2026-10-15T11:48:57Z"
  }
}
```

`handle_control` in src-tauri/src/ssh/mod.rs checks the range before route commit
and conversion. The actual SSH protocol fixtures in
src-tauri/src/ssh/remote_allocation_tests.rs cover zero, 65536, 65537, `u32::MAX`,
valid 1 and 65535, retained binary shell and forwarding traffic, the same owner's
existing route, early-channel closure, and the absence of cancellation requests.
The boundary cases verify route values and forwarded channels; they do not
require an operating-system listener on port 1 or 65535. Run them with:

```text
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib ssh::remote_forwarding_tests::allocation_validation
```

Apply this exception only to invalid allocated Remote reply values. Keep the SSH
and platform manifest entries pending until their complete scopes and supported
desktop behavior have been verified.

## Approved CLI paste behavior exception

After confirmation, `tabby paste` sends text to the currently focused terminal
inside the active split container. It preserves cancellation, shell escaping,
and the guard against missing terminal sessions. Other panes receive no input.
The fixed `14e2d60` handler checks the top-level active tab directly; ordinary
tabs are split containers, so confirmation can finish without sending input.

The user approved this focused-terminal correction as a narrow exception to
complete parity, conditional on actual Windows paste verification before merge.
The timestamp records this approval when entered in the release contract:

```json
{
  "id": "terminal.cli-paste-focused-pane",
  "baseline": "14e2d60b9b6dee84a53c37f05eefeb803787de04",
  "approval": {
    "decision": "accepted-difference",
    "approver": "user",
    "approvedAt": "2026-10-16T13:09:39Z"
  }
}
```

`TerminalCLIHandler` in `tabby-terminal/src/cli.ts` resolves the focused pane
only after confirmation. Run its controlled routing regression with
`yarn test:terminal-cli-paste`. This test exercises the actual handler and split
focus accessor with modal and terminal boundaries substituted. Actual native
confirmation, complete pasted text, terminal output, and process cleanup still
need desktop evidence. Keep CLI and platform manifest entries pending until
their complete scopes have been verified.

## Windows x64

Run on a real Windows host matching the currently supported Tauri/CI
environment, record its actual OS version, and use the fresh NSIS installer and
the exact artifact under test:

- `local-shell`: start and close the configured local shell, send input,
  resize, interrupt, and verify clean process exit.
- `powershell`, `cmd`, `wsl`, `git-bash`, and `visual-studio-developer-shell`:
  start each shell, run a command,
  verify output and resize, then close it without leaving a child process.
- `clink` and `uac`: verify the configured integration and an elevated session;
  record the Windows build and integration versions.
- `ssh` and `serial`: connect to the approved fixtures or attached device,
  verify input/output, resize or line settings, disconnect, and reconnect.
- `nsis`: install, launch, update or rollback using the approved artifact, then
  uninstall and verify the application executable and data boundary are gone.
- `side-by-side`: run original Tabby and Tabby RS simultaneously, verify
  independent windows, config paths, plugins, and uninstall isolation.

### WinSCP feature acceptance

Record the `ssh-winscp` manual flow under `features` with ID `ssh.winscp`.
Compare both SSH-tab entry points with the fixed original on the same Windows
host, using isolated test profiles, credentials, and application copies:

- Launch from live and ended SSH tabs, then after successful, failed, and
  cancelled reconnects. Verify the selected host, port, resolved username,
  current-directory option where supplied, and the last authenticated identity.
- Verify password and private-key sessions, saved and prompted passphrases with
  Remember checked and unchecked, and jump-host connection options. Include
  sessions with no configured private keys and preserve the original behavior.
- Start a new WinSCP process and hand a session to an already running instance.
  Verify the actual selected connection and continued usability, plus diagnostics
  when the executable is unavailable or key conversion or launch fails.
- Inspect owned source-key stages and converted keys after normal WinSCP exit,
  application quit during conversion or launch, and isolated crash recovery.
  Require cleanup of abandoned keys while preserving keys still used by another
  process or active WinSCP session. Keep this acceptance pending until the
  existing temporary-key cleanup gap has been resolved and verified.

The service and session fixtures substitute process/IPC boundaries. Passing
`tauri-platform`, `ssh-reconnect`, or Rust tests does not replace these desktop,
WinSCP-process, and key-file lifecycle checks. Keep `ssh.winscp` pending until
its complete scope has evidence for the exact release revision and artifact.

## macOS Intel and Apple Silicon

Run the same checklist independently on `macos-x64` and `macos-arm64`:

- `local-shell`: verify the default shell, input, resize, signals, and clean
  exit from a fresh app launch.
- `keychain`: save and retrieve a test credential through the system Keychain,
  then remove only that test entry.
- `ssh` and `serial`: verify authentication, host-key handling, terminal I/O,
  serial settings, disconnect, and reconnect on approved fixtures/devices.
- `dmg`: mount the fresh DMG, verify the volume and app icons, launch the app,
  install, and uninstall without changing the original Tabby data directory.
- `side-by-side`: run original Tabby and Tabby RS together and verify separate
  identity, config, plugin, and update paths.

Record whether the host is Intel or Apple Silicon and the macOS/WebView
versions. Do not use a cached DMG or Finder thumbnail as icon evidence.

## Linux x64

Run on the supported Ubuntu host and record the session type and package
versions:

- `local-shell`: verify PTY input, resize, signals, output, and clean exit.
- `x11-or-wayland`: verify launch and window/input behavior under the active
  display session.
- `webkitgtk`: record the installed WebKitGTK version and verify the complete
  UI starts and accepts terminal input.
- `secret-service`: test both available and unavailable secret-service paths;
  verify the unavailable path reports a structured limitation.
- `ssh` and `serial`: verify the approved remote fixture or attached device,
  input/output, settings, disconnect, and reconnect.
- `appimage`, `deb`, and `rpm`: install or execute each generated package,
  launch it, verify the package metadata and uninstall boundary.
- `side-by-side`: run original Tabby and Tabby RS together with independent
  config, plugin, and data paths.

## Final manifest update

After review, add the evidence record paths to the affected `evidence` arrays
and change status only for entries whose complete scope passed. Keep failed or
unverified entries as `pending` or `failed`; run `yarn test:parity-report` and
the aggregate release gate again. The stable workflow remains blocked while
Epic #1 child issues #2 through #27 are open.
