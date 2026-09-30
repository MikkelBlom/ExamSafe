//! [`ProcessControl`] for Windows: list, close and start programs.

use std::time::Duration;

use examsafe_core::apps::{LaunchTarget, ProcessDetails, RunningProcess};
use examsafe_core::ports::{ProcessControl, ProcessError};

use crate::windows_impl;

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsProcessControl;

impl ProcessControl for WindowsProcessControl {
    fn list(&self) -> Result<Vec<RunningProcess>, ProcessError> {
        windows_impl::list_processes()
    }

    fn describe(&self, pid: u32) -> ProcessDetails {
        windows_impl::describe_process(pid)
    }

    fn request_close(&self, pids: &[u32]) {
        windows_impl::request_close(pids);
    }

    fn wait_for_exit(&self, pids: &[u32], timeout: Duration) -> Vec<u32> {
        windows_impl::wait_for_exit(pids, timeout)
    }

    fn terminate(&self, pid: u32) -> Result<(), ProcessError> {
        windows_impl::terminate(pid)
    }

    /// Starts through explorer.exe so the app runs as the normal user, exactly as if it had been
    /// clicked — never as a child of ExamSafe.
    fn launch(&self, target: &LaunchTarget) -> Result<(), ProcessError> {
        let argument = match target {
            LaunchTarget::Path(path) => path.clone(),
            LaunchTarget::AppId(app_id) => format!("shell:AppsFolder\\{app_id}"),
        };
        std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn()
            .map(|_| ())
            .map_err(|error| ProcessError::Os(error.to_string()))
    }
}

/// See [`windows_impl::attach_parent_console`].
pub fn attach_parent_console() -> bool {
    windows_impl::attach_parent_console()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    fn spawn_sleeper() -> Child {
        // A window-less process we own: ping waits ~30 s.
        Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("ping should start")
    }

    #[test]
    fn lists_our_own_process() {
        let own = std::process::id();
        let processes = WindowsProcessControl.list().unwrap();
        let me = processes
            .iter()
            .find(|p| p.pid == own)
            .expect("own pid listed");
        assert!(me.exe_name.to_lowercase().ends_with(".exe"));
    }

    #[test]
    fn describes_our_own_path() {
        let details = WindowsProcessControl.describe(std::process::id());
        let path = details.path.expect("own path readable");
        let exe = std::env::current_exe().unwrap();
        assert!(path.eq_ignore_ascii_case(&exe.to_string_lossy()));
        assert!(details.app_id.is_none());
    }

    /// Needs an interactive desktop (opens a real window for a few seconds), so it is opt-in:
    /// `cargo test -p examsafe-platform -- --include-ignored`.
    #[test]
    #[ignore = "opens a real window; run locally with --include-ignored"]
    fn politely_closes_a_real_window_without_force() {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
                      $form = New-Object Windows.Forms.Form; $form.Text = 'ExamSafe test window'; \
                      [Windows.Forms.Application]::Run($form)";
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", script])
            .spawn()
            .expect("powershell should start");
        let pid = child.id();
        let control = WindowsProcessControl;
        // The window takes a moment to appear; keep asking until it closes (max ~15 s).
        let mut closed = false;
        for _ in 0..30 {
            control.request_close(&[pid]);
            if control
                .wait_for_exit(&[pid], Duration::from_millis(500))
                .is_empty()
            {
                closed = true;
                break;
            }
        }
        if !closed {
            control.terminate(pid).unwrap();
        }
        let status = child.wait().unwrap();
        assert!(closed, "window did not close on request");
        assert!(status.success(), "closed politely, not killed: {status:?}");
    }

    #[test]
    fn terminates_a_real_process_and_sees_it_exit() {
        let mut child = spawn_sleeper();
        let pid = child.id();
        let control = WindowsProcessControl;
        assert_eq!(
            control.wait_for_exit(&[pid], Duration::from_millis(50)),
            vec![pid]
        );
        control.terminate(pid).unwrap();
        assert!(
            control
                .wait_for_exit(&[pid], Duration::from_secs(5))
                .is_empty()
        );
        child.wait().unwrap();
        // Terminating something that is already gone is fine.
        control.terminate(pid).unwrap();
    }
}
