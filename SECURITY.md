# Security policy

Tabby RS is an unofficial fork and is maintained separately from upstream Tabby. Do not include passwords, private keys, Vault files, raw terminal scrollback, diagnostic archives, or updater signing material in a report.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting flow for this repository when it is available. Do not open a public issue for an unpatched vulnerability. If private reporting is unavailable, contact the repository maintainers privately and include only the minimum reproducible details needed to validate the report.

Useful details include the affected release or commit, operating system and architecture, reproduction steps without secrets, impact, and a safe mitigation if known.

## Security boundaries

- Release bundles must not contain Electron, a Node.js runtime, forbidden native Node addons, Sentry, or Mixpanel runtime content.
- Updater artifacts require the configured updater signature and hash checks before installation.
- npm plugin management is opt-in and isolated from the application bundle. Plugins that require Electron, Node runtime APIs, native addons, or private Tabby internals are outside the supported compatibility contract.
- Vault v1 remains a compatibility format and does not provide authenticated encryption. Do not treat it as a modern integrity-protected secret store.
- Unsigned OS installers may trigger SmartScreen, Gatekeeper, enterprise policy, or Linux package warnings. Verify checksums and source before using any manual override.
