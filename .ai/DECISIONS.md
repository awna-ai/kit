# kit: decisions

Newest last. Each decision says what was chosen and why.

## D1: one repository, separate packages

One public repository holds every package, but each package has its own version, `CHANGELOG.md` and tag
`<package>-vX.Y.Z`. A tool pins only the packages it uses, each at its own tag, so a change to one package never moves
another's pin. One repository keeps one gate, one term check and one place to look.

## D2: public hygiene

The repository names no product, person or private term, and code arrives as a fresh commit, never with the history of
the project it came from. A private term list, checked by `scripts/term-check.sh` over every file, path, object,
commit and tag message and ref name, enforces it; the list stays out of the repository.

## D3: parsing apart from spawning

`awna-agents` parses agent output and builds argument lists with no process code; `awna-run` spawns and supervises
processes with no agent parsing. A consumer that forbids process code in its parsers can take one without the other.

## D4: no network client in the four core packages

`awna-run`, `awna-agents`, `awna-keystore` and `awna-launch` resolve none of `reqwest`, `hyper`, `ureq`, `isahc`,
`surf`, `attohttpc`, any `tauri-plugin-*` or any `iroh*` crate, by any route: normal, build or dev dependency, on any
target, under any feature. Consumers that must stay offline unify features across their whole workspace, so even an
optional dependency would reach them. The same check refuses a dependency from outside crates.io and this workspace,
which a consumer could not audit the same way. `scripts/deps-check.sh` enforces both. A later package that needs HTTP
lives apart from these four.

## D5: ranges, not exact pins; one toolchain

Dependencies take caret ranges (`libc = "0.2"`, `zeroize = "1"`) and enable no optional `serde_json` features, so
consumers that pin different patch versions or features can all resolve the kit. Edition 2024, resolver 3; the minimum
supported Rust is 1.99.0, which `rust-toolchain.toml` also pins for the gate, and the gate builds with it explicitly.

## D6: the gate's exit contract

`scripts/ci-local.sh` exits 0 green, 1 red, 2 on usage or an internal error, and 75 when this host cannot run a step
under `KIT_STRICT=1` (a missing tool, toolchain or term list). 75 is never green: whoever runs the gate tries another
host. Without `KIT_STRICT=1` missing tools are skipped and counted, which suits an outside contributor.

## D7: awna-intake joins the no-network list

`awna-intake` is linked by tools that must stay offline, so `scripts/deps-check.sh` checks it with the same rule as
D4. It holds types and traits only; a store that needs a database client lives in a separate package.

## D8: the intake store is its own package, local database only

`awna-intake-pg` holds the database client (sqlx, Postgres only, the tokio runtime, no TLS, no `json`, `macros` or
`migrate` features), so `awna-intake` stays on the D7 list and a tool that only needs the types never resolves a
database crate. Its CLI is how a tool that must not link a database reaches the store. With no TLS, it connects only
to this machine (loopback or a Unix socket); a remote server is reached through a tunnel. It is not on the
no-network list.
