//! Win32 calls. Every `unsafe` block is small and wrapped by a safe function.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HRESULT, HSTRING, PCWSTR, w};

use crate::elevation::ElevationError;

/// Closes a Win32 handle when dropped, so early returns cannot leak it.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own this handle and close it exactly once.
            // A failed close cannot be acted on during drop; the handle is gone either way.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

pub fn run_elevated_and_wait(exe: &Path, args: &str) -> Result<u32, ElevationError> {
    let file = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>())
            .map_err(|error| ElevationError::Launch(error.to_string()))?,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    // SAFETY: `info` is fully initialised and the strings it points to outlive the call.
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
            return Err(ElevationError::Declined);
        }
        return Err(ElevationError::Launch(error.message()));
    }
    let process = OwnedHandle(info.hProcess);
    if process.0.is_invalid() {
        return Err(ElevationError::Launch(
            "no process handle returned".to_owned(),
        ));
    }

    // SAFETY: `process` is a valid handle owned by us until dropped.
    unsafe { WaitForSingleObject(process.0, INFINITE) };
    let mut exit_code = 0u32;
    // SAFETY: valid process handle and a valid out-pointer.
    unsafe { GetExitCodeProcess(process.0, &mut exit_code) }
        .map_err(|error| ElevationError::Launch(error.message()))?;
    Ok(exit_code)
}

pub fn is_elevated() -> Result<bool, ElevationError> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo-handle from GetCurrentProcess is always valid; `token` is an out-param.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|error| ElevationError::Launch(error.message()))?;
    let token = OwnedHandle(token);

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>())
        .map_err(|error| ElevationError::Launch(error.to_string()))?;
    // SAFETY: the buffer is a properly sized TOKEN_ELEVATION owned by this frame.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast::<c_void>()),
            size,
            &mut returned,
        )
    }
    .map_err(|error| ElevationError::Launch(error.message()))?;
    Ok(elevation.TokenIsElevated != 0)
}

// ----- Processes -------------------------------------------------------------------------------

use std::collections::HashSet;
use std::time::{Duration, Instant};

use examsafe_core::apps::{ProcessDetails, RunningProcess};
use examsafe_core::ports::ProcessError;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_SUCCESS, HWND, LPARAM, WAIT_OBJECT_0,
    WPARAM,
};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    WM_CLOSE,
};
use windows::core::{BOOL, PWSTR};

fn is_win32(error: &windows::core::Error, code: u32) -> bool {
    error.code() == HRESULT::from_win32(code)
}

fn wide_to_string(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

pub fn list_processes() -> Result<Vec<RunningProcess>, ProcessError> {
    // SAFETY: plain snapshot call; the handle is owned and closed by OwnedHandle.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|error| ProcessError::Os(error.message()))?;
    let snapshot = OwnedHandle(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
            .map_err(|error| ProcessError::Os(error.to_string()))?,
        ..Default::default()
    };
    let mut processes = Vec::new();
    // SAFETY: `entry` is initialised with its size, as the API requires.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok();
    while more {
        processes.push(RunningProcess {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            exe_name: wide_to_string(&entry.szExeFile),
        });
        // SAFETY: same snapshot and entry as above.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) }.is_ok();
    }
    Ok(processes)
}

pub fn describe_process(pid: u32) -> ProcessDetails {
    // SAFETY: query-only access; failure just means "no details".
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return ProcessDetails::default();
    };
    let handle = OwnedHandle(handle);

    let mut path_buffer = [0u16; 1024];
    let mut path_len = u32::try_from(path_buffer.len()).unwrap_or(0);
    // SAFETY: buffer and length describe the same writable array.
    let path = unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(path_buffer.as_mut_ptr()),
            &mut path_len,
        )
    }
    .ok()
    .map(|()| String::from_utf16_lossy(&path_buffer[..path_len as usize]));

    let mut id_buffer = [0u16; 512];
    let mut id_len = u32::try_from(id_buffer.len()).unwrap_or(0);
    // SAFETY: buffer and length describe the same writable array.
    let status = unsafe {
        GetApplicationUserModelId(handle.0, &mut id_len, Some(PWSTR(id_buffer.as_mut_ptr())))
    };
    let app_id = (status == ERROR_SUCCESS)
        .then(|| wide_to_string(&id_buffer))
        .filter(|id| !id.is_empty());

    ProcessDetails { path, app_id }
}

unsafe extern "system" fn close_window_if_targeted(window: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the address of the HashSet passed by `request_close`, alive for the
    // duration of EnumWindows.
    let targets = unsafe { &*(lparam.0 as *const HashSet<u32>) };
    let mut pid = 0u32;
    // SAFETY: `window` comes from EnumWindows; `pid` is a valid out-pointer.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    // Only visible top-level windows without an owner: the app's main windows.
    // SAFETY: plain queries on a window handle supplied by EnumWindows.
    let is_main_window = unsafe { IsWindowVisible(window) }.as_bool()
        && unsafe { GetWindow(window, GW_OWNER) }.map_or(true, |owner| owner.is_invalid());
    if targets.contains(&pid) && is_main_window {
        // Like clicking the X. A failed post just means the process gets force-closed later.
        // SAFETY: posting a message to a valid window handle.
        let _ = unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
    BOOL::from(true)
}

pub fn request_close(pids: &[u32]) {
    let targets: HashSet<u32> = pids.iter().copied().collect();
    // Best effort by contract: if enumeration fails, apps are force-closed after the grace period.
    // SAFETY: the callback only reads `targets`, which outlives this call.
    let _ = unsafe {
        EnumWindows(
            Some(close_window_if_targeted),
            LPARAM(std::ptr::from_ref(&targets) as isize),
        )
    };
}

pub fn wait_for_exit(pids: &[u32], timeout: Duration) -> Vec<u32> {
    let deadline = Instant::now() + timeout;
    let mut alive = Vec::new();
    for &pid in pids {
        // SAFETY: synchronize-only access, handle owned below.
        let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(handle) => OwnedHandle(handle),
            // No such process any more.
            Err(error) if is_win32(&error, ERROR_INVALID_PARAMETER.0) => continue,
            // Can't even watch it (e.g. elevated process): assume it is still there.
            Err(_) => {
                alive.push(pid);
                continue;
            }
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        let millis = u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: valid process handle.
        if unsafe { WaitForSingleObject(handle.0, millis) } != WAIT_OBJECT_0 {
            alive.push(pid);
        }
    }
    alive
}

pub fn terminate(pid: u32) -> Result<(), ProcessError> {
    let map_error = |error: windows::core::Error| {
        if is_win32(&error, ERROR_ACCESS_DENIED.0) {
            ProcessError::AccessDenied
        } else {
            ProcessError::Os(error.message())
        }
    };
    // SAFETY: terminate + synchronize access, handle owned below.
    let handle = match unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(handle) => OwnedHandle(handle),
        Err(error) if is_win32(&error, ERROR_INVALID_PARAMETER.0) => return Ok(()),
        Err(error) => return Err(map_error(error)),
    };
    // SAFETY: valid handle with terminate rights.
    match unsafe { TerminateProcess(handle.0, 1) } {
        Ok(()) => Ok(()),
        // Windows answers "access denied" for a process that has already exited but whose
        // handle someone still holds. Gone is what we wanted, so that counts as success.
        // SAFETY: valid handle with synchronize rights; zero timeout only polls.
        Err(_) if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 => Ok(()),
        Err(error) => Err(map_error(error)),
    }
}

/// Lets a GUI-subsystem exe print to the terminal it was started from. Does nothing when output
/// is already redirected (to a file or pipe). Returns whether output has somewhere to go.
pub fn attach_parent_console() -> bool {
    // SAFETY: no pointers involved.
    let has_output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }
        .is_ok_and(|handle| !handle.is_invalid() && !handle.0.is_null());
    // SAFETY: no pointers involved.
    has_output || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok()
}
