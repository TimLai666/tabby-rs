#!/usr/bin/env bash
# Reproduces the linux-x64 bundle job of .github/workflows/release.yml on a Linux
# host or build container, then runs the release gate on the result. Every step
# writes its log under <staging>-work/logs/; keys and logs stay outside the audited
# staging directory, which is the evidence.
#
# Usage: scripts/acceptance/linux-release-gate.sh [phase...]
#   phases: build stage audit bench smoke parity gate   (default: all of them)
#
# Environment:
#   TABBY_RS_RELEASE_STAGING   staging directory (default: release-staging)
#   TABBY_RS_SOURCE_REVISION   revision recorded in every report (default: git HEAD)
#   TABBY_RS_RELEASE_CHANNEL   stable | nightly (default: stable)
#   TABBY_RS_RELEASE_VERSION   artifact version (default: src-tauri/Cargo.toml version)
#   CARGO_TARGET_DIR           honored for the bundle root (default: src-tauri/target)
#   TAURI_CLI_VERSION          pinned cargo-tauri version (default: 2.11.4)
#   TABBY_RS_BENCHMARK_SAMPLES benchmark samples (default: 20, as in CI)
#   TABBY_RS_PARITY_TIMEOUT    seconds before the parity collector is stopped (default: 7200)
#
# The updater key is an ephemeral evidence-only key generated here, like the
# workflow's evidence_only mode. Nothing produced by this script is publishable.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
STAGING=${TABBY_RS_RELEASE_STAGING:-release-staging}
STAGING=$(realpath -m "$STAGING")
WORK=$STAGING-work
LOGS=$WORK/logs
TARGET=x86_64-unknown-linux-gnu
PLATFORM=linux
ARCH=x86_64
BUNDLES=appimage,deb,rpm
export TAURI_CLI_VERSION=${TAURI_CLI_VERSION:-2.11.4}
export TABBY_RS_RELEASE_CHANNEL=${TABBY_RS_RELEASE_CHANNEL:-stable}
export TABBY_RS_SOURCE_REVISION=${TABBY_RS_SOURCE_REVISION:-$(git rev-parse HEAD)}
export GITHUB_SHA=$TABBY_RS_SOURCE_REVISION
export TABBY_RS_EVIDENCE_ONLY=true
export TABBY_RS_BUNDLE_TARGETS=$BUNDLES
export TABBY_RS_RELEASE_STAGING=$STAGING
export TABBY_RS_RELEASE_TARGET=$TARGET TABBY_RS_RELEASE_PLATFORM=$PLATFORM TABBY_RS_RELEASE_ARCH=$ARCH
export TABBY_RS_TOOLCHAIN=$TARGET
export TABBY_RS_UPDATE_PLATFORM=$PLATFORM TABBY_RS_UPDATE_ARCH=$ARCH
export TABBY_RS_UPDATE_ENDPOINT='https://evidence.invalid/tabby-rs/{{channel}}/{{version}}/{{platform}}/{{arch}}/{{artifact}}'
export TABBY_RS_UPDATE_ARTIFACT_URL=$TABBY_RS_UPDATE_ENDPOINT
export TABBY_RS_BUNDLE_ROOT=${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/$TARGET/release/bundle
BINARY=${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/$TARGET/release/tabby-rs
KEY_DIR=$WORK/evidence-signing
KEY=$KEY_DIR/tabby-rs-evidence-signing-key
mkdir -p "$LOGS" "$KEY_DIR"

if [ -z "${TABBY_RS_RELEASE_VERSION:-}" ]; then
  TABBY_RS_RELEASE_VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' src-tauri/Cargo.toml | head -1)
fi
export TABBY_RS_RELEASE_VERSION

if command -v yarn >/dev/null 2>&1; then YARN=yarn; else YARN="npm run"; fi

step() { # name command...
  local name=$1; shift
  echo "==> $name"
  { echo "# $(date -u +%FT%TZ) $*"; } >"$LOGS/$name.log"
  if "$@" >>"$LOGS/$name.log" 2>&1; then
    echo "    ok ($name)"
  else
    local code=$?
    echo "    FAILED ($name, exit $code): see $LOGS/$name.log"
    tail -n 40 "$LOGS/$name.log"
    return $code
  fi
}

load_public_key() {
  [ -f "$KEY.pub" ] || { echo "missing evidence signing key: run the build phase first" >&2; exit 1; }
  export TABBY_RS_UPDATE_PUBLIC_KEY
  TABBY_RS_UPDATE_PUBLIC_KEY=$(cat "$KEY.pub")
}

phase_build() {
  step restore-icons node scripts/restore-tauri-icons.mjs
  step build-typings $YARN build:typings
  step dependency-audit $YARN audit:tauri:dependencies --tauri-release --output "$STAGING/dependency-audit.json"
  step frontend $YARN build:tauri:frontend
  step branding $YARN test:tabby-rs-branding
  step telemetry-audit $YARN audit:telemetry
  if ! cargo tauri --version 2>/dev/null | grep -q "$TAURI_CLI_VERSION"; then
    step install-tauri-cli cargo install tauri-cli --version "$TAURI_CLI_VERSION" --locked
  fi
  step signer-generate cargo tauri signer generate --ci --password '' --write-keys "$KEY" --force
  load_public_key
  step release-config node scripts/create-tauri-release-config.mjs
  TAURI_SIGNING_PRIVATE_KEY=$KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD='' \
    step tauri-build cargo tauri build --target "$TARGET" --bundles "$BUNDLES" --config src-tauri/tauri.release.conf.json
}

phase_stage() {
  load_public_key
  step third-party-notices node scripts/generate-third-party-notices.mjs --output "$STAGING/THIRD_PARTY_NOTICES.md"
  TABBY_RS_LICENSE_PATH=LICENSE TABBY_RS_NOTICES_PATH=$STAGING/THIRD_PARTY_NOTICES.md \
    TABBY_RS_LICENSE_REPORT=$STAGING/license-report.json TABBY_RS_LICENSE_REPORT_HTML=$STAGING/license-report.html \
    step license-report node scripts/create-license-report.mjs
  step release-metadata node scripts/create-tauri-release-metadata.mjs
  step stage-artifacts node scripts/stage-tauri-release.mjs
  local update_artifact
  update_artifact=$(node scripts/assert-tauri-release-artifacts.mjs "$STAGING" --platform $PLATFORM --bundles "$BUNDLES" --output "$STAGING/artifact-contract.json")
  echo "    update artifact: $update_artifact"
  TABBY_RS_UPDATE_ARTIFACT=$update_artifact TABBY_RS_UPDATE_SIGNATURE=$update_artifact.sig \
    step update-manifest node scripts/create-tauri-update-manifest.mjs
  step verify-signature cargo run --quiet --manifest-path src-tauri/Cargo.toml --bin verify-tauri-signature -- \
    "$update_artifact" "$update_artifact.sig" "$STAGING/updater-public-key.txt"
  rm -rf "$WORK/release-benchmark-bundle"
  step snapshot-benchmark-bundle cp -R "$STAGING" "$WORK/release-benchmark-bundle"
}

# Audit, benchmark and smoke failures are recorded in their reports and logs;
# later phases still run so the release gate sees every input.
phase_audit() {
  step bundle-audit node scripts/check-tauri-bundle.mjs "$STAGING" --release --output "$STAGING/bundle-audit.json" || true
}

phase_bench() {
  mkdir -p "$STAGING/benchmarks"
  TABBY_RS_BENCHMARK_FRAME_REPORT=$STAGING/benchmarks/ui-frames.json \
    step benchmarks node scripts/run-benchmarks.mjs \
      --output-dir "$STAGING/benchmarks" \
      --samples "${TABBY_RS_BENCHMARK_SAMPLES:-20}" \
      --warmup-samples 1 \
      --binary xvfb-run \
      --binary-path "$BINARY" \
      --binary-args "[\"-a\",\"$BINARY\"]" \
      --platform $PLATFORM --arch $ARCH --target $TARGET \
      --output-command node \
      --output-args '["scripts/fixtures/benchmark-process.mjs","--output","104857600"]' \
      --minimum-output-bytes 104857600 \
      --bundle "$WORK/release-benchmark-bundle" \
      --ui-frame-report "$STAGING/benchmarks/ui-frames.json" \
      --config-fixture-path scripts/fixtures/benchmark-config.json \
      --config-fixture release-default || true
}

phase_smoke() {
  step installer-smoke xvfb-run -a node scripts/smoke-tauri-release.mjs --staging "$STAGING" --platform $PLATFORM --output "$STAGING/installer-smoke.json" || true
}

# Release variables exported above would leak into the test fixtures
# (benchmark-harness reads TABBY_RS_TOOLCHAIN), so parity runs without them.
RELEASE_VARS=(TABBY_RS_RELEASE_CHANNEL TABBY_RS_EVIDENCE_ONLY TABBY_RS_BUNDLE_TARGETS TABBY_RS_RELEASE_STAGING
  TABBY_RS_RELEASE_TARGET TABBY_RS_RELEASE_PLATFORM TABBY_RS_RELEASE_ARCH TABBY_RS_TOOLCHAIN TABBY_RS_UPDATE_PLATFORM
  TABBY_RS_UPDATE_ARCH TABBY_RS_UPDATE_ENDPOINT TABBY_RS_UPDATE_ARTIFACT_URL TABBY_RS_BUNDLE_ROOT TABBY_RS_RELEASE_VERSION
  TABBY_RS_UPDATE_PUBLIC_KEY GITHUB_SHA TABBY_RS_SOURCE_REVISION)
clean_env() { local unset_args=(); for v in "${RELEASE_VARS[@]}"; do unset_args+=(-u "$v"); done; env "${unset_args[@]}" "$@"; }

phase_parity() {
  step parity-links clean_env $YARN test:parity-links
  step parity-evidence clean_env timeout "${TABBY_RS_PARITY_TIMEOUT:-7200}" node scripts/collect-parity-evidence.mjs --platform $PLATFORM --arch $ARCH --target $TARGET \
    --source-revision "$TABBY_RS_SOURCE_REVISION" --output "$STAGING/parity-automated-evidence.json"
}

phase_gate() {
  step parity-report node scripts/compare-parity.mjs --report-only --evidence-root "$STAGING" \
    --output "$STAGING/parity-report.json" --html-output "$STAGING/parity-report.html" || true
  if [ -f "$STAGING/manual-acceptance/linux-x64.json" ]; then
    step manual-acceptance node scripts/check-manual-platform-acceptance.mjs \
      --record "$STAGING/manual-acceptance/linux-x64.json" --evidence-root "$STAGING" \
      --source-revision "$TABBY_RS_SOURCE_REVISION" --architecture $ARCH --target $TARGET || true
  fi
  # The gate is expected to fail while manifest entries are pending; its report is the evidence.
  step release-gate node scripts/check-release-gate.mjs --evidence-root "$STAGING" \
    --bundle-audit "$STAGING/bundle-audit.json" --dependency-audit "$STAGING/dependency-audit.json" \
    --license-report "$STAGING/license-report.json" --installer-smoke "$STAGING/installer-smoke.json" \
    --parity-report "$STAGING/parity-report.json" --parity-evidence "$STAGING/parity-automated-evidence.json" \
    --manual-acceptance-dir "$STAGING/manual-acceptance" --benchmarks-dir "$STAGING/benchmarks" \
    --source-revision "$TABBY_RS_SOURCE_REVISION" --platform $PLATFORM --arch $ARCH --target $TARGET \
    --output "$STAGING/release-gate.json" || true
  step release-checksums node scripts/create-release-checksums.mjs "$STAGING"
}

PHASES=${*:-build stage audit bench smoke parity gate}
for phase in $PHASES; do
  echo "#### phase $phase ($(date -u +%FT%TZ))"
  "phase_$phase"
done
echo "#### done; staging at $STAGING"
