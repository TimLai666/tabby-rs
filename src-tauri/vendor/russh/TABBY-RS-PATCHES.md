# Tabby RS russh patch

This directory contains the published `russh` 0.54.4 crate, under Apache-2.0.
The crate archive SHA-256 is `54b1296a0e9d970321e59d77367b8d1511311445f3dd5d42f24e7d9a1706cf51`.
Its upstream source commit is `0ed1195025f4d2c72a35c241aaa29dfcd793276d`.
The upstream repository is https://github.com/Eugeny/russh.
`LICENSE.txt` is retained from that source commit. Cargo's `.cargo-ok` cache marker is omitted.

The only source modification is in `src/client/encrypted.rs`: while waiting for a
keyboard-interactive answer, accept a new `Msg::Authenticate` through the existing
`write_auth_request_if_needed` method. This replaces the pending authentication
request and allows the next candidate to reach the server without submitting an
answer to the dismissed challenge. Normal challenge answers retain their existing
path. This patch does not add a prompt-cancellation UI.

When updating russh, compare this change with the new upstream state machine and
run the keyboard-interactive cancellation unit and real SSH regression tests.
Remove the local Cargo patch when the same behavior is verified in the replacement
upstream version. Do not carry the change forward solely by version number.
