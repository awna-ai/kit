# Security: kit

**Last reviewed:** 2026-11-23, when `awna-keystore` 0.1.0 was added.

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
| Consumer → `awna-run` | program, arguments, environment, stdin, output paths | the child leads its own process group and its environment is cleared and rebuilt from the caller's pairs; secret values are held zeroized and never shown by `Debug`, and no `Debug` prints input or output bytes; every group signal is sent while the leader is still unreaped, never to the caller's own group or to pid 0 or 1; a failed signal falls back to the leader and a bounded wait; files are opened without following a final link and without blocking, must be regular, and outputs must be the user's own, single-linked and private (0600) | `crates/awna-run/tests/`, the unit tests in its `lib.rs`, a security review and an independent verification before release |
| Consumer → `awna-keystore` | items (service, account, label) and secret values | `Secret` is zeroized on drop, has no `Clone`, `Display`, `PartialEq` or `Serialize`, and its `Debug` is redacted; items are checked against fixed character sets and every store refuses a value outside the token characters (at most 3072 bytes), so a value can never end its quoted word on the macOS command line, and that line is refused before anything runs if it would pass 4000 bytes (`security -i` cuts lines at 4095 bytes and runs the rest as another command); accounts hold no `.`, so the Windows credential name `account.service` names one item only; the macOS value goes to `security -i` on stdin only, never argv, with the environment cleared to `HOME`, stdin, stdout and stderr each sent to `/dev/null` where unused; the Linux store refuses a session bus address that is not `unix:`; every store is read back; errors carry the item and a fixed phrase, never a value or a store's own error text, and stored bytes in a keyring error are wiped; never a file fallback | `crates/awna-keystore/tests/` (a stand-in `security` records argv, stdin and environment), the unit tests in `src/keyring.rs` over keyring-core's mock store, a live keychain round trip run by hand, a security review and an independent verification before release |
| Consumer → `awna-launch` | unit definitions written to disk and loaded by launchd or the systemd user manager | units are rendered from typed fields, never raw text: labels from a fixed character set, an absolute program, control characters refused; property list text is XML-escaped; systemd `ExecStart` and `Environment` values are quoted with `\`, `"`, `%` (and in `ExecStart` `$`) escaped, and a log or working directory path holding a character systemd would reinterpret is refused, never escaped; unit files are written 0600 through a temporary file and a rename; every existing file and directory `apply` trusts must belong to the manager's user, and the directories must have no group or other write; a running service is never stopped by default; the real `launchctl` or `systemctl` runs only for the account's own home, and a stand-in that is or resolves to it is refused; lingering is reported, never enabled | `crates/awna-launch/tests/` (a recording stand-in manager; stand-in `launchctl` and `systemctl` scripts that record argv; unit text compared byte for byte), a live launchd round trip run by hand, a security review and an independent verification before release |
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

None over a network. The four core packages make no network calls; cargo fetches dependencies from crates.io at build
time. `awna-keystore` talks to the operating system's password store: `/usr/bin/security` on macOS, the Credential
Manager API on Windows, and the Secret Service over the local D-Bus session on Linux (a `DBUS_SESSION_BUS_ADDRESS` that
names anything but `unix:` sockets is refused, since the D-Bus library can also reach a TCP address or run a program).

## Supply chain

- Lockfile committed and every cargo step runs `--locked`: yes.
- GitHub Actions: none; the gate runs locally before each land.
- Dependencies: crates.io and this workspace only, checked by `scripts/deps-check.sh`.
- Gate: gitleaks (blocking), the term check (blocking), the dependency check (blocking).

## Known gaps and accepted risks

| Gap | Why accepted | Accepted by | Revisit |
| --- | --- | --- | --- |
| The forbidden-crate list matches exact names and two prefixes, not every networking crate | it mirrors what consumers forbid today; widening it is a design change | the maintainers, 2026-11-23 | when a package first takes a dependency |
| No advisory scan (`cargo audit` or `cargo deny`) yet | macOS builds resolve only `libc`, `zeroize`, `keyring-core` and `log`; `awna-keystore`'s Windows and Linux stores bring the platform API crates (7 on Windows) and a pure-Rust D-Bus and crypto stack (101 on Linux; 118 external crates in `Cargo.lock` in all, none a network client by name, checked by `scripts/deps-check.sh`); the gate hosts have no advisory tool | the maintainers, 2026-11-23 | before the first release that ships to outside users, or when a gate host gains an advisory tool |
| `awna-keystore`: the macOS `security` call has no timeout (a locked keychain can wait on a prompt); the standard library's output buffer and the keyring stores may leave unwiped copies of a value; `find` returns any non-empty text, so a consumer that needs the token rule checks `storable` itself; the Windows store has no label and holds at most 1,280 characters; the Linux store sets a label only when it creates an item; Linux needs a running Secret Service, else `NoStore`; an item written through `/usr/bin/security` trusts that program, so any process of the same user can read it through it without a prompt | the same behaviour as the stores it replaces; each is documented in the crate | the maintainers, 2026-11-23 | when a consumer needs any of them closed |
| The gate builds only the host's target, so `awna-keystore`'s Windows and Linux code is checked by hand (`cargo check --target` for each) before a release, not on every land | the gate hosts' pinned toolchain has no other targets installed | the maintainers, 2026-11-23 | when the gate hosts gain the targets |
| `awna-run`: a descendant that leaves the child's group, a file descriptor the host opened without close-on-exec, the caller's controlling terminal, a link in a parent directory of an output path, and copies of a secret inside the standard library's command builder are outside its control | each is documented in the crate's Limits; closing them needs a different spawn path or directory-relative opens, which the first release keeps out | the maintainers, 2026-11-23 | when a consumer needs any of them closed |
| `awna-launch`: the systemd path is proven against a stand-in `systemctl` only (no gate host runs a Linux user session), and Linux units support only `KeepAlive` and `Once` (interval and calendar runs need timer units, refused for now) | the rendered text follows systemd's documented unit syntax and the verbs are the standard `systemctl --user` ones; refusing timers is explicit, not silent | the maintainers, 2026-11-23 | before a consumer first installs a unit on Linux |
| `awna-run`: a host that ignores SIGCHLD or reaps children itself loses the reap-before-signal guarantee | documented; the run then ends `Unfinished` rather than signalling a group it cannot vouch for | the maintainers, 2026-11-23 | at the next review |
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
- 2026-11-23: `awna-run` 0.1.0: its boundary row, the advisory-scan gap revisited for its two dependencies, and its
  accepted limits.
- 2026-11-23: `awna-keystore` 0.1.0: its boundary row, the operating-system stores under external integrations, the
  advisory-scan gap revisited for its dependencies, and its accepted limits.
