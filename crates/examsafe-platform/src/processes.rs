//! [`ProcessControl`] for Windows: list, close and start programs.

use std::time::Duration;

use examsafe_core::apps::{LaunchSpec, LaunchTarget, ProcessDetails, RunningProcess};
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

    fn has_visible_window(&self, pids: &[u32]) -> bool {
        windows_impl::has_visible_window(pids)
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

    /// Starts the app with its original arguments and folder, as the normal user. Programs are
    /// started by the desktop's Explorer (never as ExamSafe's child, never with ExamSafe's
    /// rights); Store apps through Windows' app activation, which passes their arguments.
    fn launch(&self, spec: &LaunchSpec) -> Result<(), ProcessError> {
        // A folder that no longer exists would make the start fail; the app's default is better.
        let working_dir = spec
            .working_dir
            .as_deref()
            .filter(|dir| std::path::Path::new(dir).is_dir());
        let elevated = crate::elevation::is_elevated().unwrap_or(true);
        match &spec.target {
            LaunchTarget::Path(path) => {
                let shell = windows_impl::launch_via_desktop_shell(
                    path,
                    spec.arguments.as_deref(),
                    working_dir,
                    spec.background,
                );
                match shell {
                    // Without a desktop Explorer (rare: another shell), start it ourselves, but
                    // only when that cannot hand it admin rights.
                    Err(_) if !elevated => windows_impl::launch_path(
                        path,
                        spec.arguments.as_deref(),
                        working_dir,
                        spec.background,
                    ),
                    other => other,
                }
            }
            // App activation refuses elevated callers; Explorer can start it, without arguments.
            LaunchTarget::AppId(app_id) if elevated => windows_impl::launch_via_desktop_shell(
                &format!("shell:AppsFolder\\{app_id}"),
                None,
                None,
                spec.background,
            ),
            LaunchTarget::AppId(app_id) => {
                windows_impl::activate_app(app_id, spec.arguments.as_deref())
            }
        }
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

    #[test]
    fn reads_command_line_and_folder_of_another_process() {
        let folder = std::env::temp_dir();
        let mut child = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .current_dir(&folder)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("ping should start");
        let details = WindowsProcessControl.describe(child.id());
        child.kill().unwrap();
        child.wait().unwrap();

        let line = details.command_line.expect("command line readable");
        assert_eq!(
            examsafe_core::apps::command_line_arguments(&line).as_deref(),
            Some("-n 30 127.0.0.1")
        );
        let dir = details.working_dir.expect("working dir readable");
        let normalise = |p: &str| p.trim_end_matches('\\').to_lowercase();
        assert_eq!(normalise(&dir), normalise(&folder.to_string_lossy()));
    }

    #[test]
    fn a_console_less_background_process_has_no_visible_window() {
        let mut child = spawn_sleeper();
        assert!(!WindowsProcessControl.has_visible_window(&[child.id()]));
        child.kill().unwrap();
        child.wait().unwrap();
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
