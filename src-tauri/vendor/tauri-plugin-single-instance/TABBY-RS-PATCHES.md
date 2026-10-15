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
deep-link dispatch, semver handling, and native argument framing are unchanged.
macOS cleanup retains the selected socket instead of deriving another socket
from the bundle identifier.

The application derives the portable override from its configured identifier
and canonical data directory. See `docs/architecture/identity-and-launch.md`.
Before upgrading or removing the patch, run the resolver regressions in
`src-tauri/src/portable_instance_tests.rs`, the real native transport regressions
in `src-tauri/src/instance_transport_tests.rs`, and concurrent full-app launches
from different portable directories. Verify same-directory handoff, independent
directories, installed-mode handoff, and cleanup that preserves other instances
on each supported desktop platform.
