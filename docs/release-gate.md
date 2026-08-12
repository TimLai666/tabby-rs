# Tabby RS release gate

`parity/features.yaml` and `parity/platform-matrix.yaml` are acceptance contracts, not status claims. Every entry needs evidence before it can change from `pending` to `passed` or a documented `accepted-difference`.

The gate also requires a passed bundle audit and a generated license report. Run:

```text
node scripts/create-license-report.mjs
node scripts/check-release-gate.mjs --benchmarks-dir path/to/benchmarks --bundle-audit path/to/bundle-audit.json --license-report license-report.json
```

The command always writes a machine-readable report. It exits non-zero when any feature, platform, bundle, or license evidence is missing or failed. It does not invent benchmark numbers or convert an unrun manual check into a pass.

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
