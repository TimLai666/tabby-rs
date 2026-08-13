# Tabby RS releases

Release builds run from the protected release workflow, not from pull requests. The workflow builds each platform in a separate matrix job, produces the Tauri package and updater artifact, prepares license and identity metadata, runs the bundle audit, and uploads the result for the release job.

Stable and Nightly use separate updater channels and manifests. Stable releases come from protected `v*` tags or an approved manual run. Nightly releases are marked prerelease and must not overwrite Stable assets.

Each matrix job must receive the environment variable `TABBY_RS_UPDATE_ARTIFACT_URL`. It is an HTTPS template served by the configured channel endpoint for that platform's signed updater artifact. It may use `{{channel}}`, `{{version}}`, `{{platform}}`, `{{arch}}`, and `{{artifact}}`. The workflow derives `update-manifest.json` from the platform's primary updater artifact and its `.sig`, including the version, `pub_date`, channel, platform, architecture, size, and SHA-256. Linux still publishes AppImage, DEB, and RPM packages; AppImage is the updater artifact while every generated `.sig` remains in the release staging. A missing URL, primary artifact, or matching signature fails the release job.

Before generating the manifest, the workflow checks every bundle type declared by the platform matrix. It also requires exactly one platform-specific primary updater artifact and its adjacent `.sig`, so a Linux release cannot pass with AppImage alone when DEB or RPM was requested.

The Tauri release dependency audit records its policy and explicit exclusions. Root development or peer dependencies and the legacy Electron/native entries in `app/package.json` are not shipped by the Tauri entry; the generated bundle audit remains authoritative for the installed renderer and fails on forbidden runtime content.

The updater public key is compiled into the release configuration. The private key is read only from the release environment secret used by the Tauri signing step. It must never be committed, printed, uploaded, or copied into an application bundle.

Update recovery is crash-safe at the application-data boundary: the updater writes an atomic `pending-update.json` journal beside `tabby-rs.json` after creating the pre-update backup. The next startup uses that journal to restore the recorded backup when the newly installed configuration or state cannot be read, then removes the journal after recovery or a successful startup. The current Tauri updater transport buffers the complete response during download; after verification, Tabby RS keeps the artifact in a private temporary file until installation and aborts downloads that exceed the configured limit. Resumable or streamed transport remains outside this release contract.

Every release report records the source revision, channel, version, bundle file list, file sizes, SHA-256 values, dependency notices, and whether OS code signing was performed. Each staged release also contains `license-report.json`, its escaped human-readable `license-report.html` view, and `benchmarks/benchmark-report.json` alongside the four validated metric reports. `SHA256SUMS` is generated after the platform release gate and covers every other staged file with a stable relative path and SHA-256 digest.
