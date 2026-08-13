# Tabby RS release gate

`parity/features.yaml` and `parity/platform-matrix.yaml` are acceptance contracts, not status claims. Every entry needs evidence before it can change from `pending` to `passed` or a documented `accepted-difference`.

Release staging includes both views of the same parity result. Use `--output release-staging/parity-report.json` for automation and `--html-output release-staging/parity-report.html` for human review. The HTML renderer escapes all report values and does not change the gate result.

The gate also requires a passed bundle audit, installer smoke report, and generated license report. Run:

```text
node scripts/create-license-report.mjs
node scripts/smoke-tauri-release.mjs --staging release-staging --platform linux --output release-staging/installer-smoke.json
node scripts/check-release-gate.mjs --benchmarks-dir path/to/benchmarks --bundle-audit path/to/bundle-audit.json --license-report license-report.json --installer-smoke release-staging/installer-smoke.json --source-revision "$GITHUB_SHA" --platform linux --arch x86_64 --target x86_64-unknown-linux-gnu
```

The command always writes a machine-readable report. It exits non-zero when any feature, platform, bundle, license, benchmark, or provenance evidence is missing or failed. It checks that benchmark reports, bundle metadata, and the license report refer to the same source revision and matrix target, and that all benchmark reports refer to the same fixture and artifact hash. It does not invent benchmark numbers or convert an unrun manual check into a pass.

Generate the four benchmark reports with the real Tauri executable and an external frame trace:

```text
node scripts/run-benchmarks.mjs \
  --binary path/to/tabby-rs \
  --binary-args '["--benchmark"]' \
  --output-command path/to/output-fixture \
  --output-args '["100000000"]' \
  --ui-frame-report path/to/frame-report.json \
  --bundle path/to/installed-bundle \
  --config-fixture-path path/to/config-fixture.json \
  --output-dir path/to/benchmarks
```

The runner hashes the configuration fixture and installed bundle into every report. It fails when the Tauri process does not write `TABBY_RS_BENCHMARK_READY_FILE`, when output samples differ, when output exceeds `--max-output-bytes`, or when the frame trace is missing. Fixture tests use a separate child process and are not release evidence.
