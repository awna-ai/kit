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
