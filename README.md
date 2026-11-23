# kit

Small, separate building blocks for desktop tools that run AI coding agents on your own machine. Each one is a
package of its own, with its own version, tag and changelog, so a tool takes only the pieces it needs.

MIT licensed. Nothing here is released yet: the packages below are empty until their first tag.

## The packages

| Package | What it does |
| --- | --- |
| `awna-run` | Runs an agent as a child process: its own process group, a rebuilt environment, bounded output, timeouts, and a kill that reaches the whole group. |
| `awna-agents` | Builds the command line for coding-agent CLIs and reads their streamed output. Parsing only, with no process code. |
| `awna-keystore` | Stores credentials in the system keychain, with secrets that never print, plus an in-memory store for tests. |
| `awna-launch` | Installs and controls background services: renders launchd and systemd user units, then loads, unloads and restarts them. |

None of these four packages uses a network client, not even behind a feature, so a tool that must stay offline can
depend on them.

## Using a package

Each package is pinned by its own git tag, `<package>-vX.Y.Z`:

```toml
[dependencies]
awna-run = { git = "https://github.com/awna-ai/kit", tag = "awna-run-v0.1.0" }
```

Cargo fetches the repository and builds only the package you name. A new release of one package never moves
another package's tag.

## Working on the kit

The Rust toolchain is pinned in `rust-toolchain.toml` (1.99.0, which is also the minimum supported version). Before a
change lands on `main`, run the whole gate:

```sh
KIT_STRICT=1 bash scripts/ci-local.sh
```

It runs the shell checks and tests, the term check, a secret scan over every ref, `rustfmt`, `clippy`, the tests,
the minimum-version build and the dependency check. Without `KIT_STRICT=1`, a tool your machine lacks is skipped and
counted. Read `AGENTS.md` before your first change.
