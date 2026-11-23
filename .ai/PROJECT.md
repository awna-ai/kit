# kit: project facts

## What it is

A public, MIT-licensed Cargo workspace of small packages for desktop tools that run AI coding agents locally. Each
package is versioned, tagged and changelogged on its own and is consumed as a git dependency pinned to its tag.

## Layout

```
Cargo.toml             the workspace: members crates/*, resolver 3, edition 2024, rust-version 1.99.0
rust-toolchain.toml    the gate's toolchain (1.99.0, rustfmt, clippy)
crates/awna-run        process supervisor
crates/awna-agents     agent CLI argv builders and stream parsers (no process code)
crates/awna-keystore   credential store
crates/awna-launch     launchd and systemd user units
gate/                  awna-gate: the sourced bash library the gate is built on (VERSION, CHANGELOG.md, README.md)
scripts/ci-local.sh    the gate
scripts/term-check.sh  the private-term check (files, paths, objects, messages, ref names)
scripts/secret-scan.sh gitleaks over every ref, every reachable object and the working tree
scripts/deps-check.sh  the forbidden-dependency check for the four packages
tests/*_test.sh        the scripts' own tests
```

## Commands

| Task | Command |
| --- | --- |
| The whole gate | `KIT_STRICT=1 bash scripts/ci-local.sh` |
| One package's tests | `cargo test -p awna-run --locked` |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| Format | `cargo fmt --all` |
| A commit message | `bash scripts/term-check.sh --message <file>` |
| The dependency check alone | `bash scripts/deps-check.sh` |
| The gate's steps, run none | `bash scripts/ci-local.sh --list` |

## The gate's exit codes

`0` green · `1` a step failed · `2` usage or an internal error · `75` this host can't run a step (a missing tool,
toolchain or term list under `KIT_STRICT=1`). Without `KIT_STRICT=1`, missing tools are skipped and counted.

## The term list

`scripts/term-check.sh` reads `KIT_TERMS_FILE`, by default `${XDG_CONFIG_HOME:-$HOME/.config}/awna/terms.txt`: one
term per line, `#` comments. The list itself is private and never enters the repo. Without it, the strict gate exits
75.

## Releases

A package releases on its own: bump its `version`, move its `CHANGELOG.md` entries under the new version, land that on
`main` through the gate, then tag the green commit `<package>-vX.Y.Z`. There is no crates.io publish
(`publish = false`).
