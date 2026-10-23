# Tabby RS identity and launch contract

This document defines the independent desktop identity, structured launch input, deep-link grammar, portable mode, and optional CLI alias for Tabby RS.

## Identity

| Surface | Value |
| --- | --- |
| Product name | `Tabby RS` |
| Tauri application identifier | `io.tabbyrs.app` |
| Primary CLI | `tabby-rs` |
| Desktop URL scheme | `tabby-rs://` |
| Data directory name | `tabby-rs` |
| Credential service name | `tabby-rs` |

Tabby RS must never write into the original Tabby application directory, credential namespace, URL scheme, or updater channel.

## Runtime paths

Installed mode stores host state in `<system-data-directory>/tabby-rs`.

Portable mode is selected before any config, plugin, log, or credential-backed service is initialized. It is enabled when either of these entries exists beside the executable:

- `.tabby-rs-portable`
- `data/`, retained as a compatibility marker for the original portable convention

In portable mode all file-backed state is rooted at `<executable-directory>/data`.
The executable and portable marker are resolved once before Tauri creates any
WebView. The same startup snapshot selects host paths and the
`data/webview` directory for every configured or subsequently created window.
Installed mode retains its existing WebView configuration.

On Windows and Linux, portable configured windows are created during setup after
host state is registered. Each keeps its full startup configuration and receives
an explicit builder data directory, matching subsequent windows. Originally manual
windows remain manual. A construction error stops startup. The pinned Tauri
configuration conversion drops the data-directory field, so setting that field
alone does not select the portable browser store.

On macOS, the pinned Wry runtime ignores `data_directory` when selecting a
persistent WKWebsiteDataStore. The native adapter creates one directory-backed
store before window construction and supplies it through `defaultDataStore`.
It uses the directory configuration initializer when available and the older
path setters otherwise. The store initializer is
`-_initWithConfiguration:`, which the
[WebKit private header](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebsiteDataStorePrivate.h)
declares from macOS 10.13. Runtime method checks and bounded native errors
prevent an unsupported installation from opening a shared default store.
An existing override is preserved. Explicit incognito window settings remain
unchanged.

Existing shared WebView history is retained in its original location. It is
not imported automatically, because it cannot be attributed to one portable
directory. A portable directory starts its own recent-profile and recovery
history. Move its `data` directory together with the executable.

`yarn test:portable-storage` compiles the production macOS adapter into owned
Cocoa helpers and exercises actual WebKit: invalid paths and threads, existing
overrides, repeated installation, two WebViews sharing a store, persistence
across processes, independent directories, and relocation of the app with its
data. Automatic API selection and the forced older-setter path run on the
host system. These focused checks do not establish older macOS runtime,
installer, concurrent-copy routing, or full desktop acceptance.

## Structured CLI input

The Rust host parses process arguments into a `LaunchRequest`. It never joins arguments into a shell command string.

Supported top-level options:

- `--profile <name-or-id>`
- `--cwd <path>`
- `--new-window`
- `--safe-mode`
- `--config <path>`
- `-- <executable> [arguments...]`

The existing Tabby commands remain accepted and are translated into the existing Angular `CLIEvent` shape:

- `open [directory]`
- `run [command...]` and `/k`
- `profile <profileName>`
- `paste [-e|--escape] [text...]`
- `recent <index>`
- `quickConnect <providerId> <query>`

The parser preserves every trailing command argument as a separate array item. Text containing shell operators such as `;`, `&&`, pipes, redirections, or command substitutions remains inert data until a later terminal feature explicitly starts a process with an argument array.

## Deep-link grammar

Supported links:

```text
tabby-rs://open?profile=<profile-id>
tabby-rs://open?cwd=<encoded-path>
tabby-rs://ssh/<encoded-profile-id>
tabby-rs://local?cwd=<encoded-path>
```

Deep links reject:

- unknown actions
- unknown or duplicate query parameters
- URL credentials, passwords, ports, and fragments
- incomplete or invalid percent escapes
- control characters
- excessive argument, scalar, or URL lengths
- conflicting values supplied by multiple inputs

Deep links intentionally cannot execute arbitrary commands.

## Single-instance routing

Portable processes select their communication identity from the configured
application identifier and the canonical `data` directory. Different portable
directories run independently. Launching the same physical directory again,
including through a directory symlink, forwards the request to its existing
process. Moving a directory after all its processes exit selects a new
communication identity while retaining its stored data. An inaccessible or
non-directory data path stops startup instead of using a shared identity.

Installed mode keeps the existing application identifier for single-instance
communication. The portable override changes only the native socket, Windows
mutex/window names, or Linux D-Bus name. The bundle identifier, credentials,
URL scheme, updater, and profile paths retain their existing values. The local
plugin patch and its source provenance are documented in
`src-tauri/vendor/tauri-plugin-single-instance/TABBY-RS-PATCHES.md`.

Windows second invocations query the existing process for structured-message
support before sending a JSON `[cwd, args]` frame. Each argument is preserved,
including literal pipes and empty strings. The same endpoint also accepts the
original NUL-terminated format within its declared byte length. With an older
running process, requests use that format when every value is representable.
A literal pipe causes startup to fail with restart guidance before delivery.
Rejected messages cause a nonzero startup result. The sender exits successfully
only after the receiver confirms that it handled the request. The protocol
keeps the existing mutex and window identity.

When that mutex already exists but the receiver window is not ready, the
launch retains its request and waits for the window or mutex ownership.
Only the owner may create the receiver and continue application setup.
If the previous owner releases the mutex or its owning thread ends, one
waiting launch acquires ownership and may initialize; other launches forward
to it. Native mutex or receiver creation errors stop setup. Teardown destroys
the receiver before releasing ownership.

The native host presents existing windows and queues a second invocation for the last-created surviving window, matching the fixed upstream behavior. If no window remains, it creates one first. Incoming requests enter a FIFO channel registered before plugins initialize. A single background receiver starts after application state is ready, preserving requests received during setup and their arrival order. Target selection and queue insertion run together on the main event thread, alongside window destruction; creating a replacement window runs on the receiver thread.

`app:launch` is a window-scoped notification with a null payload. The renderer installs its listener before draining `app.initialLaunch`; each call consumes one request for the invoking window, or returns null when its queue is empty. Reads are serialized, so repeated notifications cannot duplicate a request. The command name is retained for compatibility, but it also consumes subsequent requests.

The renderer holds received contexts until `HostAppService.emitReady()` so profile, directory, and other requests cannot run before Angular config and tab services are ready. It dispatches them through the existing priority-ordered `CLIHandler` list, preserving `secondInstance`. Only an unhandled second invocation reaches the original `LastCLIHandler` new-window fallback.

## Optional `tabby` alias

The canonical command is always `tabby-rs`.

The optional `tabby` alias is available only when:

1. the directory containing the running Tabby RS executable is already present on `PATH`; and
2. no other `tabby` executable, script, or shim exists anywhere on `PATH`.

On Unix, Tabby RS creates a symlink named `tabby`. On Windows, it creates a managed `tabby.cmd` shim beside `tabby-rs.exe`. Disabling the alias removes only an entry proven to be managed by Tabby RS. Existing commands are never replaced or deleted.

The alias status and conflict path are exposed in the **Settings → Tabby RS** page.

## Ownership boundaries

The Rust host owns the desktop window lifecycle. `--new-window` is transported through the launch contract, while the Tauri `window.new` command creates an additional renderer window with the same application entry point. Window state commands and desktop events are scoped to the invoking window, so moving or closing one window cannot mutate another window.

After focus, movement, or resizing, the renderer reads the full native window state. Only the latest requested state read can replace its cached snapshot. Bounds use outer window dimensions on macOS and Windows and client bounds on Linux. Linux movement events read the owning window's client origin rather than Tao's frame-position payload; resize events trigger a full snapshot refresh. `scripts/test-tauri-host-window-state.mjs` exercises the actual renderer service with simulated IPC, including delayed replies, failures, and independent windows. These checks do not establish native desktop acceptance.

Complete `window.applyState` patches are serialized by the host operation lock.
Frame, title, topmost, and fullscreen fields retain their order before bounds.
When bounds and an explicit maximized state arrive together, the host first
unmaximizes, applies the requested bounds, and then applies the requested final
maximized state before showing the window. On macOS, the asynchronous outer-size
helper reports the actual queued native result; the synchronous docking helper
retains its preflight-and-dispatch contract. Completion waits pass through the
Tauri event thread and, on macOS, its main GCD queue. Geometry and show requests
also drain earlier queued operations before starting, including work left by a
cancelled command. These dispatch checks do not establish compositor, fullscreen,
or supported-desktop acceptance.

The renderer returns a launch context to the native host when it requests a new
window. An omitted legacy command array decodes as an empty array, matching the
serialized form for profile, working-directory, and plain new-window requests.
Nonempty command arguments retain their separate array items.

When a launch request creates a window, its new-window flags and second-instance marker are consumed before that window reads the request. The initial main window also consumes those flags. The remaining arguments, working directory, and parse errors are preserved. A manual new window receives no copy of the original process arguments. Failed window creation and window destruction remove that window's pending requests. Window creation runs outside synchronous commands and native event handlers to avoid the documented WebView2 deadlock.

An approved window close marks its label as closing before requesting native destruction. Routing excludes it immediately, including the interval before the operating system reports `Destroyed`. If destruction fails, the label becomes eligible again and its renderer is notified to resume pending launches.

On macOS, the close controller claims that label before waiting for the shared
geometry lock and native operation barrier. It saves the cached normal bounds
with the live maximized flag, then destroys the window. Repeated pending requests
return without another save or destroy. A cancelled wait or failed destruction
releases the claim and wakes that label's queued launches. State read/write errors
are logged while approved destruction continues. The renderer's confirmation
path remains responsible for approval; a cancelled confirmation never invokes
this controller. The controller fixture simulates OS and state I/O, so complete
application/desktop close acceptance remains separate.

Normal application exit requests (`None` or exit code `0`) prevent exit while windows remain and request each window's existing close confirmation. A cancelled window retains its tabs; approved windows close through the same scoped renderer flow. With no windows, exit proceeds. Platform-level quit during startup sends the optional `force: true` request and exits without renderer confirmation, preserving the encrypted-configuration cancellation path. Restart and nonzero exit codes retain their original behavior. The host clears the crash marker only when the event loop actually exits.

On macOS, AppKit's `applicationShouldTerminate:` cancels native termination and posts an ordinary Tauri exit request. This brings Cmd+Q, application-menu Quit, and Dock Quit through the same window confirmation flow. Setup attaches a zero-ivar subclass to the existing Tao delegate on the main thread, retains the same object, and refreshes AppKit's delegate registration. Its layout and inherited callbacks remain intact. Installation fails if the delegate is missing, already has a termination policy, or changes after installation; it does not replace another delegate's policy.

`scripts/test-macos-native-quit.mjs` drives real Cocoa termination in separate helper processes, loading the production adapter and replacing only the AppHandle exit boundary with a recorder. It verifies repeated requests, original delegate identity, inherited callbacks/ivars, and installation errors. Both a complete evidence marker and normal process exit are required. The focused Cocoa probe explicitly skips other platforms; existing application-event and renderer fixtures cover forced exit, restart, zero windows, and confirmation decisions separately.

On macOS, stock primary and secondary windows start at 800×600 with a
400×300 outer minimum, matching the fixed upstream. The native geometry adapter
prepares the stock main window configuration before construction; secondary
creation uses the same defaults. Both initially use Overlay, where content and
outer dimensions coincide. Startup frame selection preserves that outer frame and
minimum. Explicit custom geometry, additional configured windows, creation
policies, and other window fields are preserved. Bootstrap, docking, reload,
and ready do not reinitialize default bounds. Saved window boundaries and other
platforms retain their separate pending acceptance requirements.

On Windows, stock primary and secondary windows are initialized to 800×600
outer logical pixels before renderer bootstrap can show them. Native initialization
uses the owning HWND's live DPI and preserves its chosen position, visibility,
activation, and maximized state. An independent, stateless window subclass applies
the 400×300 outer logical minimum after the existing native minimum calculation;
it preserves other constraints and survives Native/Thin/Full frame changes.
Apply-state and docking also read the live outer minimum after unmaximizing and
clamp each requested dimension before setting the outer frame. Tao consumes
`WM_WINDOWPOSCHANGING` without Win32's default size validation, so a direct
`SetWindowPos` otherwise accepts a size below that minimum.
Successful initialization is idempotent. Foreign subclass bindings are preserved,
and failed initialization prevents bootstrap rather than exposing a partly sized
window. Custom main-window geometry and construction policies are retained.

On Linux, stock primary and secondary windows start at 800×600 client logical
pixels with a 400×300 content minimum. The stock primary configuration is prepared
before construction; custom declarations retain their geometry and creation policy.
Stock windows are centered only when neither primary coordinate is configured;
explicit coordinates and custom declarations are retained. Native initialization
constrains the existing GTK content container, because GTK window-level minimum
hints can include Wayland client-side decorations. It selects Static gravity on
X11 and preserves size, visibility, frame selection, and subsequent user bounds.
The client-bounds adapter reads the owning GTK display rather than session
environment variables. On X11, native client coordinates and mapped dimensions
come from the live X window; hidden dimensions come from GTK's requested size.
Logical bounds enclose the physical rectangle, rounding the origin down and the
far edge up before returning the existing physical bridge values. Hidden position
writes update both GTK's request and its existing unmapped surface. Wayland keeps
the existing getter/setter behavior without advertising global positioning or
docking. The isolated Wry regressions check freshly constructed Native and
frameless windows on X11 and Wayland at scales one and two, including centering,
hidden moves/resizes, native client coordinates, and independent windows. These
checks do not accept physical resizing, full application behavior, original desktop
comparison, or installers.

The Windows and Linux bootstrap command waits asynchronously for the specific window's
construction result, then runs the existing config/plugin transaction on the UI
thread. Primary readiness is registered before Tauri construction and secondary
readiness before its builder runs. Destruction cancels pending initialization;
late completion cannot resolve a replacement window's registration. Bootstrap,
docking, reload, and ready wait for construction without resetting user bounds.
Concurrent CLI launch presentation also skips pending or failed initialization;
its queued launch delivery retains the existing ordering.
Real Win32 and Wry checks, fixed-original desktop comparisons, supported display
configurations, persistence, and installers retain their separate acceptance gates.

macOS and Linux use that registry for saved-frame restoration. Configured automatic
windows register before construction; requested windows register before their
builder runs. After restoring the frame, native move/resize events drain through
the operation barrier before capture is enabled and the ticket completes.
Bootstrap and concurrent launch presentation wait for that completion. The
native capture fixture exercises the actual restore/capture methods and tracker
with real Wry events; the AppState boundary is substituted. Full application
persistence and original desktop comparison remain pending.

Linux keeps the original automatic-window configuration before portable deferral,
restores each created window, then arms X11 first-event capture or Wayland
unchanged-startup suppression before completing its ticket. It preserves custom
minimums and explicit origins and does not create manual windows. Its first-page
hook follows the same claim/wait/close guards, without macOS appearance operations.
Linux emitReady only releases queued launches. Blank-page production application
and IPC probes verify hidden resize and combined unmaximize/resize/hide followed
by reload and approved-close saving on X11 and Wayland at scales one and two.
Full rendered desktop persistence and original desktop comparison remain pending.

macOS captures frame selection from the first successful raw construction-time
config read, before restoring saved bounds. Windows captures it from the first raw
bootstrap configuration. Both retain that snapshot for every window in the process.
Native uses the system title bar; macOS Thin and Full use hidden native chrome with
the shared renderer layout. Missing or unknown raw selections, including encrypted
wrappers without an appearance field, use Thin. Bootstrap returns the snapshot;
Windows applies it when its renderer is ready.

On macOS, the global finished-page hook claims one initialization ticket and keeps
its owning window before waiting. After restoration completes, presentation applies
raw vibrancy and colorSchemeMode, then shows and focuses the window, first maximizing
when its construction-time saved flag is true. Initial main-window hidden startup
suppresses maximization, showing, and focus even after its CLI request is consumed.
Other windows do not inherit that startup flag. Page reloads do not repeat these
operations. Renderer readiness releases queued launch requests without showing the
window. An approved close claimed during awaited appearance or zoom suppresses
subsequent showing and focus. Controller and filesystem checks cover these
boundaries separately. The owned blank Wry fixture executes the actual native
prepare/present methods for two windows with hidden startup and saved maximization
on/off, including normal bounds after unmaximizing. Its AppState/config I/O boundary
is substituted. An owned ad-hoc signed full application with a freshly compiled
renderer and isolated data also verifies a 700×450 saved normal window, saved
maximization, native unmaximize/remaximize, and ordinary Cmd+Q persistence of the
normal bounds and live maximized flag. These visible cases use an incognito
primary WebView and a welcome tab without a PTY. Hidden-startup UI and normal
quit acceptance, secondary windows, reload timing, original desktop comparison,
and installer acceptance remain pending. A UI observation timeout does not accept
hidden visibility.

On Windows, Thin and Full keep the decorated native host and expand its client area. Standard native BUTTON controls own caption input, accessibility, capture, and window actions. A single pointer-transparent canvas in the owning WebView paints the glyphs in the configured terminal foreground, with the pinned Electron/Chromium geometry and hover animation. The native adapter publishes physical button bounds, display scale, hover, pressed, and maximized state through the scoped caption event and window snapshot. Native mode and fullscreen clear the renderer overlay. The adapter retains the existing close-request confirmation path and the previous WebView focus.

A non-focusable native child receives the eight resize edges above the WebView. Its window region excludes the content center and all caption buttons, so nested browser windows cannot consume border input. It forwards the existing native nonclient resize messages to the owning root and hides while maximized or fullscreen. The same per-window registry validates its subclass state and releases its region and child during owned teardown.

The caption service subscribes before reading the initial snapshot, ignores stale replies, and releases its canvas, event listener, configuration subscription, and animation on destruction. The caption tests cover this lifecycle and real Chromium canvas pixels. The Windows native example exercises real Win32 controls and actual hit recipients over a nested opaque child window; complete Wry rendering, physical input, process restart, and supported display configurations require separate desktop evidence.

The shared restart action invokes `app.restart`, which requests a Tauri process restart. The framework restart environment retains only the executable argument, so consumed terminal commands and deep links are not replayed by manual or updater restarts. Initial launch parsing and single-instance delivery retain the original process arguments. Keep the managed restart environment in place before Tauri builds its default environment.

Safe-mode and configuration behavior remain owned by their respective milestones. Keeping those concerns out of the window builder preserves one launch contract without duplicating their implementations.
