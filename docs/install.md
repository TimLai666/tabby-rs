# Installing Tabby RS

Tabby RS is a non-official fork of Tabby. It uses a separate application identifier, data directory, URL scheme, and package identity so it can be installed beside the upstream Tabby application.

Release artifacts are provided for Windows NSIS, macOS DMG on Intel and Apple Silicon, and Linux AppImage, DEB, and RPM when the corresponding release matrix passes. Do not infer support for another operating system or architecture from a similarly named file.

The installer packages are not signed with Windows Authenticode or an Apple Developer ID certificate. Windows SmartScreen or macOS Gatekeeper may warn before installation. Follow the operating system's documented, user-visible override flow only after verifying the release checksum and source.

Tabby RS does not include a Node.js runtime in its application bundle. Node.js is only used by the build and by the explicit npm plugin-management workflow.
