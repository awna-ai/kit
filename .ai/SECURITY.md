# Security: kit

**Last reviewed:** 2026-11-23, when the workspace and its gate were created.

## What this system is

A set of libraries that desktop tools link to run AI coding agents as child processes, keep their credentials in the
system keychain, and install background services. Nothing here runs as a service or listens on a network. The worst
outcomes are a library that leaks a secret (into a log, an argument list or a child's environment), lets a child
process outlive its supervisor, or installs a service that runs something it was not asked to; and, for the
repository itself, a private term or a secret published in public history.

## Trust boundaries

| Boundary | What crosses it | Control that holds today | Proven how |
| --- | --- | --- | --- |
| Contributor → public `main` | code, messages, tags, ref names | the gate: term check over every object, message and ref name; `scripts/secret-scan.sh` runs gitleaks over every ref's history, every reachable object whole (merges, tags, messages included) and the working tree, and refuses a repo-level gitleaks config | `tests/term_check_test.sh`, `tests/secret_scan_test.sh`; the gate before every land |
| Crate ecosystem → the four core packages | dependencies | `scripts/deps-check.sh`: no network client, no `tauri-plugin-*`, no `iroh*` (optional dependencies of dependencies included, through `Cargo.lock`), nothing from outside crates.io and this workspace, no repo-level cargo config; `Cargo.lock` committed, `--locked` everywhere | `tests/deps_check_test.sh` |
| Consumer → `awna-run` (from its first release) | program, arguments, environment, stdin | to be set by its design: the environment is rebuilt, never inherited | its tests, a security review and an independent verification before release |
| Consumer → `awna-keystore` (from its first release) | secret values | to be set by its design: a secret type with no `Clone`, `Display` or `Serialize`, read back after every store | the same |
| Consumer → `awna-launch` (from its first release) | unit definitions written to disk and loaded | to be set by its design: rendered from typed fields, never from raw text | the same |
| Agent output → `awna-agents` | untrusted model and tool text | parsed as data only, never evaluated or passed to a shell | its tests |

## Data classes

| Class | Examples | Where stored | Exposure impact | Retention |
| --- | --- | --- | --- | --- |
| Consumers' credentials | API tokens a consumer stores through `awna-keystore` | the consumer's system keychain, never this repo | full compromise of that account | the consumer's policy |
| The private term list | words the repo must not contain | each gate host's config directory, outside the repo | names private work | kept by its owners |

## Where secrets live

The repository holds no secret and its gate needs none. Tests use invented values only.

## Entry points that take untrusted input

- `awna-agents`: streamed agent output, line by line (from its first release).
- `awna-run`, `awna-keystore`, `awna-launch`: their public APIs, called by consumers (from each first release).
- `scripts/*.sh`: run by contributors on their own checkout; they read the repo, the term list and cargo's output.

## External integrations

None. The four core packages make no network calls; cargo fetches dependencies from crates.io at build time.

## Supply chain

- Lockfile committed and every cargo step runs `--locked`: yes.
- GitHub Actions: none; the gate runs locally before each land.
- Dependencies: crates.io and this workspace only, checked by `scripts/deps-check.sh`.
- Gate: gitleaks (blocking), the term check (blocking), the dependency check (blocking).

## Known gaps and accepted risks

| Gap | Why accepted | Accepted by | Revisit |
| --- | --- | --- | --- |
| The forbidden-crate list matches exact names and two prefixes, not every networking crate | it mirrors what consumers forbid today; widening it is a design change | the maintainers, 2026-11-23 | when a package first takes a dependency |
| No advisory scan (`cargo audit` or `cargo deny`) yet | the workspace has no dependencies | the maintainers, 2026-11-23 | with the first dependency |
| The gate runs the candidate commit's own code (its scripts, tests, build scripts and Rust tests) with the gate host's user and network | only maintainers' commits are gated; an outside contribution is read before it is run | the maintainers, 2026-11-23 | before the gate runs anything not authored here |
| The gate's steps and toolchain versions come from the commit under test, so a commit can weaken its own gate | the same: commits are the maintainers' own, and every land runs with `KIT_STRICT=1` | the maintainers, 2026-11-23 | the same |
| The term check prints the path of a file that names a term, and does not read author or committer identities | gate output stays on the host that ran it | the maintainers, 2026-11-23 | if gate output is ever published |
| An inline `gitleaks:allow` marker silences that line | a visible, reviewable opt-out | the maintainers, 2026-11-23 | at the next review |

## Who to tell

| Situation | Contact | How |
| --- | --- | --- |
| A secret or private term in public history | the maintainers | a private security advisory on the repository; rotate first, then purge |
| A vulnerability in a package | the maintainers | a private security advisory on the repository |

## Change log

- 2026-11-23: created with the workspace and its gate.
