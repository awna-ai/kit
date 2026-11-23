# Changelog: awna-keystore

This package is versioned on its own. Its releases are tagged `awna-keystore-vX.Y.Z` (for example `awna-keystore-v0.1.0`).

## 0.1.0 - 2026-11-23

- `Secret`, wiped on drop, redacted in `Debug`, and never cloned or displayed; `storable`, the one rule for which
  values every store accepts (token characters, at most `MAX_SECRET` bytes).
- `Item`: a service, an account and a label of its own, each checked against a fixed character set.
- The `CredentialStore` trait: find, exists, store (read back before it succeeds) and delete; errors that never hold
  a value or a store's own error text.
- `SecurityCli`, the macOS keychain through `security -i`, with the value on stdin only and the environment cleared
  to `HOME`; the Windows Credential Manager (kept on this computer) and the Linux Secret Service (each item with its
  own label) through keyring-core; `MemoryStore` for tests.
- `system()`, which picks this computer's store and never falls back to a file.
