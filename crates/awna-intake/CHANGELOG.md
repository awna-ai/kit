# Changelog: awna-intake

This package is versioned on its own. Its releases are tagged `awna-intake-vX.Y.Z` (for example `awna-intake-v0.1.0`).

## 0.1.0 - 2026-11-25

- Allow `video/mp4` (`mp4`, `m4v`) and `video/quicktime` (`mov`) attachments by default.
- Add intake item types and validated JSON and text boundaries.
- Add the status machine, with every illegal move refused.
- Add attachment limits and a content-type and extension allowlist.
- Add `Store`, `MemoryStore` and the reusable conformance suite.
- Add `Processor` and `process` for validated processing without I/O in the crate.
- Define CONTRACT v1.
- Add `serde` (with `derive`) and `serde_json` as the only direct dependencies.
