# Changelog

## Unreleased

### Added

- `gate_init --map` opts a consumer into map-based selection and validation.
- `gate_init --base` chooses the local ref used to measure changes.
- `--tier touched` selects steps for working-tree and committed changes, falling back to the full tier on uncertainty.
- `--check-map` validates map entries against registered steps and tracked paths without running steps.
- `gate_step --always` keeps repository-wide checks in every touched run.
- `gate_init` records the repository root and parses the gate's name, strict variable and consumer arguments.
- `gate_missing` reports and counts a missing host capability, or exits 75 in strict mode.
- `gate_require` checks whether a tool is available before a consumer registers its steps.
- `gate_step` registers a command with its label, result kind and optional macOS requirement.
- `gate_run` runs the steps with logs and timings, reports the slowest five and exits with the gate's contract.

### Fixed

- Resolve the base to one full commit ref, refusing ambiguous names, and compare from its merge base.
- Include submodule changes despite ignore settings, use repository-relative paths and require the physical repository top.
- Select file names containing spaces from labels and require `/` or `.` in individual path words.
- Always run the full tier for changes to the map, consumer script or sourced library, even under catch-all map rows.
- Replace non-printable map diagnostic bytes with `?` and refuse `nocasematch` without changing consumer options.
- Document plain map globs and the consumer's `extglob` setting, working-tree limits and selection time for large changes.
- Document that a signal can leave one 0600 selection file under `TMPDIR` between creation and removal.
- Refuse map validation with exit 75 when a host tool is missing, in strict and non-strict mode.
