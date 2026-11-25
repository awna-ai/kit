# kit

Small, separate building blocks for desktop tools that run AI coding agents on your own machine. Each one is a
package of its own, with its own version, tag and changelog, so a tool takes only the pieces it needs.

MIT licensed. Each package is released on its own tag; a package with no tag yet is still empty.

## The packages

| Package | What it does |
| --- | --- |
| `awna-run` | Runs an agent as a child process: its own process group, a rebuilt environment, bounded output, timeouts, and a kill that reaches the whole group. |
| `awna-agents` | Builds the command line for coding-agent CLIs and reads their streamed output. Parsing only, with no process code. |
| `awna-keystore` | Stores credentials in the system keychain, with secrets that never print, plus an in-memory store for tests. |
| `awna-launch` | Installs and controls background services: renders launchd and systemd user units, then loads, unloads and restarts them. |
| `awna-intake` | The shape of an intake item (a request someone sends in, its triage, the work proposed for it, and its status from submitted to shipped), a status machine that refuses every illegal move, attachment limits, and the store and processor traits, with an in-memory store and a conformance suite for other stores. Types only, with no I/O. |
| `awna-intake-pg` | A Postgres store for `awna-intake` items, its schema as one SQL file, and the `awna-intake` command line for reading items, moving their status, adding notes and exporting an item's files. Reaches only a database on this machine (a socket or a tunnel). |
| `awna-gate` | A bash library for a repository's local gate: register steps, run each with its own log, and exit by one contract (green, red, internal error, or "this host can't run it"). It can run only the steps a change needs, read steps from GitHub Actions workflows, and ships a term check and a secret scan. |

The five core Rust packages use no network client, not even behind a feature, so a tool that must stay offline can
depend on them. `awna-intake-pg` is separate and reaches only a local database target. `awna-gate` is shell, in `gate/`; its `README.md` says how to use it.

## Using a package

Each package is pinned by its own git tag, `<package>-vX.Y.Z`:

```toml
[dependencies]
awna-run = { git = "https://github.com/awna-ai/kit", tag = "awna-run-v0.1.0" }
```

Cargo fetches the repository and builds only the package you name. A new release of one package never moves
another package's tag.

`awna-gate` is not a crate. A repository copies it in at a tag and sources the copy:

```sh
git -C kit fetch --tags
bash kit/gate/vendor.sh awna-gate-v0.1.0 path/to/your-repo/tools/gate
```

The copy carries a `PIN` file naming the tag and its git tree. The gate checks the copy against it on every run and
fails if a file was changed, added or removed, which catches an accidental edit; the pin is not a signature,
since whoever can edit the copy can also edit or delete its `PIN`.

## Working on the kit

The Rust toolchain is pinned in `rust-toolchain.toml` (1.99.0, which is also the minimum supported version). Before a
change lands on `main`, run the whole gate:

```sh
KIT_STRICT=1 bash scripts/ci-local.sh
```

It runs the shell checks and tests, the term check, a secret scan over every ref, `rustfmt`, `clippy`, the tests,
the minimum-version build and the dependency check. Without `KIT_STRICT=1`, a tool your machine lacks is skipped and
counted. While you work, `bash scripts/ci-local.sh --tier touched` runs only the steps your change needs, by
`scripts/ci-touched.map`, and falls back to the whole gate when it can't tell. Read `AGENTS.md` before your first change.
