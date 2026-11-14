# Tabby RS · Product Facts

> Verified: 2026-09-06

- The repository identifies the Rust icon directory as containing provisional Tabby RS development icons derived from the existing Tabby logo: `src-tauri/icons/README.md`.
- Tauri bundle configuration consumes `icons/icon.png`, `icons/icon.ico`, `icons/icon.icns`, and the configured legacy macOS mirror `../build/mac/icon.icns`: `src-tauri/tauri.conf.json`.
- The build script materializes `icon.png` and `icon.ico` from their tracked `.b64` sources: `src-tauri/build.rs`.
- Public search did not establish a distinct product named “TAPRAS”; this task therefore treats “TAPRAS” as the user’s reference to the repository’s Tabby RS Rust variant.
- The official Rust artwork repository is separate from this repo-specific mark: https://github.com/rust-lang/rust-artwork.
