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

The single-instance plugin is registered before other Tauri plugins. A second invocation is converted into the same `LaunchContext` used for the initial process, then sent to the existing window through the `app.launch` event.

The renderer queues launch contexts until `HostAppService.emitReady()` so profile, directory, and other requests cannot run before Angular config and tab services are ready. It then dispatches them through the existing priority-ordered `CLIHandler` list.

## Optional `tabby` alias

The canonical command is always `tabby-rs`.

The optional `tabby` alias is available only when:

1. the directory containing the running Tabby RS executable is already present on `PATH`; and
2. no other `tabby` executable, script, or shim exists anywhere on `PATH`.

On Unix, Tabby RS creates a symlink named `tabby`. On Windows, it creates a managed `tabby.cmd` shim beside `tabby-rs.exe`. Disabling the alias removes only an entry proven to be managed by Tabby RS. Existing commands are never replaced or deleted.

The alias status and conflict path are exposed in the **Settings → Tabby RS** page.

## Ownership boundaries

This foundation only parses and transports `--new-window`, `--safe-mode`, and `--config`. Their full behavior belongs to the desktop-window, safe-mode, and configuration milestones respectively. This keeps the launch contract stable without duplicating those implementations here.
