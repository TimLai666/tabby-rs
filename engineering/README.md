# Engineering documents

Internal engineering documents for Tabby RS live here. Documentation written for
users and contributors (installation, migration, releases, plugin compatibility,
the release gate) stays in `docs/`.

- `architecture/` holds the design contracts that code must follow:
  `identity-and-launch.md` (application identity, single instance, launch
  requests, quit) and `data-storage-and-migration.md` (storage layout, journal,
  backups, conflict handling and rollback of the first-run import).
- Follow-ups, pending acceptance and approved parity exceptions are tracked in
  `AGENTS.md` at the repository root and in `docs/release-acceptance.md`; this
  project keeps no ticket or specification directories here yet. Create
  `specs/` and `tickets/` under this directory when that work starts.
- Design working files (brand specification, product facts) live in `design/`
  at the repository root; implementation plans go to `plans/`.
