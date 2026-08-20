# Tabby RS release gate

`parity/features.yaml` and `parity/platform-matrix.yaml` are acceptance contracts, not status claims. Every entry needs evidence before it can change from `pending` to `passed` or a documented `accepted-difference`.

Each feature entry represents one user-facing capability and must identify its owning issue numbers, platform scope, and at least one automated or manual test. Each platform entry must identify its runner, compilation target, and non-empty `requiredChecks` list. These fields describe coverage responsibilities; they do not count as evidence until the named check actually runs and its result is recorded.

Collect the automated results for one platform with `node scripts/collect-parity-evidence.mjs --platform linux --arch x86_64 --target x86_64-unknown-linux-gnu --source-revision "$GITHUB_SHA" --output release-staging/parity-automated-evidence.json`. The collector selects the checks linked to that platform, runs each Yarn test sequentially, streams the output for CI review, and records only exit status, duration, byte counts, and SHA-256 hashes in the JSON artifact. A failed or missing check fails the collector. This artifact proves automated execution only. It does not change the `pending` status of a feature or platform and cannot replace the required manual evidence.

Release staging includes both views of the same parity result. Use `--output release-staging/parity-report.json` for automation and `--html-output release-staging/parity-report.html` for human review. The HTML renderer escapes all report values and does not change the gate result.

The gate also requires a passed parity report, bundle audit, installer smoke report, and generated license report. Release staging includes both `license-report.json` for automation and `license-report.html` for human review, plus `benchmarks/benchmark-report.json` aggregating the four individual benchmark reports. Run:

```text
node scripts/create-license-report.mjs
node scripts/smoke-tauri-release.mjs --staging release-staging --platform linux --output release-staging/installer-smoke.json
node scripts/check-release-gate.mjs --benchmarks-dir path/to/benchmarks --bundle-audit path/to/bundle-audit.json --license-report license-report.json --installer-smoke release-staging/installer-smoke.json --parity-report release-staging/parity-report.json --parity-evidence release-staging/parity-automated-evidence.json --manual-acceptance-dir release-staging/manual-acceptance --source-revision "$GITHUB_SHA" --platform linux --arch x86_64 --target x86_64-unknown-linux-gnu
```

The command always writes a machine-readable report. It exits non-zero when any feature, platform, bundle, license, benchmark, or provenance evidence is missing or failed. It checks that benchmark reports, bundle metadata, and the license report refer to the same source revision and matrix target, and that all benchmark reports refer to the same fixture and artifact hash. It does not invent benchmark numbers or convert an unrun manual check into a pass.

The automated evidence collector records platform-required checks that are not automated in `unverifiedRequiredChecks`, but does not fail solely because those checks are manual. The final gate still requires every feature and platform manifest entry to be `passed` or an evidenced `accepted-difference`, so manual checks remain release blockers until their evidence is recorded in the parity manifests.

Generate the four benchmark reports with the real Tauri executable and an external frame trace:

```text
node scripts/run-benchmarks.mjs \
  --binary path/to/tabby-rs \
  --binary-path path/to/tabby-rs \
  --target x86_64-unknown-linux-gnu \
  --output-command path/to/output-fixture \
  --output-args '["100000000"]' \
  --ui-frame-report path/to/frame-report.json \
  --bundle path/to/installed-bundle \
  --config-fixture-path path/to/config-fixture.json \
  --output-dir path/to/benchmarks
```

The runner hashes the configuration fixture and installed bundle into every report. It fails when the Tauri process does not write `TABBY_RS_BENCHMARK_READY_FILE`, when output samples differ, when output exceeds `--max-output-bytes`, or when the frame trace is missing. Fixture tests use a separate child process and are not release evidence.

Stable releases also run a read-only GitHub issue-state gate before the bundle job's release environment approval. It requires Epic #1 child issues `#2` through `#27` to be closed. Nightly releases skip this gate. An open child issue must remain a release blocker until the issue is actually closed.
