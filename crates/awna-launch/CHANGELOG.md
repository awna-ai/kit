# Changelog: awna-launch

This package is versioned on its own. Its releases are tagged `awna-launch-vX.Y.Z` (for example `awna-launch-v0.1.0`).

## 0.2.0

- Render systemd user service units, and add a systemctl manager with a stand-in for tests.
- `Manager` gains `platform`, so `apply`, `remove` and `retire` write and remove the platform's own unit file.
- Add `lingering`, which reports whether a Linux user's services outlive a logout. The kit never enables it.
- Breaking: `Manager::platform` is a new required method, and `UnitError` gains `Unsupported` and is now `#[non_exhaustive]`.
- A log file that appears between the check and its creation is accepted only when it is a regular file of the manager's user with one link, and a log directory `apply` creates is checked again before use.
- A Linux `KeepAlive` unit has no start rate limit, a mask link in the units directory counts as disabled and is never deleted, and values systemd would refuse are refused when rendering.

## 0.1.0

- Add a typed service unit with validation.
- Render launchd property lists.
- Add the Manager trait, a launchctl manager, and a recording stand-in.
- Apply service changes with policies to skip unchanged services, keep disabled services disabled, and protect running services, plus bounded unload waits and load retries, rollback, a health check, and ownership checks on the files it trusts.
- Remove a service or retire a list of older service labels.
