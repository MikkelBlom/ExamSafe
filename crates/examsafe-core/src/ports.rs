//! Ports: the only ways the domain reaches the outside world. Adapters live in
//! `examsafe-platform`; tests supply fakes.

use std::time::Duration;

use crate::apps::{LaunchSpec, ProcessDetails, RunningProcess};
use crate::exam_mode::ExamModeRecord;
use crate::inventory::{ListeningPort, NetworkAdapter, StartupItem};
use crate::preferences::Preferences;
use crate::protocol::{HelperAction, HelperResponse};
use crate::system::{ServiceInfo, TaskInfo};

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
    /// Whether any of these processes shows a visible main window (a tray-only app does not).
    fn has_visible_window(&self, pids: &[u32]) -> bool;
    /// Politely asks the processes' windows to close (like clicking the X). Best effort.
    fn request_close(&self, pids: &[u32]);
    /// Waits up to `timeout` in total; returns the pids that are still running.
    fn wait_for_exit(&self, pids: &[u32], timeout: Duration) -> Vec<u32>;
    /// Force-closes one process. Succeeds if it is already gone.
    fn terminate(&self, pid: u32) -> Result<(), ProcessError>;
    /// Starts an app as the normal (non-admin) user, with the spec's arguments and folder, and
    /// without showing a window when `spec.background` is set.
    fn launch(&self, spec: &LaunchSpec) -> Result<(), ProcessError>;
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not read {what}: {reason}")]
pub struct InventoryError {
    pub what: &'static str,
    pub reason: String,
}

/// Read-only view of what is installed and running besides apps. Needs no admin rights.
pub trait SystemInventory: Send + Sync {
    fn services(&self) -> Result<Vec<ServiceInfo>, InventoryError>;
    fn tasks(&self) -> Result<Vec<TaskInfo>, InventoryError>;
    fn startup_items(&self) -> Result<Vec<StartupItem>, InventoryError>;
    fn listening_ports(&self) -> Result<Vec<ListeningPort>, InventoryError>;
    fn network_adapters(&self) -> Result<Vec<NetworkAdapter>, InventoryError>;
}
