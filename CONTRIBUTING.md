# Contributing to Tabby RS

Tabby RS is an unofficial Tauri/Rust fork of Tabby. Keep changes scoped to the fork and preserve the upstream attribution and license notices.

## Local setup

- Use Node.js 22 and Yarn 1.22.22 for the JavaScript workspace.
- Install the Rust stable toolchain and the Tauri platform dependencies for your host.
- Run `yarn install --frozen-lockfile` to install workspace dependencies.
- Run `yarn build:typings` before type checks or builds that resolve built-in packages.

Useful checks include:

```text
yarn lint --format unix
yarn test:tauri:types
yarn test:tauri:bundle
yarn test:release-documentation
yarn test:parity-report
yarn test:windows-caption-fixture
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

The release gate needs real bundle, installer, benchmark, license, and platform evidence. Do not change a parity manifest from `pending` to `passed` to make a local build green.

The controlled Windows frame driver requires a fresh painter script built with `node scripts/build-windows-caption-fixture.mjs --output <NEW-JS-FILE>`. Pass it with `--caption-script` to `cargo run --manifest-path src-tauri/Cargo.toml --example windows_frame_native --`; use `--help` for all required paths and cleanup ownership. The driver runs the production painter in an isolated WebView and preserves full normal/hover comparisons. Maximized caption comparisons use measured native hit regions when the capture coordinate spaces differ. These fixtures do not establish production desktop or Windows 11 Snap acceptance.

The reference requests Chromium language `EN` before readiness, matching the fixed upstream, and records its actual locale. The same driver compares offscreen GDI message-font text with the pinned English, dark TooltipAura captures. Its `gdi-tooltip-*-pixels` diagnostics record the mapped font, dimensions, independent reference bounds, and exact RGBA comparison, and write `gdi-tooltip-*.png`. These diagnostic candidates do not implement tooltip windows, timing, localization, or accessibility.

The `dwrite-unshaped-tooltip-*-dimensions` diagnostics use the existing Windows font-kit backend to compare natural message-font metrics with those complete reference popups. They record the selected font fingerprint, design metrics, per-glyph advances, font-smoothing query outcomes, and legacy glyph-pair adjustments. The pinned Electron manifest also records Canvas widths and bounds for controlled 12 px Segoe UI text with normal and disabled kerning requests, alongside the Canvas font property and actual kerning readback. These measurements do not verify TooltipAura glyph positions or rendered pixels.

For Chromium caption comparisons, use `node scripts/test-windows-caption-paint.mjs --gpu --browser-path <CHROMIUM> --reference-directory <NATIVE-EVIDENCE>`. The report records the actual graphics backend, and `--gpu` requires GPU compositing and rasterization. Run both default and GPU modes: software and GPU blending can produce different pixel values. Keep native WebView and production desktop acceptance separate from these browser checks.

## Change boundaries

- Keep user-facing behavior in the existing Angular services and put privileged host behavior behind the Tauri command boundary.
- Do not add Electron, a bundled Node.js runtime, telemetry SDKs, or native Node addons to the production bundle.
- Node.js is allowed for builds and for the explicit npm plugin-management workflow only.
- Never commit updater private keys, credentials, Vault contents, diagnostic archives, or generated local release evidence.

## Upstream fixes

When bringing a security or correctness fix from upstream, record the upstream commit in the change description, adapt it to the Tauri boundary, and run the relevant Rust, renderer, and release contract tests. Preserve a separate Tabby RS commit so the source and review history remain traceable.

Pull requests should explain the affected issue, the tests that ran, and any host or platform check that could not run locally.
