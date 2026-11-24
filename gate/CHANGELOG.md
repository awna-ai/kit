# Changelog: awna-gate

This package is versioned on its own. Its releases are tagged `awna-gate-vX.Y.Z` (for example `awna-gate-v0.1.0`).

## 0.1.0

The first release.

- `lib.sh`, a sourced library for Bash 3.2 and later that leaves the caller's options and traps alone: `gate_init`
  parses the consumer's arguments (`--tier full|land|touched`, `--list`, `--check-map`) and records the repository
  root; `gate_require` and `gate_missing` skip and count a missing tool, or exit 75 in strict mode; `gate_step`
  registers a command as plain, `--test` or `--contract`, optionally `--mac-only` or `--always`; `gate_run` runs each
  step from the root with its own log, prints its timing, the last lines of a failed log and the slowest five, and
  exits 0 green, 1 red, 2 on an internal error or 75 when this host cannot run the gate.
- The touched tier: a map of path patterns to step patterns selects the steps a change needs, measured from the merge
  base with the base ref through the working tree and untracked files; anything it cannot be sure of (no merge base,
  an unmapped path, a path it cannot match, a change to the gate's own files) runs the full tier. `--check-map`
  checks the map against the registered steps and the tracked files.
- `gate_workflow` reads single-line `run:` steps from GitHub Actions workflows, job by job, following local reusable
  workflows, and refuses with exit 2 anything it cannot mirror; the consumer allows jobs, job keys, environment
  variables, services and setup steps through hooks. Workflow steps run with `bash -e` in a scrubbed environment, and
  a failed step skips the rest of its job.
- `term-check.sh` checks every file, path, object, commit and tag message and ref name in a repository, or one
  message file, against a term list the caller names; `secret-scan.sh` runs gitleaks over every ref's history, every
  reachable object and the working tree, and refuses a repository-level gitleaks configuration. Both take the
  repository and the strict switch as arguments.
- `vendor.sh` copies the package as it is in a release tag into a consumer's tree, with a `PIN` file naming the tag
  and its git tree. `gate_run` checks a pinned copy before any step and fails on any drift.
