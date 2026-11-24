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

Plain commands and labels come from the consumer's own script. They are trusted shell
code. Workflow commands are read only through the strict parser described below.
Plain commands run with `bash -c`, without `eval`, and inherit the consumer's
environment, but not its unexported shell variables or functions. A plain step's status is its command's last status, so a pipeline or `;` list reports only
its last part; start such a command with `set -o pipefail` (or `set -e`), or move it
into a script.

The host supplies `bash`, `mktemp`, `rm` and `tail`; macOS-only steps also need `uname`.
The touched tier and `--check-map` also need `git`; the pin check needs git 2.29 or later, `find`, `readlink` and `mktemp`.
Workflow steps also need `env`, `tr` and `cmp`.
The library uses the shell's `pwd` builtin and `SECONDS` clock. Logs can contain any
output the commands produce. Failed runs retain those logs in a temporary directory;
the consumer is responsible for removing them after inspection.

## Public functions

### gate_init

`gate_init --name <name> [--strict-var <VAR>] [--map <path>] [--base <ref>] [--own <path>]... [--] [<consumer arguments>...]`

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

### gate_workflow

`gate_workflow [--dir <directory>] [--pass <VAR>]... [--] <file>`

Registers workflow commands after `gate_init` and before `gate_run`, returning 0
silently on success. Calls may repeat and mix with `gate_step`; registration order
is preserved. See “Steps from GitHub Actions workflows” for its grammar, hooks,
refusals and execution rules.

### gate_run

`gate_run`

Runs the registered steps and exits; it never returns. No steps means exit 2 with
`<name>: no steps registered` on stderr, including when listing. With `--list`, it
prints labels in registration order, one per line, and exits 0 without creating logs
or running commands. Listing also bypasses the library pin check and the operating-system preflight.
Map checking also exits before the pin check. Other runs check the library pin
after selection and before macOS preflight or log creation, even with no selected steps.

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
Every initialization option except `--own` may occur at most once. The map path
must be non-empty and contain no newline. Relative paths are resolved from the recorded repository
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
other matches. Changes to the map, the sourced library or any file named by
`gate_init --own <path>` always run the full tier, even with a matching `* -` row.
A consumer names its own script with `--own`, for example `--own scripts/check.sh`;
the library never guesses that path from the invocation. Repeat `--own` to name
additional files. Each value must be non-empty and contain no newline; a missing
or invalid value is a `gate_init` error (exit 2). Paths are resolved from the
recorded root at initialization, with directories resolved physically; paths
outside the root or whose directories cannot be resolved are silently ignored.
The map and sourced library are added automatically.

A step also maps and selects a changed path when a word of its label or command
equals the path exactly and holds a `/` or matches `*.[A-Za-z]*` (a dot followed
by a letter, as in a file extension). Bare words such as `warnings` and cargo
fragments such as `-D warnings` or `+1.99.0` do not map root files. Words use
`read -r -a` with default whitespace splitting, without pathname expansion or
shell quote interpretation. A path containing spaces
also matches a space-delimited run in the label when the path passes that same
slash-or-extension test: `bash tests/my ok_test.sh` names
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
| A changed path is the map, library or a file named by `--own` | `<path> is one of the gate's own files` |
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

## Steps from GitHub Actions workflows

A consumer can read its local GitHub Actions files instead of maintaining a second
list of commands. Workflows are executable input: use only repository files you
trust, and define hooks to allow exactly the jobs and setup the local host can mirror.
The environment scrub removes variables only: steps still run as you, with your
`HOME`, so they can read files such as SSH and cloud credentials under it. Running
`gate_workflow` on a branch you do not trust runs that branch's code with your credentials.
The parser is deliberately a small indentation grammar, not a general YAML reader.
All parsing and hook decisions happen during `gate_workflow`, including under
`--list` and `--check-map`, before any step runs. The first problem exits 2.

```bash
gate_workflow_job() {
    [ "$1" = ci.yml ] && [ "$2" = build ]
}
gate_workflow_uses() {
    [ "$3" = org/setup@v1 ]
}
gate_workflow_env() {
    if [ "$3" = APPLE ]; then printf '%s' 'a local value'; else return 1; fi
}
gate_init --name local-check -- "$@"
gate_workflow --pass EXTRA_ONE ci.yml
gate_run
```

The file argument is exactly one name matching `^[A-Za-z0-9._-]+\.ya?ml$`;
paths containing `/` are refused. `--dir` defaults to `.github/workflows`. It may
occur once and must be non-empty with no newline. Relative directories resolve
from the recorded gate root, even after a directory change; absolute directories
are accepted. Repeat `--pass` to name variables matching
`^[A-Za-z_][A-Za-z0-9_]*$`. `--` ends option parsing.

Invalid calls print `<name or gate>: gate_workflow: <reason>` on stderr and exit 2:
`gate_init has not run`, `invalid workflow file name`, `invalid --dir`,
`missing or repeated --dir`, `invalid --pass`, `unknown option: <option>`, or
`expected one workflow file`, or `nocasematch is set`. Case-insensitive matching
must be disabled by the caller; a refusal leaves the option unchanged.

Files are read breadth-first: the entry file's jobs first, then each called local
reusable workflow in discovery order. Each file is read once per call, so cycles
end. A job-level `uses: ./.github/workflows/<name>` reads that name from the same
`--dir`, even when it is a custom directory. The call itself registers no step.
Symbolic-link workflow files are refused. Every read workflow is added to the gate's
own files using the same root-relative physical-directory resolution as `gate_init --own`; a workflow change forces the
full tier. Files outside the root are not added to the own-path list.

### Accepted grammar

Blank lines containing only spaces and comments whose first non-space character
is `#` are skipped. Before parsing, any NUL byte refuses the whole file. A carriage
return or tab anywhere on a line is refused. Every other byte must be printable
ASCII (0x20 through 0x7E), with parsing local to the C locale. Non-ASCII text in a
workflow, even in a comment or a `name:`, is refused; comments are not exempt.
For every value, text from the first space or tab followed by `#` is removed, then
surrounding spaces are trimmed, without interpreting quotes or escapes.
Thus `run: echo "a # b"` becomes the command `echo "a`, which fails in Bash.

Every inline value read at any level must close on its own physical line. Values
starting with `&`, `*` or `!` are refused as anchors, aliases or tags. A value starting
with `"` must match `^"[^"\\]*"$`; one starting with `'` must match `^'[^']*'$`.
A value starting with `[` must match `^\[[^][{}"']*\]$`; one starting with `{`
must match `^\{[^][{}"']*\}$`. These checks apply after comment removal, including
to ignored inline names, ids, timeouts, setup arguments and replaced env values.
Ignored nested blocks also receive this scalar check. The gate checks the whole
line, including keys and standalone list items, after stripping leading sequence
markers (`-` followed by one or more spaces). Explicit YAML keys and values (`?` or `:` followed by a space,
or alone) are refused. Plain keys cannot contain any of `" ' [ ] { } # & * ! | > , ?`
or a backtick. Plain scalar list items and continuations cannot contain
`" ' [ ] { }`. Keyless block scalars (`|` or `>`) are refused.
In ignored nested regions, a keyed block scalar (`|`, `>` and their indentation
and chomping modifiers) has literal content until a non-blank line at or left of
the key's column, including a comment line. Content still receives every byte
check but is not checked for keys, quotes or flow values.
Thus `name: "a # b"` is refused because comment removal leaves `"a`. A plain
`run:` keeps its stricter rules below. Duplicate top-level keys, job ids, job keys,
step keys, env names and service names are refused within their respective scopes.

| Indentation | Accepted content |
| --- | --- |
| 0 | `---` only before any other non-blank, non-comment line, or `<key>:` with an optional value; keys are `name`, `run-name`, `on`, `permissions`, `concurrency`, `jobs`. Keys match `^[A-Za-z_-]+$`. `jobs:` must have no value. |
| Under another top-level key | Nested contents are ignored. Top-level `env` and `defaults` are refused. |
| 2 under `jobs:` | `<job>:` with no value, matching `^[A-Za-z0-9_-]+$`; jobs keep file order. |
| 4 | A job key matching `^[A-Za-z0-9_-]+$`, with an optional value. |
| 6 under `env:` | Single-line `<NAME>: <value>` pairs with names matching `^[A-Za-z_][A-Za-z0-9_]*$`; deeper lines are refused. Workflow values are ignored and replaced by the env hook. |
| 6 under `services:` | `<name>:` with no value, matching `^[A-Za-z0-9_-]+$`; deeper settings are ignored. |
| 6 under `steps:` | `- <key>: <value>` starts a block-style step. |
| 8 under a step | `<key>: <value>` continues the same step. |
| 10 or deeper under step `with:` | Arguments are ignored. |

Built-in job keys are `name`, `needs`, `runs-on`, `steps`, `env`, `services`,
`permissions`, `timeout-minutes`, `uses`. Other keys require the job-key hook.
Allowing such a key means the gate ignores the key and its whole nested block,
including a matrix or condition; the hook must ensure ignoring it is appropriate.
Nested lines under `runs-on` or job-level `uses` are refused because YAML could
continue their plain values. Nested contents of other built-in keys are ignored
except for the specific blocks above. Odd indentation under `jobs:` is refused
except inside an ignored job-key block, service settings deeper than 6, or `with:` arguments at 10 or deeper.
A top-level line ends the current jobs block.

`runs-on` must match `^[A-Za-z0-9._-]+$`. A case-insensitive `macos-*`
label marks every run step of the job macOS-only. Only selected steps trigger the existing preflight: off
macOS it exits 75 before any step runs, naming the step's label. Other plain labels
impose no operating-system restriction; the gate does not provision a runner.
A job with steps must have `runs-on`, and cannot also have job-level `uses`.
Every job must contain at least one run command or a local reusable call. Every
file, including a called file, must contain at least one job in its `jobs:` block.

Step keys are only `name`, `id`, `run`, `uses`, `with`, `timeout-minutes`.
Every step needs exactly one of `run` and `uses`. Allowed `uses` steps register
nothing; names, ids, timeouts and setup arguments are ignored. An action reference
must match `^[A-Za-z0-9._/@:-]+$` and pass its hook. No action is downloaded or run.

A run value is the exact trimmed single-line command, without unquoting or
unescaping. Empty values and block scalars (`|`, `>` and their modifiers) are
refused. It cannot start with any of `' " | > [ ] { } & * ! % @` or a backtick,
`# , ? :`, or `- `; cannot contain `: ` or end in `:`; and cannot contain `${{`.
This refuses expressions, quoted YAML values, flow collections, anchors and tags.
Multi-line commands, alternate shells, step conditions, step environments,
working directories, outputs, matrices, remote reusable workflows, action behavior,
service provisioning and dependency scheduling are deliberately not mirrored.
`needs` is ignored: execution is sequential file order, not GitHub's scheduler.

### Consumer hooks

Hooks must be shell functions defined before `gate_workflow`; an undefined hook
allows nothing. Each runs in a subshell in a condition with stdin from `/dev/null`,
so rejection works under `set -euo pipefail` and an `ERR` trap. Stderr passes through unchanged.
Only the env hook's stdout is used; other hook stdout is discarded.
Hooks run with `LC_ALL=C` set for the duration of `gate_workflow`.
Parser locals use the reserved `_GATE_WF_` prefix so hooks can read their own
consumer globals. Hook assignments cannot change parser state. Arguments are
validated data, never evaluated as shell code.

| Function | Arguments | Return 0 to |
| --- | --- | --- |
| `gate_workflow_job` | `<file> <job>` | Mirror the job. |
| `gate_workflow_job_key` | `<file> <job> <key>` | Ignore an extra job key and its entire nested block. |
| `gate_workflow_env` | `<file> <job> <name>` | Supply the variable's value on stdout, replacing the workflow value. The name comes from the workflow: allow only names the hook knows, and never pass a value through by name (such as `${!3}`). |
| `gate_workflow_service` | `<file> <job> <name>` | Accept a service whose stand-in the consumer provides. |
| `gate_workflow_uses` | `<file> <job> <ref>` | Skip an action whose setup the host already provides. |

Env stdout is captured with `$( )`, which removes trailing newlines. A value that
still contains a newline is refused. Each job's assignments keep workflow order,
including when `env:` follows `steps:`.

### Execution and selection

Run steps register as plain steps labelled `<job>: <command>`, with no `--always`
flag. Lists and touched-map patterns match these labels like any other step; path
words in labels or commands also select them. In the touched tier, selecting any
workflow step selects every step in that same job group, in registration order,
including its earlier setup. Their skip group is `<call number>/<file>/<job>`, where
the call number counts `gate_workflow` calls in this run. Jobs from separate files or calls, including
same-named files in different `--dir` directories, remain independent.

Workflow commands run from the recorded root with stdin `/dev/null`, the same
private logs and timings as plain steps, using this argv:

```text
env -i <assignments...> "$BASH" -e -c "<command>"
```

`$BASH` is the running shell's absolute path. `-e` mirrors GitHub Actions'
unspecified shell on Linux and macOS, `bash -e {0}`; no pipefail is implied.
Each assignment is one argv word, never a re-parsed string. In order:

1. The caller's exported values of `HOME`, `USER`, `LOGNAME`, `PATH`, `TMPDIR`, `LANG`, `TERM`.
2. `CI=true`.
3. Each exported `--pass` variable, in call order.
4. The job env hook's assignments, in workflow order.

Later assignments win, including env-hook values overriding `--pass` values.
Unset and unexported variables are omitted; exported empty values remain empty.
Caller values are taken when `gate_run` starts. Other environment variables and
exported functions are absent. Plain `gate_step` commands retain their existing
inherited environment and `bash -c` behavior.

After a red workflow step, later selected steps of the same group print
`step <n> skipped <label> (an earlier step of job <job> failed)` to stdout. They
create no log, have no timing, add nothing to the missing-tool skip count, and do
not appear among the slowest steps. Numbering retains their selected positions.
Other jobs and plain steps continue. The final red count, logs and exit 1 follow
the existing contract.

### Workflow refusals

Each refusal prints one stderr line and exits 2: `<name>: <file>: <reason>` for
file problems, or `<name>: <file>, job <job>: <reason>` for job problems.
Non-printable bytes in workflow diagnostics become `?`. Physical line numbers
start at 1. Hook stderr can print additional lines of the consumer's own.

| Problem | Reason |
| --- | --- |
| Missing or unreadable file | `workflow not found in <dir as given>` |
| Symbolic-link workflow file | `workflow is a symbolic link` (file-level) |
| NUL byte anywhere | `a NUL byte` (file-level) |
| Other non-printable or non-ASCII byte | `line <n>: a byte outside printable ASCII` (file-level) |
| Anchor, alias or tag prefix in an inline value | `line <n>: anchors, aliases and tags are not mirrored` |
| Explicit YAML key or value | `line <n>: explicit keys are not mirrored` |
| Quoted or flow value that fails the single-line pattern, unsafe key characters, or quotes/brackets/braces in a plain scalar list item or continuation | `line <n>: a quoted or flow value must close on its own line` |
| Duplicate key, job id, env name or service name | `line <n>: duplicate key '<key>'` (job-level inside a job) |
| Document marker after other content or another marker | `line <n>: a second YAML document` |
| Caller has case-insensitive matching enabled | `nocasematch is set` (`gate_workflow` call error) |
| Carriage return or tab anywhere on a line | `line <n>: a carriage return or a tab` |
| Unknown top-level key | `line <n>: top-level key '<key>' is not mirrored` |
| Invalid top-level line, indentation/shape under jobs, or nested runner/call value | `line <n>: cannot be mirrored` |
| Missing, empty or inline jobs block | `no jobs: block` |
| Invalid job id or job with an inline value | `line <n>: cannot be mirrored` |
| Rejected job (file-level) | `job '<job>' is not mirrored; define gate_workflow_job to allow it` |
| Rejected extra job key | `job key '<key>' is not mirrored` |
| Inline env, services or steps | `<key> must be a block` |
| Non-plain runner | `runs-on '<value>' is not a plain runner label` |
| Steps with no runner | `steps without runs-on` |
| Non-local reusable call | `uses '<value>' is not a local reusable workflow` |
| Job has steps and a call | `both steps and uses` |
| Job has neither run commands nor a call | `no run: steps to mirror` |
| Rejected env name or multiline hook value | `env variable <name> is not mirrored` |
| Invalid or nested env pair | `line <n>: env must be single-line NAME: value pairs` |
| Rejected service | `service '<name>' is not mirrored` |
| Flow or bare step | `line <n>: write the step in block style` |
| Unknown step key | `step key '<key>' is not mirrored` |
| Empty or block-scalar run | `line <n>: a multi-line run: step` |
| Non-plain run scalar | `line <n>: run: is not a plain one-line value` |
| Expression in run | `line <n>: run: uses an expression` |
| Rejected or malformed action reference | `step uses '<value>' is not mirrored` |
| Step with neither or both run and uses | `line <n>: a step needs exactly one of run: and uses:` |
| No run command across the entry and called files | `no run: steps found` (entry file) |

## Scanners

The standalone scanners run as `bash <script>`, from any directory. They take
explicit inputs and do not read a consumer's strict setting unless named:

```sh
bash gate/term-check.sh --terms terms.txt --strict-var CHECK_STRICT --root /path/to/repository
bash gate/term-check.sh --terms terms.txt --message message.txt
bash gate/secret-scan.sh --strict-var CHECK_STRICT --root /path/to/repository
```

`--terms` is required and non-empty. Exactly one of `--root` (the repository's top directory)
and `--message` is required for the term scanner; the secret scanner requires
`--root`. Each option may occur once. `--strict-var` is optional, must be a shell
variable name, and enables strict mode only when that variable's value is `1`.
The scanners read the variable from the environment, so it must be exported
(unlike `gate_init`, which also reads a shell variable). Without it, strict mode is off.
Invalid arguments print usage to stderr and exit 2.

The term list holds one term per line; blank lines and comments starting with `#`
(after whitespace) are ignored. The term scanner checks tracked and unignored
untracked files, binary contents, link targets, paths, ref names, all reachable
history objects and commit and tag messages. Matching ignores case, uses whole
words, and also separates underscores, camel case and adjacent letters and digits.
It refuses grafted history and malformed message objects. `--message` checks only
the named readable file. Diagnostics name the file, line or object and never print
matched file contents. A path or link target that itself holds a term is printed as it is.
Exit 0 means clean, 1 means a term was found, and 2 means usage or an
internal error. A missing, unreadable or empty list prints a `, skipped` line and
exits 0, or prints the reason to stderr and exits 75 in strict mode.

The secret scanner needs `gitleaks`, `git` and the usual shell tools, including
`tar`. It scans every ref's history, every reachable object whole, and tracked and
unignored untracked regular files. It ignores external scanner configuration and
refuses repository files named `.gitleaks.toml` or `.gitleaksignore`, at any level
and in any case. Reports are redacted. Exit 0 means clean, 1 means a finding or
refused configuration, and 2 means usage or an internal error. Missing `gitleaks`
prints a `, skipped` line and exits 0, or exits 75 in strict mode with the named
variable in the diagnostic.

## Vendoring and the pin

Fetch the release tags before copying. The vendor script reads local tags only;
it never fetches and never copies the source working tree:

```sh
bash <checkout>/gate/vendor.sh awna-gate-v0.1.0 <consumer>/vendor/gate
```

The destination's parent must exist. A new destination must be absent; an existing
one must be a directory with a valid, readable, regular, non-link `PIN` whose first
line is exactly the marker below. A destination link is refused. The script validates
the tagged tree, writes and verifies a temporary copy beside the destination, then
replaces the old copy. Success exits 0 and
prints `vendor: copied <tag> (tree <tree>) to <destination>`. Usage and copy failures
exit 2 with a diagnostic on stderr and preserve the destination.
A concurrent writer in the destination's parent can race the final rename;
vendor into a directory only you write.

The copy holds the tag's files and modes plus this mode-644 `PIN`:

```text
# Written by vendor.sh. The gate fails when this directory differs from this tag's tree.
tag awna-gate-v0.1.0
tree <40 lowercase hex characters>
```

Treat the copied directory as read-only. Keep the consumer's scripts, maps and
other files elsewhere. Re-run the vendor command to update or repair the copy.
A checkout that drops executable bits or rewrites line endings shows as drift.

On every run, `gate_run` hashes the library independently of the consumer's index
and user Git configuration, leaving out only the top-level `PIN`. A matching copy
prints `<name>: library pinned at <tag>`. A changed byte, mode, link, extra or
removed file, or nested `.git` entry or an empty directory fails with exit 1 before
any step or log directory is made. The diagnostic names the pinned and found trees and says to
vendor it again. A malformed, linked or unreadable pin also exits 1. A hashing
failure exits 2. Missing `git`, `find`, `readlink` or `mktemp` calls `gate_missing <tool> 'the library pin check'`:
strict mode exits 75; otherwise the check is skipped and counted. Without a `PIN`,
there is no check or pin output. `--list` and `--check-map` bypass it.
Running a vendored scanner directly (without `gate_run`) checks no pin.

A pin accepts blank lines and lines starting with `#`, plus exactly one valid
`tag` line and one 40-lowercase-hex `tree` line, in either order. It detects local
drift; it is not a signature or a source-authenticity check.
Removing `PIN` turns the check off, so the pin guards against accidental edits to
the copy, not against someone who can also rewrite the consumer's repository.
