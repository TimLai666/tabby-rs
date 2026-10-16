# Tabby RS single-instance patch

This directory contains the published tauri-plugin-single-instance 2.4.3 crate.

- Archive SHA-256: b3214becf9ef5783c0ae99a3bb25adf5353a7a16ebf53e74b909e29205735c6c
- Upstream source commit: cad301fcc1f3ebad1eaef552c886b0bc8580c3fe
- Source directory: plugins/single-instance in https://github.com/tauri-apps/plugins-workspace
- License: Apache-2.0 OR MIT; both license texts and SPDX declarations are retained.
- The unused library Cargo.lock, README banner image, and .cargo-ok cache marker are omitted.

`Builder::instance_id` optionally overrides the native communication identity.
macOS uses it for its socket; Windows uses it for its mutex and hidden-window
names; Linux uses it when no explicit `dbus_id` is supplied. With no override,
the configured application identifier retains its original role. Callback,
deep-link dispatch, and semver handling retain their existing behavior.
macOS cleanup retains the selected socket instead of deriving another socket
from the bundle identifier.

Windows uses a private `WM_APP + 1542` capability query. A receiver supporting
the structured protocol returns exactly `0x54420001`; the sender then sends a
JSON `[cwd, args]` frame with `WM_COPYDATA.dwData = 1543`. This preserves pipes,
Unicode, quotes, and empty arguments. The receiver also accepts the original
NUL-terminated `dwData = 1542` format, reading only the declared `cbData` bytes.
Malformed or unknown frames are rejected without invoking the callback.

An older receiver gets the original format when it can represent every value.
Requests containing a pipe fail with restart guidance before delivery. Only a
handled-message reply of `1` permits the secondary process to exit successfully.
The query uses a separate message because the original receiver acknowledges
unknown `WM_COPYDATA` formats without processing them. macOS and Linux retain
their existing argument framing.

Windows setup retains the existing mutex handle until it can discover the
receiver or acquire ownership. An owner that has not created its window keeps
other launches pending; releasing or abandoning the mutex lets one waiter
become the primary. Native creation and wait errors stop setup instead of
initializing another writer. Receiver teardown precedes mutex release, and
failed receiver construction retains callback ownership until it is freed.

The application derives the portable override from its configured identifier
and canonical data directory. See `docs/architecture/identity-and-launch.md`.
Before upgrading or removing the patch, run the resolver regressions in
`src-tauri/src/portable_instance_tests.rs`, the real native transport regressions
in `src-tauri/src/instance_transport_tests.rs`, the codec checks in
`src-tauri/src/windows_instance_message_tests.rs`, the Windows native
compatibility checks in `src-tauri/src/windows_instance_transport_tests.rs`,
the startup ownership checks in `src-tauri/src/windows_instance_startup_tests.rs`,
and concurrent full-app launches
from different portable directories. Verify same-directory handoff, independent
directories, installed-mode handoff, and cleanup that preserves other instances
on each supported desktop platform.
