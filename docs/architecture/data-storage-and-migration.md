# Tabby RS data storage and first-run migration

This document defines the persistence, backup, and one-way import contract for the Tauri host.

## Ownership boundary

Angular continues to own the public `config.yaml` schema and its existing migration rules. Rust stores the YAML as opaque UTF-8 text and is responsible for:

- selecting the Tabby RS data root established by the identity layer;
- refusing symbolic-link targets in managed storage;
- serializing mutations through one process-wide storage lock;
- detecting concurrent edits with SHA-256 revisions;
- writing complete files through temporary-file replacement and filesystem sync;
- storing Tabby RS-only state separately from public Tabby configuration.

The host never rewrites the original Tabby directory.

## Managed layout

```text
<data-dir>/
  config.yaml
  tabby-rs.json
  pending-update.json
  backups/
    <backup-id>/
      manifest.json
      files/
        config.yaml
        tabby-rs.json
  migration/
    import-<timestamp>-journal.json
    import-<timestamp>-report.json
```

Portable mode uses the same layout under the portable data root. Installed mode uses the independent Tabby RS application data directory.

## Configuration revisions

`config.read` returns the YAML text, its path, and a SHA-256 revision. `config.write` accepts the revision observed by the caller. A changed or newly created file produces a structured conflict instead of overwriting another writer.

The first write requires the file to remain absent when it was absent during load. Unknown keys and plugin-owned fields are preserved because Rust never parses and emits the configuration during normal saves.

## Internal state

`tabby-rs.json` starts at schema version 1 and contains only host-owned metadata:

- first-run import status and report references;
- update channel and stable-backup pointer;
- safe-mode metadata;
- local diagnostic-log preference;
- plugin names pending reinstallation;
- optional normal window bounds in logical pixels and the maximized flag (`windowState`).

Unknown top-level fields are retained when the state is loaded and saved. A state schema newer than the current reader is rejected.

The window-state field is optional within schema 1. Older files load without saved
bounds, and saves omit the field while it is absent. Existing readers preserve it
through their unknown-field map. Backups and portable roots include it through the
existing `tabby-rs.json` file. macOS construction reads saved bounds through the
native adapter. macOS move/resize events update the per-window memory cache;
renderer-approved close saves it with the live maximized flag. Each window retains
its construction-time maximized flag separately from later samples and saves.
The first finished page applies that flag before showing, unless the initial main
window was launched hidden. Full application startup and desktop persistence
acceptance remain pending.

Linux construction registers automatic configured windows before Tauri builds
or defers them for portable storage. Requested windows register before their
builder runs. Stock geometry initialization retains its existing gate; custom
minimums and explicit configured origins remain available to restoration. The
native controller restores saved bounds, drains native work, arms capture, then
completes the owning initialization ticket. Move/resize events update the cache;
destruction removes only the owning tracker and ticket. Approved close uses the
shared close controller. A first-finished-page hook applies saved maximization,
shows and focuses once, and preserves hidden main startup. Linux renderer readiness
only releases queued launches. The fixed original's native theme setter runs only
on macOS; Linux first presentation does not read appearance config or set a theme.
Full rendered application and supported-desktop persistence acceptance remain
pending. Its public physical bounds getter reads the X11 client rectangle and encloses it in logical integer
pixels before converting back to physical pixels. A client origin on an odd
physical pixel at scale two can therefore enlarge the reported physical size
without resizing the native client. The native content-size fixture independently
checks both the requested X11 client dimensions and that enclosing rectangle.
Hidden windows retain GTK's requested size. Keep a sufficiently large logical
desktop when testing requested sizes against a window manager.

Do not copy the macOS restoration policy into Linux without a native control.
The fixed constructor on Electron 38.8.6 / Glasstron 0.1.1, under Xvfb/Openbox,
returns (609, 314) for an off-screen 701x451 window on a 1920x1080 logical display,
although the computed options are (609.5, 314.5). It retains a requested 200x100
rectangle before mapping and applies the 400x300 minimum when shown. A saved
maximized flag skips the constructor's off-screen relocation. Subsequent native
move/resize events and the window manager can change the normal cache before
maximization is reported; compare their ordering before accepting persistence.
These are isolated constructor/property observations, not original desktop or
full-application acceptance. The Wayland constructor control runs without X11
initialization rejections when Weston provides XWayland for Glasstron. At scale
two, unchanged first display preserves absent saved bounds or an off-screen seed,
while an explicit maximize/unmaximize cycle records the restored constructor
rectangle. Electron retains its requested logical origin even when Wayland
cannot provide global positions.

Tauri sends initial move/resize configure events on that same Wayland display.
For Wayland, the optional constructor sample in `WindowStates` suppresses unchanged
startup samples without replacing saved bounds. A changed normal sample or an observed
maximized state ends startup suppression; unmaximizing then records normal bounds
even when they equal the constructor sample. The ordinary `ready` operation
retains its existing capture policy. The AppState sampling bridge never writes
on movement. Its filesystem tests verify close-time persistence, independent
windows, unknown labels, and metadata preservation.

X11 selects `ready_with_origin` instead. It seeds the sampling origin without
promoting it into saved bounds and accepts the first subsequent normal sample.
An isolated Electron 38.8.6/Glasstron control with the fixed constructor options
and verbatim move/resize cache callbacks records normal bounds on first X11 show,
including absent and relocated off-screen seeds. The same Wayland control retains
those seeds. Both controls cover Native/Thin and saved maximization on/off; their
callback/property observations do not establish original desktop acceptance.
The controller selects this policy through the owning GTK display's positioning
query; query errors leave capture unarmed.

`window_persistence_linux` registers each label through AppState, restores native
constructor options, and returns the raw constructor sample. A native restoration
error removes only that registration. Readiness waits for the operation barrier,
then arms the owning backend's sampling policy without saving the sample. Capture reads
with the latest logical origin and only updates the in-memory tracker. Unknown
or removed labels without an origin are ignored; native errors retain the cache.
Mapped normal Wayland capture also requires the native client size to match the
existing default content allocation. GTK can report 848x598 after unmaximization
while that allocation is still 3840x2081; immediately recording that sample would
replace the previous fully allocated 900x650 normal bounds. `read_for_capture`
reads both observations in one native UI callback and returns no sample during
that mismatch. The controller retains its cache. Public native reads still
expose the actual transient size; hidden, maximized, and X11 samples retain their
existing behavior. No requested-size cache or additional event listener is used.
`test-linux-window-capture.mjs` loads this controller and the real tracker with
simulated native/AppState boundaries, including failure, cancellation, duplicate
readiness, absent bounds, and independent windows.

`window_capture_wayland` runs the actual controller, native adapter, and tracker
with real Native/frameless Wry events and absent/off-screen seeds. It checks the
maximization cycle, resize, destruction, and an unaffected peer at scales one and
two. AppState, the durable seed, event registration, and the main-thread barrier
are fixture boundaries. This example does not exercise the application's startup
registration, actual AppState/disk writes, or rendered application behavior.
The separate blank-page application/IPC probe reaches those production boundaries,
but does not accept Angular rendering or full desktop parity. The former
readiness operation is retained as a failing `--stock-control`.
An after-default native window-state handler is armed only after the 900x650
normal cache is verified. It observes an inconsistent normal sample and checks
cache retention immediately, before later configure events can repair it. The
pre-guard control records 848x598 and fails; the guard passes all 48 checks across
both scales and seed modes, including a subsequent genuine 901x651 resize and
peer/destruction checks. This deliberately timed native capture does not prove
that the separate application after-paint timeout is resolved.

The private `window_geometry_linux_coordinates` resolver retains the fixed
constructor options separately from native placement. Nearest-monitor selection
uses Chromium's half-open containment followed by Manhattan rectangle distance,
including first-monitor ties and zero distance at the right/bottom edges. GTK's
nearest-monitor helper adds one beyond those edges and can select another monitor.
Relocation keeps the fixed strict disjointness check, maximized-state guard, and
origin-free centering. A half-pixel result on either axis selects native default
centering rather than rounding either explicit coordinate. Native acquisition,
default centering, and application startup require separate verification.

The private `platform/window_state_linux.rs` adapter restores hidden constructor
options and reads native client snapshots. It retains constructor metadata
separately from GTK minimum clamping, and keeps Wayland origins as logical
metadata. The real `window_saved_linux` fixture covers independent Native and
frameless windows, odd and oversized default centering, saved maximization
bypass, mapped maximize/unmaximize, fullscreen, and destroyed handles.
Multiple monitors and full rendered desktop acceptance remain pending.

Linux bounds application explicitly samples the normal cache in one UI callback
before final maximization or visibility changes. Hidden GTK windows can omit
configure events. The blank-page application/IPC checks verify hidden resize,
reload, and approved-close saving on X11 and Wayland at scales one and two.
The Linux completion adapter drains Tao requests through a GLib idle and checks
native state after actual after-paint notifications. Bounds are checked against
GTK integer rounding and the original setter's logical 400x300 minimum. Ordinary
bounds-only requests preserve a maximized or fullscreen window. Hidden state
waits retain constructor semantics without mapping the window; hidden bounds
still require the requested metadata. Each wait is independent, cancellable, and
bounded to two seconds. Destroyed native windows may remove their frame-clock
handlers before cleanup; check connection ownership before disconnecting.

Wayland GTK decoration transitions can change the shadow extents added to a
resize request after normal state is acknowledged. Initialization installs
`decoration { transition: none; }` on the owning window's style context. The
provider retains the theme's final style and the global animation preference.
The real Wry fixture checks Native and frameless windows with 40 cases per X11
scale and 38 per Wayland scale, all passing. A stable native decoration render
at scale two has identical before/after PNG bytes. All four blank-page
application startup/reload/close cases pass, including mapped maximize-to-normal
bounds followed immediately by hiding. The fixed original's Native and Thin
Wayland API controls also retain the immediate requested bounds at both scales.
These controls do not establish full original desktop or Angular UI acceptance.

Linux docking uses the same state-application path, including the off action
that only clears always-on-top. The command waits for that path before reading its reply and the normal
cache is available to approved close. The extracted-command regression uses the
production docking geometry with simulated framework/application boundaries.
Real X11 blank-page checks cover four directions, hidden and mapped windows,
and scales one and two. All 20 geometry/reply/save and always-on-top/disable
cases pass. Absolute positioning waits for effective client size before moving:
a GTK move can overtake the queued Tao resize and offset right placement by one
pixel. The native ordering control fails all 12 immediate resize/move trials and
passes all 12 size-before-move trials. Size observation ignores the current
origin and never issues a resize. Mapped X11 uses raw XGetGeometry dimensions
under a GDK error trap; GDK's logical geometry can truncate physical pixels.
Hidden observation preserves unrealized metadata, and shared cancellation and
two-second limits remain in force. Preserve exact requested bounds, reply/later
state equality, and approved-close persistence. Full rendered docking, multiple
monitors, and original desktop acceptance remain pending.

The local Tao 0.35.3 patch keeps its existing always-on-top atomic as the configured
level, initialized from construction and updated after successful request dispatch.
Window-state events retain their maximization/minimization handling. GDK ABOVE can
be absent for never-shown windows and stale for mapped windows even when the actual
X11 property has changed. This follows Electron 38.8.6's configured Widget level
query. The application bridge validates the live Linux native window on the UI
thread before setting it; no separate application cache or forced realization is
needed. Read `src-tauri/vendor/tao/TABBY-RS-PATCHES.md` before updating that dependency.
`test-window-topmost.mjs` exercises bridge routing and errors with simulated native
boundaries. The real `window_topmost_linux` Wry fixture passes 20 checks on X11 at
each scale, including actual ABOVE, unrealized hidden toggles, builder hints,
independent windows, clearing the level, and destroyed/rebuilt-window isolation.
These native checks do not establish Wayland compositor support, rendered controls,
original full-desktop comparison, or installer acceptance.
The fixed service also chooses the display nearest the cursor when its configured
screen is unavailable, while the native fallback selects the primary display.
Multi-monitor behavior requires its own original and native acceptance.

The fixed Electron/Glasstron property control reports `isMaximized() == false`
while iconified or fullscreen. On X11, GTK retains MAXIMIZED together with
ICONIFIED; on Wayland, it can retain MAXIMIZED together with FULLSCREEN. The
private snapshot excludes both states. X11 restore retains prior maximization.
The isolated Wayland control restores to a normal window after minimization,
while GTK receives no ICONIFIED acknowledgement and retains maximization.
Wayland minimization/restore remains an unresolved application flow; focus loss
is not evidence of minimization.

Rapid maximization followed by an explicit normal-size patch also remains unresolved.
One current scale-two Wayland run timed out awaiting after-paint and reported/saved
848x598 logical content after requesting 900x650. Subsequent clean and diagnostic
repeats passed. That does not erase the failure or establish its root cause; GTK
state, style changes, configure requests, allocation and paint must be distinguished.

A separate mapped-fullscreen-peer probe reproduces the after-paint timeout during
initial normal-state preparation, before the native 900x650 resize request is
sent. An owned GTK peer reports both mapped and fullscreen. The application
retains its prior 800x600 normal bounds during the failure. After that peer exits,
a retry in the same application window reaches 900x650 and saves those bounds.
A separate native frame-clock observer records one after-paint signal, followed
by a 7.56-second gap. Signals resume after the fullscreen peer exits; the same
window then completes the retry. Mapping therefore does not guarantee a paint
for this completion predicate. The original rapid failure remains separate.
The isolated Electron 38.8.6/Glasstron control, using the fixed constructor, returns
900x650 from unmaximize/setBounds while its fullscreen peer remains mapped.
These observations establish an additional operation-completion gap; they do not
establish the trigger of the earlier rapid 848x598 failure or accept the original
desktop. No pixel capture establishes the peers' exact stacking. The public-call
preload does not intercept GTK/GDK internal allocation/freeze calls. The separate
signal observer records native window/content allocations and frame-clock phases;
neither observer derives an internal freeze count.

A separate GTK 3.24.49 probe can advance an ordinary resize with repeated
`gtk_container_check_resize` while frame-clock layout and after-paint remain
paused. It does not establish style completion: a pending descendant minimum
leaves native and content dimensions coherently at 900x650, then normal CSS
validation changes both to 1046x650 after the fullscreen peer exits. Invalidating
the two explicit widget contexts fixes that constructed case, without establishing
validation of the full descendant/CSD tree or reentrant style changes. A separate
maximize/unmaximize probe also remains maximized despite matching native/content
allocation before the inverse request. Neither observation permits replacing
the transition commit barrier with allocation alone.

`window_completion_occluded_linux` exercises the actual completion helpers in
Wry windows with a mapped fullscreen peer, observing the main frame clock before
checking no-op state, size and bounds waits plus final native/content dimensions.
Its `--without-peer` mode retains the same operation and geometry assertions.
Both scale-one and scale-two controls pass all eight phases. With the peer mapped
and fullscreen, each run observes paused redraw and then fails all six operation
phases: same-size, normal/maximized state, normal/fullscreen state, same-bounds,
resize size, and final bounds/content allocation. The actual client remains at
800x600 physical pixels. The helpers report after-paint timeouts rather than the
requested 900x650 completion. Both windows are removed before each normal exit.
These primitive checks do not establish rendered Tabby or original desktop parity.

The public snapshot and maximize-toggle command in `commands/desktop.rs` use
`platform::window_is_maximized`. On macOS and Linux it calls the same native
predicate as saved-window capture, without acquiring unrelated geometry or
monitor data. Read errors reach the caller before any toggle action. Other
hosts retain their existing Tauri predicate. `test-window-maximized.mjs` executes
the actual predicate and command methods with simulated native/Tauri boundaries,
covering both toggle directions, one action per request, and read/action errors.
The asynchronous toggle shares the existing operation mutex with apply/presentation/
close, drains earlier work before reading state, and retains the lock through
completion. Linux also awaits native maximization and a paint before replying. An
observed rapid unmaximize formerly reached Wayland before the maximized surface
commit and could leave maximization active. The fixture covers mutex exclusion,
initial/final barrier errors, Linux acknowledgement, cancellation, and peer progress.
A current ad-hoc signed macOS application renders the saved 700x450 window, zooms
through the native control, restores those bounds through the tab-bar double-click
command, and preserves them on ordinary Cmd+Q. Its data and WebView are isolated.
This accepts that specific current-application flow, without an original-desktop
comparison or other-platform acceptance.
The native `window_saved_linux` fixture checks the bridge getter during normal,
maximized, fullscreen, restored, and destroyed states on X11 and Wayland at
scales one and two; minimized-state checks run on X11. These checks do not accept
rendered controls, IPC serialization, or supported-desktop behavior.

The fixed Electron macOS predicate excludes minimized/fullscreen windows and
uses an AppKit zoom/geometry check, which the existing native adapter retains
([Electron 38.8.6 macOS source](https://github.com/electron/electron/blob/v38.8.6/shell/browser/native_window_mac.mm#L501-L513)).
Its Windows predicate also recognizes a non-minimized transparent window whose
bounds equal its display work area
([Electron 38.8.6 Windows source](https://github.com/electron/electron/blob/v38.8.6/shell/browser/native_window_views.cc#L667-L680)).
Compare that Windows behavior with actual native snapshot/toggle requests before
acceptance; retaining the existing Tauri fallback does not establish equivalence.

Before mapping, Tauri's invisible WebView container reports a 400x300 size
request but contributes no preferred size to GTK. Native Wayland can therefore
report a raw 348x248 hidden window for requested 200x100 or 1x1 sizes. Once shown,
the actual content allocation and client size become 400x300. Preserve the raw
constructor sample and read GTK's actual size; verify the content minimum after
mapping. The native fixture checks both zero and small requests, alongside
independent windows and state transitions, on X11 and Wayland at scales one and
two. These primitive checks do not accept the pending application wiring.

Keep native default centering under comparison. On Xvfb/Openbox at scale two,
a fresh Wry 701x451 window centers at (610, 315), while the fixed constructor
returns (609, 314). Wry also retains negative default-center coordinates for a
2501x1801 window where the fixed constructor returns (0, 0). Centering after a
hidden GTK resize can retain the former 800x600 position. Fresh Wry windows with
400x300 minimums already report 400x300 for requested 1x1 or 200x100 sizes before
mapping; the fixed constructor reports 1x1 or 200x100 until shown. Its zero-width
or zero-height requests become one native pixel on that axis before mapping.
Do not accept post-construction resizing, generic `center`, or eager minimum-size
clamping as equivalent saved-window restoration from these property checks.

The private Windows coordinate core in
`src-tauri/src/window_geometry_windows_coordinates.rs` follows Electron 38.8.6 /
Chromium 140.0.7339.249. It builds logical monitor bounds from native enumeration
order, physical bounds, display IDs, and effective float32 scales. Rectangle and
point conversions retain their separate monitor-selection rules. Its tests cover
mixed scales, edge alignment, touching and overlapping layouts, empty rectangles,
and float32 rounding. Native monitor acquisition and saved-window integration
remain pending. The public physical bounds API retains its existing contract.

The private macOS adapter in `src-tauri/src/platform/window_state_macos.rs`
reads the current AppKit outer frame in Electron logical coordinates. Its
maximized predicate excludes minimized and fullscreen windows. Restoration
honors live outer-size constraints, including AppKit's large unlimited maximum,
and completes the native frame write before returning. Screen selection retains
native enumeration order, containment, corner-distance ties, and the original
primary/secondary coordinate conversion. Unit checks and owned blank Wry windows
cover these operations, including off-screen startup relocation and fractional
position fallback. The macOS primary, automatic configured, and newly requested
windows invoke restoration during initialization. Bootstrap and concurrent launch
presentation use the existing initialization registry to wait for completion or
retain a restoration error. Capture is enabled only after construction events
drain through the existing native operation barrier. The blank Wry fixture
executes the production restore/capture methods with real native events and the
actual tracker; its AppState boundary is substituted. The bootstrap fixture
executes the real command with simulated Tauri boundaries. First-page presentation
uses the same initialization ticket and serializes native appearance, maximization,
and showing with geometry operations. Its controller fixture simulates OS and
AppState/config I/O boundaries. Full application presentation timing, fullscreen
transitions, and original desktop acceptance remain pending.

`AppState::update_persisted_state` prepares a candidate under the existing storage
lock and publishes it to process memory only after saving succeeds. A rejected
write before replacement retains the previous in-memory metadata for a later
retry. An error after file replacement, such as a directory-sync failure, can
leave the file changed despite the returned error. The real filesystem tests in
`src-tauri/src/state/persistence_tests.rs` cover rejected replacement, recovery,
and reopening a saved window state through a new host state.

`AppState` also owns the window-local tracker through
`src-tauri/src/state/window_state.rs`. Registration reads the current persisted
window state, while move/resize samples update only a ready window's memory.
The explicit save method persists that label's cached normal bounds with the
supplied live maximized flag through `update_persisted_state`. Unknown or removed
labels do not write; rejected writes retain the tracker for retry. The tracker
lock is released before taking the storage lock. The AppState/filesystem tests
cover independent windows, the last successful save, fresh registrations after
state updates, duplicate labels, destruction, and recovery from a rejected save.
The macOS event adapter feeds this tracker after restoration events drain.
The macOS and Linux close commands use `window_close.rs`, which atomically claims
renderer-approved closure before its
first wait, serializes with geometry patches, and drains pending native work.
It reads the live maximized flag and saves the cached normal bounds before
destruction. A duplicate pending close does nothing. Cancellation or failed
destruction restores launch routing and wakes only the owning renderer; read or
save errors are logged while approved destruction continues. The close fixture
uses the production controller and launch queue with simulated OS/state I/O.
Linux reads the latest sampling origin before its native snapshot; without an
origin it skips sampling and saving while approved destruction continues. Linux
startup and move/resize registration feed this cache before approved close.
Blank-page application probes verify hidden resize and combined unmaximize/resize/
hide followed by reload and exact durable bounds at scales one and two on X11
and Wayland. Full rendered persistence, confirmation/cancellation, original desktop
comparison, and supported-platform presentation acceptance remain pending.

Before an updater install, Rust writes `pending-update.json` atomically after creating the `before-update` backup. It contains only the target version, backup id, and channel. Startup reads this journal independently from `tabby-rs.json`: if the installed version matches and the configuration or state is unreadable, it restores the recorded backup before clearing the journal. A successful startup also clears the journal. A failed install clears it before returning control to the current version.

## Backups

A backup is a directory with a JSON manifest, not an archive. The manifest records:

- backup identity, timestamp, reason, source app version, and update channel;
- every stored file's relative name, size, and SHA-256 checksum;
- managed files that did not exist at snapshot time.

Restore accepts only the fixed managed names `config.yaml` and `tabby-rs.json`.
It validates the selected manifest, managed paths, and file checksums, then keeps
all selected bytes in an owned `PreparedRestore` before creating the safety
backup. Safety creation retains the existing limit of 20 backups and can prune
the selected source directory. Applying the prepared bytes still restores the
selected snapshot without changing that retention policy. Target files use the
existing atomic replacement and removal guards; absent files are removed only
when they are regular files. Paths from a manifest cannot escape the managed
data root.

`AppState::restore_backup` holds the storage lock through the selected snapshot,
state validation, and in-memory publication. Later state updates therefore start
from the restored metadata. A snapshot with no `tabby-rs.json` resets the cache to
defaults. Invalid JSON or a newer state schema fails the restore and attempts to
reapply the safety snapshot; successful rollback also reloads the cache before
returning the original failure. The AppState filesystem tests cover these paths
and subsequent writes.

## Import discovery

The host checks only known Tabby and historical Terminus configuration locations for the current platform, plus `TABBY_CONFIG_DIRECTORY` when explicitly supplied. Discovery:

- canonicalizes and deduplicates candidates;
- skips symlink directories and the active Tabby RS data directory;
- reads a regular `config.yaml` no larger than 16 MiB;
- reports profile count, plugin package names, secret-bearing field paths, and source revision;
- writes nothing to the source or target configuration.

Secret values are never included in an import plan or report.

## One-way import

The user selects whether to import configuration and which detected plugin names should be queued for later installation. Import then:

1. re-detects the source and validates the selection;
2. creates a `before-first-import` backup;
3. writes a migration journal and marks state as running;
4. verifies the source revision;
5. copies `config.yaml` byte-for-byte without deleting or modifying the source;
6. verifies the source again after the copy;
7. records selected plugin names without copying executable plugin code;
8. writes the report before marking the import completed.

Any failure attempts to restore the pre-import snapshot and records a failed report. A successful import reloads the application so Angular reads the imported YAML with a fresh revision.

The Tauri updater API currently buffers the complete response in memory during download. After size, hash, and signature verification, the host stores the artifact in a private temporary file until installation, and removes it when the ready, failed, cancelled, or completed state is dropped. The host does not claim resumable or streamed transport until the updater API supports it.

## Deferred secret migration

This issue identifies secret-bearing paths and reports which fields need review. Reading the original Vault or operating-system keychain and transferring secrets belongs to the dedicated Vault migration issue. Plain secret values must never appear in the migration UI, journal, report, or logs.
