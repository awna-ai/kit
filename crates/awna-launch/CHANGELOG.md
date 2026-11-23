# Changelog: awna-launch

This package is versioned on its own. Its releases are tagged `awna-launch-vX.Y.Z` (for example `awna-launch-v0.1.0`).

## 0.1.0

- Add a typed service unit with validation.
- Render launchd property lists.
- Add the Manager trait, a launchctl manager, and a recording stand-in.
- Apply service changes with policies to skip unchanged services, keep disabled services disabled, and protect running services, plus bounded unload waits and load retries, rollback, and a health check.
- Remove a service or retire a list of older service labels.

Linux user units will come later.
