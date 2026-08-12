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
- plugin names pending reinstallation.

Unknown top-level fields are retained when the state is loaded and saved. A state schema newer than the current reader is rejected.

Before an updater install, Rust writes `pending-update.json` atomically after creating the `before-update` backup. It contains only the target version, backup id, and channel. Startup reads this journal independently from `tabby-rs.json`: if the installed version matches and the configuration or state is unreadable, it restores the recorded backup before clearing the journal. A successful startup also clears the journal. A failed install clears it before returning control to the current version.

## Backups

A backup is a directory with a JSON manifest, not an archive. The manifest records:

- backup identity, timestamp, reason, source app version, and update channel;
- every stored file's relative name, size, and SHA-256 checksum;
- managed files that did not exist at snapshot time.

Restore accepts only the fixed managed names `config.yaml` and `tabby-rs.json`. It validates all files before making changes, creates a safety backup first, atomically replaces stored files, and removes files that were absent in the selected snapshot. Paths from a manifest can never escape the managed data root.

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

The Tauri updater API currently returns the verified artifact as an in-memory byte buffer before installation. The host rejects oversized downloads during progress reporting and verifies the declared size and hash, but does not claim resumable or disk-streamed downloads until the updater transport supports them.

## Deferred secret migration

This issue identifies secret-bearing paths and reports which fields need review. Reading the original Vault or operating-system keychain and transferring secrets belongs to the dedicated Vault migration issue. Plain secret values must never appear in the migration UI, journal, report, or logs.
