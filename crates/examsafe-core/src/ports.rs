//! Ports: the only ways the domain reaches the outside world. Adapters live in
//! `examsafe-platform`; tests supply fakes.

use std::time::Duration;

use crate::apps::{LaunchTarget, ProcessDetails, RunningProcess};
use crate::exam_mode::ExamModeRecord;
use crate::preferences::Preferences;
use crate::protocol::{HelperAction, HelperResponse};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("could not access ExamSafe's saved data: {0}")]
    Io(String),
    #[error("ExamSafe's saved data is unreadable: {0}")]
    Corrupt(String),
}

/// Persists the exam-mode journal (what was changed, so it can be restored).
pub trait ExamModeRepository: Send + Sync {
    fn load(&self) -> Result<Option<ExamModeRecord>, StoreError>;
    fn save(&self, record: &ExamModeRecord) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
}

/// Persists user preferences. A missing file means defaults.
pub trait PreferencesRepository: Send + Sync {
    fn load(&self) -> Result<Preferences, StoreError>;
    fn save(&self, preferences: &Preferences) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    #[error("Windows denied access (it may be running as administrator)")]
    AccessDenied,
    #[error("{0}")]
    Os(String),
}

/// Listing, closing and starting programs.
pub trait ProcessControl: Send + Sync {
    fn list(&self) -> Result<Vec<RunningProcess>, ProcessError>;
    /// Best effort: missing details are simply `None`.
    fn describe(&self, pid: u32) -> ProcessDetails;
    /// Politely asks the processes' windows to close (like clicking the X). Best effort.
    fn request_close(&self, pids: &[u32]);
    /// Waits up to `timeout` in total; returns the pids that are still running.
    fn wait_for_exit(&self, pids: &[u32], timeout: Duration) -> Vec<u32>;
    /// Force-closes one process. Succeeds if it is already gone.
    fn terminate(&self, pid: u32) -> Result<(), ProcessError>;
    /// Starts an app as the normal (non-admin) user.
    fn launch(&self, target: &LaunchTarget) -> Result<(), ProcessError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HelperError {
    #[error("administrator permission was declined")]
    Declined,
    #[error("could not start the admin helper: {0}")]
    Launch(String),
    #[error("the admin helper failed: {0}")]
    Failed(String),
    #[error("the admin helper is not supported on this platform")]
    Unsupported,
}

/// Runs an action with administrator rights (via ExamSafe's helper mode).
pub trait PrivilegedExecutor: Send + Sync {
    fn execute(&self, action: HelperAction) -> Result<HelperResponse, HelperError>;
}
