//! Ports: the only ways the domain reaches the outside world. Adapters live in
//! `examsafe-platform`; tests can supply fakes.

use crate::exam_mode::ExamModeRecord;
use crate::protocol::{HelperAction, HelperResponse};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("could not access the exam-mode record: {0}")]
    Io(String),
    #[error("the exam-mode record is unreadable: {0}")]
    Corrupt(String),
}

/// Persists whether exam mode is on.
pub trait ExamModeRepository {
    fn load(&self) -> Result<Option<ExamModeRecord>, StoreError>;
    fn save(&self, record: &ExamModeRecord) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
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

/// Runs an action with administrator rights (via the separate helper process).
pub trait PrivilegedExecutor: Send + Sync {
    fn execute(&self, action: HelperAction) -> Result<HelperResponse, HelperError>;
}
