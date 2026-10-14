# Tabby RS local patch

This directory contains russh-sftp 2.4.0 from its published crate archive.

- Archive SHA-256: 9de67aace74530a29086db0671fa200c470a58eb380081f28ad512ffb0c5356b
- Upstream source commit: e145c1f7ece99f41f558949ef59731f2cd1a9dfe
- Upstream license: Apache-2.0, retained in LICENSE
- Changed source: src/client/session.rs

The high-level session exposes posix_rename. It sends the OpenSSH extension only
when the server advertises version 1. The payload contains two SSH strings, and
success requires an SSH_FXP_STATUS reply with status OK. The method returns false
without sending a request when the extension is absent or has another version.
Server status errors and unexpected replies remain errors. Ordinary rename is
unchanged.

Tabby RS uses this method to commit explicit overwrite uploads after closing the
owned staging file. Without the supported extension, Tabby RS checks LSTAT and
refuses replacement of an existing entry. Only NoSuchFile permits ordinary
RENAME for a new destination. See the approved SFTP overwrite safety exception
in docs/release-acceptance.md. Skip and Rename uploads retain ordinary SFTP RENAME.

Before upgrading or removing this patch, run the SFTP protocol regressions in
src/ssh/sftp/overwrite_tests.rs and the isolated Unix OpenSSH integration tests.
Verify capability negotiation, binary and empty overwrites, close-before-commit,
failed replacement, cancellation, and isolation between concurrent sessions.
Remove the patch only when the replacement dependency provides equivalent behavior.

Protocol reference:
https://github.com/openssh/openssh-portable/blob/master/PROTOCOL#43-sftp-extension-request-posix-renameopensshcom
