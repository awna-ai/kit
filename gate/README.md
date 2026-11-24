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
A step's status is its command's last status, so a pipeline or `;` list reports only
its last part; start such a command with `set -o pipefail` (or `set -e`), or move it
into a script.

The host supplies `bash`, `mktemp`, `rm` and `tail`; macOS-only steps also need `uname`.
The touched tier and `--check-map` also need `git`.
The library uses the shell's `pwd` builtin and `SECONDS` clock. Logs can contain any
output the commands produce. Failed runs retain those logs in a temporary directory;
the consumer is responsible for removing them after inspection.

## Public functions

### gate_init

`gate_init --name <name> [--strict-var <VAR>] [--map <path>] [--base <ref>] [--] [<consumer arguments>...]`

Call exactly once before the other functions; a second call exits 2 with
`<name>: gate_init: called twice`. It records `pwd -P` as the root where every
step will run, even if the consumer later changes directory. It returns 0 on success.

The required name matches `^[a-z0-9][a-z0-9._-]*$`. The optional strict variable name
matches `^[A-Za-z_][A-Za-z0-9_]*$`. Its value turns strict mode on only when exactly
`1`; without this option strict mode is off. The value is read at initialization.
Invalid initialization options exit 2 and print `<name or gate>: gate_init: <reason>`
to stderr. Before a valid name is known, the diagnostic prefix is `gate`.

After `--`, pass the consumer's `"$@"`. Accepted arguments are `--tier full`,
`--tier land` and `--list`, in any order, each at most once. The full and land tiers select the
same steps; the touched tier selects steps by the map. Invalid or repeated arguments exit 2 and print
`usage: <script> [--tier full|land] [--list]` to stderr, using the consumer's file name.

These public globals are set by initialization and may be read by the consumer:

| Global | Value |
| --- | --- |
| `GATE_NAME` | The validated diagnostic name. |
| `GATE_TIER` | `full` by default, or `land` or `touched` when requested; unchanged after a fallback. |
| `GATE_LIST` | `0` by default, or `1` when `--list` was given. |
| `GATE_CHECK_MAP` | `0` by default, or `1` when `--check-map` was given. |

All other library state starts with `_GATE_`; all private functions start with
`_gate_`. Consumers must leave this private state alone.

### gate_missing

`gate_missing <what> <skipping>`

For a consumer's own toolchain or component probes. In strict mode, except when
listing or checking the map, it prints `<name>: <what> is missing on this host (<VAR>=1)` to stderr and
exits 75. Otherwise it prints `<name>: <what> is missing, skipping <skipping>` to
stdout, increments the skip count and returns 0. Listing and checking the map permit missing tools even
in strict mode; probe diagnostics go to stderr so stdout contains only the listed labels.
Map checking then exits 75 if any probe reported a missing tool.

### gate_require

`gate_require <tool> <skipping>`

Returns 0 without output when `command -v <tool>` succeeds. Otherwise it calls
`gate_missing` with these arguments, then returns 1 if that call did not exit.

### gate_step

`gate_step [--test | --contract] [--mac-only] [--always] <label> [<command>]`

Registers a step, in order, and returns 0 without output. The command defaults to
the label. Label and command must be non-empty strings without newlines. Invalid
registration exits 2 with `<name>: gate_step: <reason>` on stderr. Kind options are
mutually exclusive. Repeated flags exit 2. `--always` selects the step on every touched run;
it has no effect on full or land runs. `--mac-only` requires `uname -s` to print `Darwin` when running.

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

## The touched tier

Opt in with `gate_init --name <name> --map <path> [--base <ref>] -- "$@"`.
Every initialization option may occur at most once. The map path must be non-empty
and contain no newline. Relative paths are resolved from the recorded repository
root; absolute paths are allowed. The map is read only by `gate_run`, so a missing
map is not an initialization error. The base defaults to `origin/main` and must
match `^[A-Za-z0-9][A-Za-z0-9._/-]*$`; invalid values exit 2. A base without a map
is allowed and has no effect. The base is read from local refs only: the gate never fetches.
A base starting with `refs/` names only that full ref. Otherwise the candidates are
`refs/<base>`, `refs/tags/<base>`, `refs/heads/<base>` and `refs/remotes/<base>`.
Exactly one must resolve to a commit; multiple candidates are ambiguous even when
pointing at the same commit. Selection uses the merge base of HEAD and that commit.
The recorded root must physically equal the top of the repository.

With a map, consumer usage is:

```text
usage: <script> [--tier full|land|touched] [--list] | --check-map
```

`--check-map` must be the only consumer argument. Without a map, touched and
check-map are usage errors (exit 2), and the original usage line still applies.

Each physical map line is blank, a `#` comment, or `<path pattern> <step pattern>`
after trimming leading and trailing spaces and tabs. The first field is the path
pattern; the trimmed rest is the step pattern and may contain spaces. Both are
Bash patterns used as data with `[[ $text == $pattern ]]`, never evaluated as code.
`*` matches across slashes and spaces. A step pattern of `-` maps a path without
selecting a step. Every other step pattern must match a registered label, not a
command. Patterns follow the consumer's `extglob` setting; Bash 4.1 and later
read `[[ ]]` patterns as extended globs. Maps should use plain globs. The library
does not change shell options. With `nocasematch` set, touched selection falls
back, and map checking reports `FAIL: nocasematch is set`.

```text
# Changes to source run the matching registered checks.
src/*       build *
docs/*      -
```

Touched selection uses the resolved merge base, then NUL-separated paths from
`git diff -z --name-only --no-renames --no-ext-diff --no-textconv --ignore-submodules=none --no-relative <merge base> --`
and `git ls-files -z --others --exclude-standard`. It sees committed, staged,
unstaged, deleted and untracked changes, excludes ignored untracked files, and
counts each path once. The tier measures the working tree: index-only changes
(staged then removed or reverted in the working tree) and files marked
`--assume-unchanged` or `--skip-worktree` are not changed paths. Restored paths are
not changes. Selecting many thousands of paths takes seconds (about 10 seconds
for 3000 paths in a prior measured run).
Temporary files are created under `${TMPDIR:-/tmp}` with `mktemp` and removed
before listing or running steps. A signal between creation and removal can leave
one 0600 file there; the library installs no cleanup trap.

Every matching map line selects every matching step label; `-` does not cancel
other matches. Changes to the map, the consumer script or the sourced library
always run the full tier, even with a matching `* -` row. These paths are recorded
relative to the root at initialization, with directories resolved physically;
files outside the root are ignored for this rule.

A step also maps and selects a changed path when a word of its label or command
equals the path exactly and holds a `/` or a `.`. Bare words such as `warnings`
do not map root files. Words use `read -r -a` with default whitespace splitting,
without pathname expansion or shell quote interpretation. A path containing spaces
also matches a space-delimited run in the label: `bash tests/my ok_test.sh` names
`tests/my ok_test.sh`. `--always`
steps are selected even with no changes. Selected steps run once, in registration
order. The notice is `<name>: tier touched, <p> paths changed, <s> steps` on stdout,
or stderr when listing; listing's stdout holds only selected labels.

The first uncertainty falls back to every registered step, in this order:

| Condition | Reason |
| --- | --- |
| No git on PATH | `git is unavailable` |
| Case-insensitive matching enabled | `nocasematch is set` |
| Unreadable map | `cannot read <map>` |
| Missing step field | `<map> line <n> has no step` |
| Step pattern matches no label | `<map> line <n> names no registered step: <pattern>` |
| Root is not the physical repository top | `the gate root is not the top of the repository` |
| Multiple base refs resolve to commits | `the base <base> is ambiguous` |
| Merge-base fails or no base ref resolves | `no merge base with <base>` |
| Change listing or temporary-file creation fails | `could not list the change` |
| A changed path has bytes outside printable ASCII, a double quote or a backslash | `a changed path has a character the map cannot match` |
| A changed path is the map, consumer or library | `<path> is one of the gate's own files` |
| A path matches neither a map line nor a step path | `<path> has no entry in <map>` |

Map problems use physical line numbers starting at 1 and the map path exactly as
given. The notice is `<name>: tier touched falls back to the full tier: <reason>`
on stdout, or stderr when listing. Map diagnostic bytes outside printable ASCII
are replaced by `?`; unsafe changed-path bytes are never printed. `GATE_TIER`
remains `touched`, including after fallback.

Selection happens before listing and macOS preflight. Only selected macOS steps
can cause exit 75. Timing, numbering, slowest steps and the count of steps not run
cover only selected steps. An empty selection creates no log directory, prints no
step timing or slowest list, and exits 0 with `<name>: all passed` (or
`<name>: all passed, <s> skipped`). An empty list prints no labels and exits 0.

If any `gate_missing` call happened, `--check-map` prints
`<name>: cannot check the map on this host: a tool is missing` to stderr and exits
75 before reading the map, in both strict and non-strict mode. This is unavailable,
never a successful validation.

Otherwise `--check-map` requires git, lists tracked files with `git ls-files -z`, then checks
every map line in order. It prints `FAIL: <map problem>` to stderr for every map
problem above, plus `FAIL: <map> line <n> matches no tracked file: <path pattern>`
for each line whose path pattern matches no tracked file. No problems
prints `<name>: map ok` to stdout and exits 0; otherwise the final stderr line is
`<name>: map has <k> problems`, exit 1. Missing git, a failed tracked-file listing,
a temporary-file failure or a root that is not the physical repository top exits
2 with a `gate_run` diagnostic (`the gate root is not the top of the repository`
for the root mismatch). No steps still
exits 2 before map checking. Checking runs no step and creates no log directory;
missing-tool probes are counted and printed to stderr before the final exit 75.
