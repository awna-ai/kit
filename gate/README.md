# awna-gate

A sourced Bash library for a repository's local gate. The consumer probes its tools,
registers ordered steps and calls `gate_run`. This package needs Bash 3.2 or later;
it supports both Bash 3.2 and Bash 5. It adds no dependencies to the workspace.

Source `gate/lib.sh` from a location relative to the consumer script. Sourcing is
silent, defines functions and a private guard, and is harmless when repeated. It does
not change shell options or traps. Consumers may use `set -euo pipefail` and an `ERR`
trap, or neither. Use `gate_require` in a condition because absence returns 1.

```bash
#!/usr/bin/env bash
set -euo pipefail
trap 'exit 2' ERR
cd "$(dirname "$0")/.."
# shellcheck source=gate/lib.sh
. gate/lib.sh
gate_init --name local-check --strict-var CHECK_STRICT -- "$@"
if gate_require shellcheck 'the shell checks'; then
    gate_step 'shellcheck scripts/*.sh'
fi
gate_step --test 'bash tests/example_test.sh'
gate_step --contract 'bash scripts/example-check.sh'
gate_run
```

Commands and labels must come from the consumer's own script. They are trusted shell
code, never external input. Commands run with `bash -c`, without `eval`. A step
inherits the consumer's environment, but not its unexported shell variables or functions.

The host supplies `bash`, `mktemp`, `rm` and `tail`; macOS-only steps also need `uname`.
The library uses the shell's `pwd` builtin and `SECONDS` clock. Logs can contain any
output the commands produce. Failed runs retain those logs in a temporary directory;
the consumer is responsible for removing them after inspection.

## Public functions

### gate_init

`gate_init --name <name> [--strict-var <VAR>] [--] [<consumer arguments>...]`

Call once before the other functions. It records `pwd -P` as the root where every
step will run, even if the consumer later changes directory. It returns 0 on success.

The required name matches `^[a-z0-9][a-z0-9._-]*$`. The optional strict variable name
matches `^[A-Za-z_][A-Za-z0-9_]*$`. Its value turns strict mode on only when exactly
`1`; without this option strict mode is off. The value is read at initialization.
Invalid initialization options exit 2 and print `<name or gate>: gate_init: <reason>`
to stderr. Before a valid name is known, the diagnostic prefix is `gate`.

After `--`, pass the consumer's `"$@"`. Accepted arguments are `--tier full`,
`--tier land` and `--list`, in any order, each at most once. Both tiers select the
same steps in this version. Invalid or repeated arguments exit 2 and print
`usage: <script> [--tier full|land] [--list]` to stderr, using the consumer's file name.

These public globals are set by initialization and may be read by the consumer:

| Global | Value |
| --- | --- |
| `GATE_NAME` | The validated diagnostic name. |
| `GATE_TIER` | `full` by default, or `land` when requested. |
| `GATE_LIST` | `0` by default, or `1` when `--list` was given. |

All other library state starts with `_GATE_`; all private functions start with
`_gate_`. Consumers must leave this private state alone.

### gate_missing

`gate_missing <what> <skipping>`

For a consumer's own toolchain or component probes. In strict mode, except when
listing, it prints `<name>: <what> is missing on this host (<VAR>=1)` to stderr and
exits 75. Otherwise it prints `<name>: <what> is missing, skipping <skipping>` to
stdout, increments the skip count and returns 0. Listing permits missing tools even
in strict mode; probe diagnostics may therefore precede the listed labels.

### gate_require

`gate_require <tool> <skipping>`

Returns 0 without output when `command -v <tool>` succeeds. Otherwise it calls
`gate_missing` with these arguments, then returns 1 if that call did not exit.

### gate_step

`gate_step [--test | --contract] [--mac-only] <label> [<command>]`

Registers a step, in order, and returns 0 without output. The command defaults to
the label. Label and command must be non-empty strings without newlines. Invalid
registration exits 2 with `<name>: gate_step: <reason>` on stderr. Kind options are
mutually exclusive. `--mac-only` requires `uname -s` to print `Darwin` when running.

| Kind | Exit 0 | Exit 1 | Exit 2 | Exit 75 | Other exit |
| --- | --- | --- | --- | --- | --- |
| Plain | Pass | Red | Red | Red | Red |
| `--test` | Pass | Red | Red | Unavailable | Red |
| `--contract` | Pass | Red | Internal error | Unavailable | Internal error |

For passing test and contract steps, every output line ending in `, skipped` adds
one skip and is echoed to stdout with four spaces of indentation after the timing
line. Plain steps do not contribute skips. Other passing output stays in the logs
until successful completion removes them.

### gate_run

`gate_run`

Runs the registered steps and exits; it never returns. No steps means exit 2 with
`<name>: no steps registered` on stderr, including when listing. With `--list`, it
prints labels in registration order, one per line, and exits 0 without creating logs
or running commands. Listing also bypasses the operating-system preflight.

Before any command runs, the first macOS-only step on another operating system
causes exit 75 with this stderr line:

```text
<name>: <label> runs only on macOS and this host is <os>; exit 75 so another host can run the gate
```

One log directory is created with `mktemp -d "${TMPDIR:-/tmp}/<name>.XXXXXX"`.
Creation failure exits 2 with `<name>: gate_run: cannot create a log directory` on
stderr. Each command runs from the recorded root, with stdin from `/dev/null` and
both output streams in `step-<n>.log`, numbered from 1. After each command the library
prints `step <n> <seconds>s <label>` to stdout, using elapsed whole seconds.

A red step prints `FAIL: <label> (whole log: <path>)` to stderr and continues.
An unavailable step prints
`UNAVAILABLE: <label> (exit 75: this host cannot run it; whole log: <path>)` to stderr.
An internal error prints `ERROR: <label> (exit <code>; whole log: <path>)` to stderr.
Each diagnostic is followed by the log's last 15 lines, indented four spaces, on
stderr. Unavailable and internal results stop execution and print
`<name>: stopped after step <n>; <k> steps not run` to stderr.

After execution, stdout shows `<name>: slowest steps` and up to five lines
`  <seconds>s <label>`, slowest first. Equal durations keep registration order.
Only steps that ran appear. Then the final line and exit status are:

| Priority | Final line | Exit | Stream |
| --- | --- | --- | --- |
| Stopped on internal error | `<name>: internal error (logs: <logdir>)` | 2 | stderr |
| Any red step | `<name>: <r> failed (logs: <logdir>)` | 1 | stderr |
| Stopped on unavailable | `<name>: this host cannot run the gate (logs: <logdir>)` | 75 | stderr |
| All passed, no skips | `<name>: all passed` | 0 | stdout |
| All passed, with skips | `<name>: all passed, <s> skipped` | 0 | stdout |

An internal error takes priority over earlier red results. A red result takes
priority over a later unavailable result. Exit 75 always means another host must
run the gate; it is never green. Logs remain on every non-green run. Green removes
the log directory before printing its final line. A failure to identify the host,
record the root or remove logs exits 2 with a diagnostic on stderr.
