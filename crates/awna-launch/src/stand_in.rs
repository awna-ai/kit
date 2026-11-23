use crate::{Manager, ManagerError, State};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A recorded call to the stand-in manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// The session was checked.
    SessionOk,
    /// A service state was queried.
    State(String),
    /// A service's disabled flag was queried.
    Disabled(String),
    /// A plist was passed for loading.
    Load(PathBuf),
    /// A service was passed for unloading.
    Unload(String),
    /// A service was passed for restarting.
    Restart(String),
}

/// A recording service manager for tests that never invokes launchctl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandIn {
    uid: u32,
    session_ok: bool,
    disabled_unreadable: bool,
    stuck_unloads: bool,
    loads_despite_failure: bool,
    disabled: BTreeSet<String>,
    state: RefCell<Recorded>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Recorded {
    loaded: BTreeMap<String, State>,
    failing_loads: u32,
    erroring_loads: u32,
    calls: Vec<Call>,
}

impl Default for StandIn {
    fn default() -> Self {
        Self::new()
    }
}

impl StandIn {
    /// Creates a manager for uid 0 with an available session and no loaded or disabled services.
    pub fn new() -> Self {
        Self {
            uid: 0,
            session_ok: true,
            disabled_unreadable: false,
            stuck_unloads: false,
            loads_despite_failure: false,
            disabled: BTreeSet::new(),
            state: RefCell::default(),
        }
    }

    /// Sets the user whose service domain this manager drives.
    pub fn with_uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self
    }

    /// Makes the next loads return a bootstrap error before any false results.
    pub fn erroring_loads(mut self, n: u32) -> Self {
        self.state.get_mut().erroring_loads = n;
        self
    }

    /// Makes a configured load failure leave the service loaded and idle.
    pub fn loads_despite_failure(mut self) -> Self {
        self.loads_despite_failure = true;
        self
    }

    /// Makes this manager report that its session is unavailable.
    pub fn without_session(mut self) -> Self {
        self.session_ok = false;
        self
    }

    /// Sets a service's state before the test, removing it for `NotLoaded`.
    pub fn with_loaded(mut self, label: &str, state: State) -> Self {
        let loaded = &mut self.state.get_mut().loaded;
        if state == State::NotLoaded {
            loaded.remove(label);
        } else {
            loaded.insert(label.into(), state);
        }
        self
    }

    /// Marks a label as disabled.
    pub fn with_disabled(mut self, label: &str) -> Self {
        self.disabled.insert(label.into());
        self
    }

    /// Makes queries of disabled flags fail.
    pub fn with_disabled_unreadable(mut self) -> Self {
        self.disabled_unreadable = true;
        self
    }

    /// Makes the next `n` attempts to load a service return false.
    pub fn failing_loads(mut self, n: u32) -> Self {
        self.state.get_mut().failing_loads = n;
        self
    }

    /// Makes unloading a loaded service report success while leaving it loaded.
    pub fn stuck_unloads(mut self) -> Self {
        self.stuck_unloads = true;
        self
    }

    /// Returns the recorded calls in the order they occurred.
    pub fn calls(&self) -> Vec<Call> {
        self.state.borrow().calls.clone()
    }

    /// Looks up a service state without recording a call.
    pub fn state_of(&self, label: &str) -> State {
        self.state
            .borrow()
            .loaded
            .get(label)
            .copied()
            .unwrap_or(State::NotLoaded)
    }
}

impl Manager for StandIn {
    fn uid(&self) -> u32 {
        self.uid
    }

    fn session_ok(&self) -> Result<bool, ManagerError> {
        self.state.borrow_mut().calls.push(Call::SessionOk);
        Ok(self.session_ok)
    }

    fn state(&self, label: &str) -> Result<State, ManagerError> {
        self.state
            .borrow_mut()
            .calls
            .push(Call::State(label.into()));
        Ok(self.state_of(label))
    }

    fn disabled(&self, label: &str) -> Result<bool, ManagerError> {
        self.state
            .borrow_mut()
            .calls
            .push(Call::Disabled(label.into()));
        if self.disabled_unreadable {
            return Err(ManagerError::Failed {
                verb: "print-disabled",
                code: Some(1),
                stderr: String::new(),
            });
        }
        Ok(self.disabled.contains(label))
    }

    fn load(&self, plist: &Path) -> Result<bool, ManagerError> {
        let mut state = self.state.borrow_mut();
        state.calls.push(Call::Load(plist.into()));
        let label = plist.file_name().and_then(|name| name.to_str());
        let label = label.map(|name| name.strip_suffix(".plist").unwrap_or(name));
        let failure = if state.erroring_loads > 0 {
            state.erroring_loads -= 1;
            Some(Err(ManagerError::Failed {
                verb: "bootstrap",
                code: Some(5),
                stderr: String::new(),
            }))
        } else if state.failing_loads > 0 {
            state.failing_loads -= 1;
            Some(Ok(false))
        } else {
            None
        };
        if let Some(result) = failure {
            if self.loads_despite_failure
                && let Some(label) = label
            {
                state.loaded.insert(label.into(), State::Idle);
            }
            return result;
        }
        let Some(label) = label else {
            return Ok(false);
        };
        if state.loaded.contains_key(label) {
            return Ok(false);
        }
        state.loaded.insert(label.into(), State::Idle);
        Ok(true)
    }

    fn unload(&self, label: &str) -> Result<bool, ManagerError> {
        let mut state = self.state.borrow_mut();
        state.calls.push(Call::Unload(label.into()));
        if self.stuck_unloads {
            return Ok(state.loaded.contains_key(label));
        }
        Ok(state.loaded.remove(label).is_some())
    }

    fn restart(&self, label: &str) -> Result<bool, ManagerError> {
        let mut state = self.state.borrow_mut();
        state.calls.push(Call::Restart(label.into()));
        Ok(state.loaded.contains_key(label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_failures_and_state_changes_are_recorded_in_order() {
        let label = "com.example.alpha";
        let path = Path::new("/var/tmp/example/com.example.alpha.plist");
        let manager = StandIn::new()
            .without_session()
            .with_loaded(label, State::Running { pid: Some(42) })
            .with_loaded(label, State::NotLoaded)
            .with_disabled(label)
            .failing_loads(1)
            .stuck_unloads();

        assert_eq!(manager.state_of(label), State::NotLoaded);
        assert!(!manager.session_ok().unwrap());
        assert!(!manager.restart(label).unwrap());
        assert!(!manager.unload(label).unwrap());
        assert!(manager.disabled(label).unwrap());
        assert!(!manager.load(path).unwrap());
        assert!(manager.load(path).unwrap());
        assert_eq!(manager.state(label).unwrap(), State::Idle);
        assert!(!manager.load(path).unwrap());
        assert!(manager.restart(label).unwrap());
        assert!(manager.unload(label).unwrap());
        assert_eq!(manager.state_of(label), State::Idle);
        assert_eq!(
            manager.calls(),
            [
                Call::SessionOk,
                Call::Restart(label.into()),
                Call::Unload(label.into()),
                Call::Disabled(label.into()),
                Call::Load(path.into()),
                Call::Load(path.into()),
                Call::State(label.into()),
                Call::Load(path.into()),
                Call::Restart(label.into()),
                Call::Unload(label.into()),
            ]
        );
    }
}
