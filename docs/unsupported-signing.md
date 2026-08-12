# Signing limitations

Tabby RS updater packages use the Tauri updater signature and the embedded updater public key to authenticate update artifacts. This does not provide operating-system trust.

The project does not perform Windows Authenticode, Microsoft Store signing, macOS Developer ID signing, notarization, or Mac App Store signing. SmartScreen, Gatekeeper, enterprise policy, or Linux package policy can therefore block or warn about an otherwise valid updater artifact.

Do not disable those protections through a script. Verify the release checksum, inspect the release source, and use the operating system's documented manual approval flow when the risk is acceptable.
