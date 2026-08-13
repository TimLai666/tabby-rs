# Contributing to Tabby RS

Tabby RS is an unofficial Tauri/Rust fork of Tabby. Keep changes scoped to the fork and preserve the upstream attribution and license notices.

## Local setup

- Use Node.js 22 and Yarn 1.22.22 for the JavaScript workspace.
- Install the Rust stable toolchain and the Tauri platform dependencies for your host.
- Run `yarn install --frozen-lockfile` before running checks.

Useful checks include:

```text
yarn lint --format unix
yarn test:tauri:bundle
yarn test:release-documentation
yarn test:parity-report
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

The release gate needs real bundle, installer, benchmark, license, and platform evidence. Do not change a parity manifest from `pending` to `passed` to make a local build green.

## Change boundaries

- Keep user-facing behavior in the existing Angular services and put privileged host behavior behind the Tauri command boundary.
- Do not add Electron, a bundled Node.js runtime, telemetry SDKs, or native Node addons to the production bundle.
- Node.js is allowed for builds and for the explicit npm plugin-management workflow only.
- Never commit updater private keys, credentials, Vault contents, diagnostic archives, or generated local release evidence.

## Upstream fixes

When bringing a security or correctness fix from upstream, record the upstream commit in the change description, adapt it to the Tauri boundary, and run the relevant Rust, renderer, and release contract tests. Preserve a separate Tabby RS commit so the source and review history remain traceable.

Pull requests should explain the affected issue, the tests that ran, and any host or platform check that could not run locally.
