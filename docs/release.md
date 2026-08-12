# Tabby RS releases

Release builds run from the protected release workflow, not from pull requests. The workflow builds each platform in a separate matrix job, produces the Tauri package and updater artifact, prepares license and identity metadata, runs the bundle audit, and uploads the result for the release job.

Stable and Nightly use separate updater channels and manifests. Stable releases come from protected `v*` tags or an approved manual run. Nightly releases are marked prerelease and must not overwrite Stable assets.

Each matrix job must receive the environment variable `TABBY_RS_UPDATE_ARTIFACT_URL`. It is an HTTPS template served by the configured channel endpoint for that platform's signed updater artifact. It may use `{{channel}}`, `{{version}}`, `{{platform}}`, `{{arch}}`, and `{{artifact}}`. The workflow derives `update-manifest.json` from the platform's primary updater artifact and its `.sig`, including the version, `pub_date`, channel, platform, architecture, size, and SHA-256. Linux still publishes AppImage, DEB, and RPM packages; AppImage is the updater artifact while every generated `.sig` remains in the release staging. A missing URL, primary artifact, or matching signature fails the release job.

The updater public key is compiled into the release configuration. The private key is read only from the release environment secret used by the Tauri signing step. It must never be committed, printed, uploaded, or copied into an application bundle.

Every release report records the source revision, channel, version, bundle file list, file sizes, SHA-256 values, dependency notices, and whether OS code signing was performed.
