# AGENTS.md: kit

Read in this order: `README.md`, `.ai/PROJECT.md`, `.ai/DECISIONS.md`, `.ai/SECURITY.md`. Run
`KIT_STRICT=1 bash scripts/ci-local.sh` before anything lands on `main`.

Gotchas, before anything else:

- **This repo is public and names no product, person or private term** (D2): not in code, docs, tests, paths,
  branch names, tags or commit messages. Tests use invented names. `scripts/term-check.sh` enforces a private term
  list over every file, path, object, message and ref name; check a commit message with
  `bash scripts/term-check.sh --message <file>` before `git commit -F <file>`.
- **Code enters as a fresh commit**, never with the history of the project it came from (D2).
- **Each package stands alone** (D1): its own version, `CHANGELOG.md` and tag `<package>-vX.Y.Z`. A package depends on
  another package here only when its design says so.
- **No network client in `awna-run`, `awna-agents`, `awna-keystore`, `awna-launch` or `awna-intake`** (D4, D7),
  not even behind an optional feature. `scripts/deps-check.sh` fails the gate on one, and on any dependency from
  outside crates.io and this workspace.
- **Parsing and spawning stay apart** (D3): `awna-agents` holds no process code; `awna-run` holds no agent parsing.
- **Version ranges, not exact pins** (D5): `libc = "0.2"`, `zeroize = "1"`, and no optional `serde_json` features.
- **No fixed paths.** Scripts find the repo from their own location.
- **Exit 75 means "this host can't run the gate"** (D6), never green: try another host.
