# Changelog: awna-run

This package is versioned on its own. Its releases are tagged `awna-run-vX.Y.Z` (for example `awna-run-v0.1.0`).

## 0.1.0 - 2026-11-23

- The supervisor `run`: one child in its own process group, with an environment rebuilt from the caller's pairs;
  stdin, stdout, stderr and kill policies, a stop flag and a tick callback; every group signal sent while the leader
  is still unreaped.
- File destinations are opened without following a final link and without blocking, and must be regular files;
  an output must also be the caller's own, have one link and be private, else the run does not start.
- `Debug` never prints a secret, nor any input or output bytes.
- `find_program`, which skips relative and empty search entries.
