# Tabby RS release gate

`parity/features.yaml` and `parity/platform-matrix.yaml` are acceptance contracts, not status claims. Every entry needs evidence before it can change from `pending` to `passed` or a documented `accepted-difference`.

The gate also requires a passed bundle audit and a generated license report. Run:

```text
node scripts/create-license-report.mjs
node scripts/check-release-gate.mjs --bundle-audit path/to/bundle-audit.json --license-report license-report.json
```

The command always writes a machine-readable report. It exits non-zero when any feature, platform, bundle, or license evidence is missing or failed. It does not invent benchmark numbers or convert an unrun manual check into a pass.
