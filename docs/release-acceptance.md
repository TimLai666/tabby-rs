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
   `accepted-difference` only with a written reason and an explicit approval.
4. Do not put passwords, private keys, tokens, personal data, or full config
   files in evidence. Redact terminal output and screenshots before upload.
5. Keep the original Tabby installation and data directory untouched during
   side-by-side tests. Record both application identities and the paths used.

## Evidence record

Create one JSON record per platform under the release staging directory. The
following shape is the minimum required record; additional fields may describe
the environment or attach evidence files.

```json
{
  "schemaVersion": 1,
  "kind": "tabby-rs-manual-platform-acceptance",
  "sourceRevision": "<git sha>",
  "platform": "windows-x64",
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
  "artifacts": [
    { "path": "<relative artifact path>", "sha256": "<digest>" }
  ]
}
```

The record must identify the same `sourceRevision`, platform, architecture, and
target as the parity manifest and release artifact reports. A report that only
says “smoke passed” is insufficient.

## Windows x64

Run on a real Windows 11 host with the fresh NSIS installer and the exact
artifact under test:

- `local-shell`: start and close the configured local shell, send input,
  resize, interrupt, and verify clean process exit.
- `powershell`, `cmd`, `wsl`, and `git-bash`: start each shell, run a command,
  verify output and resize, then close it without leaving a child process.
- `clink` and `uac`: verify the configured integration and an elevated session;
  record the Windows build and integration versions.
- `ssh` and `serial`: connect to the approved fixtures or attached device,
  verify input/output, resize or line settings, disconnect, and reconnect.
- `nsis`: install, launch, update or rollback using the approved artifact, then
  uninstall and verify the application executable and data boundary are gone.
- `side-by-side`: run original Tabby and Tabby RS simultaneously, verify
  independent windows, config paths, plugins, and uninstall isolation.

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
Epic #1 child issues #26 or #27 are open.
