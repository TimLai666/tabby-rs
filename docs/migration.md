# Migrating from Tabby

Tabby RS imports selected configuration from known Tabby and historical Terminus locations into its independent data directory. The import is one-way: it never deletes or rewrites the original Tabby data.

Before importing, close the source application and keep a backup. Tabby RS shows discovered profiles, plugin package names, and secret-bearing field paths without displaying secret values. Select the configuration and plugin names to import, then review the generated report.

The migration copies `config.yaml` byte-for-byte after checking its source revision. If the source changes during the operation, the import fails and the pre-import snapshot is retained for recovery. Plugin executable code and secret values are not copied by this first-run flow.

The detailed storage, journal, backup, conflict, and rollback contract is in [data-storage-and-migration.md](architecture/data-storage-and-migration.md). Vault v1 and operating-system keychain transfer require separate compatibility handling and must not be assumed from a successful configuration import.
