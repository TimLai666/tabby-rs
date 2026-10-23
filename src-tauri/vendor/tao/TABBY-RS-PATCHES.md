# Tabby RS Tao patch

This directory contains the published Tao 0.35.3 crate under Apache-2.0.
The crate archive SHA-256 is d1c93047acf68669466a34690ac58cca7010bd1b201e1ec86f1fd0a75d3dd4a9.
Its published VCS metadata identifies commit 5a14e624c81b7a799728129417e9218be25f17d9
in https://github.com/tauri-apps/tao and marks the release source dirty.
LICENSE and LICENSE.spdx are retained. Cargo cache markers are not included.

The source modification is limited to src/platform_impl/linux/window.rs:

- Keep the existing per-window always-on-top atomic as the configured level.
  Initialize it from the constructor and update it after successfully sending
  an AlwaysOnTop request. A failed send retains the previous level.
- Window-state events continue to update maximization and minimization, but do
  not replace the configured level with GDK's ABOVE flag. Never-shown windows
  have no surface, and mapped X11 windows can change _NET_WM_STATE_ABOVE while
  GDK keeps reporting no ABOVE flag. Electron 38.8.6's BaseWindow getter reads
  the configured Widget ZOrderLevel; Tabby RS follows that contract.

Native setters and other platform implementations are unchanged. This reports
configured state, not proof that a compositor honors the request. Wayland
compositor support must be assessed separately.

Before updating Tao or removing this patch, run window_topmost_linux with real
Wry windows on X11 at scales one and two, including never-shown true/false,
constructor hints, independent windows, real _NET_WM_STATE_ABOVE, hide/show,
and destruction/recreation. Run the native docking/application IPC matrix and
cross-platform host suites as well. Keep rendered desktop and installer
acceptance separate from these controlled checks.
