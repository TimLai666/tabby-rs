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

Installed mode uses the Tauri application data directory for `io.tabbyrs.app`.

Portable mode is selected before any config, plugin, log, or credential-backed service is initialized. It is enabled when either of these entries exists beside the executable:

- `.tabby-rs-portable`
- `data/`, retained as a compatibility marker for the original portable convention

In portable mode all file-backed state is rooted at `<executable-directory>/data`.

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

After focus, movement, or resizing, the renderer reads the full native window state. Only the latest requested state read can replace its cached snapshot. Bounds come from that snapshot's outer window dimensions; resize event payloads describe the client area. `scripts/test-tauri-host-window-state.mjs` exercises the actual renderer service with simulated IPC, including delayed replies, failures, and independent windows. These checks do not establish native desktop acceptance.

`window.getCaptionState` reads the invoking window's prepared Windows caption snapshot. It returns null when no controller is prepared or the host is not Windows, and rejects native query failures. The query never prepares controls. `desktop:windowCaptionControls` carries the same snapshot through that window's scoped listener. A renderer consumer must await subscription before its initial query and retain the highest revision. This transport does not enable Windows frame settings or paint controls.

When a launch request creates a window, its new-window flags and second-instance marker are consumed before that window reads the request. The initial main window also consumes those flags. The remaining arguments, working directory, and parse errors are preserved. A manual new window receives no copy of the original process arguments. Failed window creation and window destruction remove that window's pending requests. Window creation runs outside synchronous commands and native event handlers to avoid the documented WebView2 deadlock.

An approved window close marks its label as closing before requesting native destruction. Routing excludes it immediately, including the interval before the operating system reports `Destroyed`. If destruction fails, the label becomes eligible again and its renderer is notified to resume pending launches.

Normal application exit requests (`None` or exit code `0`) prevent exit while windows remain and request each window's existing close confirmation. A cancelled window retains its tabs; approved windows close through the same scoped renderer flow. With no windows, exit proceeds. Platform-level quit during startup sends the optional `force: true` request and exits without renderer confirmation, preserving the encrypted-configuration cancellation path. Restart and nonzero exit codes retain their original behavior. The host clears the crash marker only when the event loop actually exits.

macOS frame selection is captured from the first raw bootstrap configuration and retained for every window in that process. Native uses the system title bar; Thin and Full use hidden native chrome with the shared renderer layout. Missing or unknown raw selections, including encrypted wrappers without an appearance field, use Thin. The renderer receives the snapshot through bootstrap data and applies it before showing its owning window.

The shared restart action invokes `app.restart`, which requests a Tauri process restart. The framework restart environment retains only the executable argument, so consumed terminal commands and deep links are not replayed by manual or updater restarts. Initial launch parsing and single-instance delivery retain the original process arguments. Keep the managed restart environment in place before Tauri builds its default environment.

Safe-mode and configuration behavior remain owned by their respective milestones. Keeping those concerns out of the window builder preserves one launch contract without duplicating their implementations.
