# Changelog: awna-agents

This package is versioned on its own. Its releases are tagged `awna-agents-vX.Y.Z` (for example `awna-agents-v0.1.0`).

## 0.1.0

- `print`: build print-mode arguments, and reject prompts or tool names that look like flags, invalid session IDs,
  and invalid budgets.
- `print::stream`: summarize print-mode `stream-json` and `json` output, including results, usage, and rate limits.
- `exec`: summarize exec-mode JSON Lines events, including the reply, error category, and accumulated token counts.
- Add `serde_json` as the sole direct dependency.
