# Changelog

## Unreleased

### Added

- `gate_init` records the repository root and parses the gate's name, strict variable and consumer arguments.
- `gate_missing` reports and counts a missing host capability, or exits 75 in strict mode.
- `gate_require` checks whether a tool is available before a consumer registers its steps.
- `gate_step` registers a command with its label, result kind and optional macOS requirement.
- `gate_run` runs the steps with logs and timings, reports the slowest five and exits with the gate's contract.
