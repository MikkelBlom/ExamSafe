//! Starting processes with administrator rights, and checking our own rights.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ElevationError {
    #[error("administrator permission was declined")]
    Declined,
    #[error("could not start the process: {0}")]
    Launch(String),
    #[error("not supported on this platform")]
    Unsupported,
}

/// Starts `exe` with administrator rights (UAC prompt on Windows), waits for it and returns its
/// exit code.
pub fn run_elevated_and_wait(exe: &Path, args: &str) -> Result<u32, ElevationError> {
    #[cfg(windows)]
    {
        crate::windows_impl::run_elevated_and_wait(exe, args)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, args);
        Err(ElevationError::Unsupported)
    }
}

/// Whether the current process runs with administrator rights.
pub fn is_elevated() -> Result<bool, ElevationError> {
    #[cfg(windows)]
    {
        crate::windows_impl::is_elevated()
    }
    #[cfg(not(windows))]
    {
        Err(ElevationError::Unsupported)
    }
}
