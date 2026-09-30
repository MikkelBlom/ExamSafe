//! [`PrivilegedExecutor`] that runs actions through the `examsafe-helper` process.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use examsafe_core::ports::{HelperError, PrivilegedExecutor};
use examsafe_core::protocol::{
    HelperAction, HelperRequest, HelperResponse, PROTOCOL_VERSION, RESPONSE_FILE_PREFIX,
    encode_request,
};

use crate::elevation::{ElevationError, run_elevated_and_wait};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Normal: UAC prompt, helper runs as administrator.
    Elevated,
    /// Development only: start the helper without elevation (no UAC prompt).
    Direct,
}

#[derive(Debug, Clone)]
pub struct HelperClient {
    helper_exe: PathBuf,
    mode: LaunchMode,
}

pub const HELPER_FILE_NAME: &str = if cfg!(windows) {
    "examsafe-helper.exe"
} else {
    "examsafe-helper"
};

impl HelperClient {
    pub fn new(helper_exe: impl Into<PathBuf>, mode: LaunchMode) -> Self {
        Self {
            helper_exe: helper_exe.into(),
            mode,
        }
    }

    /// The helper ships next to the app executable.
    pub fn next_to_current_exe(mode: LaunchMode) -> std::io::Result<Self> {
        let exe = std::env::current_exe()?;
        Ok(Self::new(exe.with_file_name(HELPER_FILE_NAME), mode))
    }

    fn launch(&self, encoded: &str) -> Result<u32, HelperError> {
        match self.mode {
            LaunchMode::Elevated => {
                run_elevated_and_wait(&self.helper_exe, &format!("--request {encoded}")).map_err(
                    |error| match error {
                        ElevationError::Declined => HelperError::Declined,
                        ElevationError::Unsupported => HelperError::Unsupported,
                        ElevationError::Launch(message) => HelperError::Launch(message),
                    },
                )
            }
            LaunchMode::Direct => run_direct(&self.helper_exe, encoded),
        }
    }
}

fn run_direct(exe: &Path, encoded: &str) -> Result<u32, HelperError> {
    let mut command = std::process::Command::new(exe);
    command.arg("--request").arg(encoded);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let status = command
        .status()
        .map_err(|error| HelperError::Launch(error.to_string()))?;
    // A signal-terminated process has no code; report it as a generic failure code.
    Ok(status.code().map_or(u32::MAX, |code| code.cast_unsigned()))
}

fn unique_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

impl PrivilegedExecutor for HelperClient {
    fn execute(&self, action: HelperAction) -> Result<HelperResponse, HelperError> {
        let id = unique_id();
        let response_path = std::env::temp_dir().join(format!("{RESPONSE_FILE_PREFIX}{id}.json"));
        let request = HelperRequest {
            protocol: PROTOCOL_VERSION,
            id: id.clone(),
            action,
            response_path: response_path.to_string_lossy().into_owned(),
        };
        let encoded =
            encode_request(&request).map_err(|error| HelperError::Failed(error.to_string()))?;

        let exit_code = self.launch(&encoded)?;
        let body = std::fs::read_to_string(&response_path).map_err(|error| {
            HelperError::Failed(format!("no response (exit code {exit_code}): {error}"))
        })?;
        std::fs::remove_file(&response_path).map_err(|error| {
            HelperError::Failed(format!("could not remove the response file: {error}"))
        })?;

        let response: HelperResponse = serde_json::from_str(&body)
            .map_err(|error| HelperError::Failed(format!("unreadable response: {error}")))?;
        if response.id != id {
            return Err(HelperError::Failed(
                "response does not match the request".to_owned(),
            ));
        }
        if !response.ok {
            return Err(HelperError::Failed(response.message));
        }
        Ok(response)
    }
}
