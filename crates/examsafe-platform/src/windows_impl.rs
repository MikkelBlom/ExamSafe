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
use windows::Wdk::System::Threading::{
    NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation,
};
use windows::Win32::Foundation::UNICODE_STRING;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_SUCCESS, HWND, LPARAM, WAIT_OBJECT_0,
    WPARAM,
};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ, QueryFullProcessImageNameW,
    TerminateProcess,
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

    ProcessDetails {
        path,
        app_id,
        command_line: command_line(&handle),
        working_dir: working_dir(pid),
    }
}

/// The process's full command line. Needs only limited query rights (Windows 8.1+).
fn command_line(process: &OwnedHandle) -> Option<String> {
    let mut needed = 0u32;
    // SAFETY: a zero-length query only reports the size needed.
    let _ = unsafe {
        NtQueryInformationProcess(
            process.0,
            ProcessCommandLineInformation,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return None;
    }
    // u64 elements keep the UNICODE_STRING header correctly aligned.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `needed` bytes and lives until we are done reading it.
    let status = unsafe {
        NtQueryInformationProcess(
            process.0,
            ProcessCommandLineInformation,
            buffer.as_mut_ptr().cast::<c_void>(),
            needed,
            &mut needed,
        )
    };
    if status.is_err() {
        return None;
    }
    // SAFETY: on success the buffer starts with a UNICODE_STRING whose Buffer points into the
    // same allocation, `Length` bytes long.
    let text = unsafe {
        let header = &*buffer.as_ptr().cast::<UNICODE_STRING>();
        if header.Buffer.is_null() {
            return None;
        }
        std::slice::from_raw_parts(header.Buffer.0, usize::from(header.Length) / 2)
    };
    Some(String::from_utf16_lossy(text)).filter(|line| !line.trim().is_empty())
}

/// The process's current directory, read from its process parameters. Needs read access to the
/// process memory, which Windows refuses for elevated and protected processes (then `None`).
#[cfg(target_pointer_width = "64")]
fn working_dir(pid: u32) -> Option<String> {
    // Offsets in the 64-bit PEB / RTL_USER_PROCESS_PARAMETERS layout (stable since Windows XP).
    const PEB_PROCESS_PARAMETERS: usize = 0x20;
    const PARAMS_CURRENT_DIRECTORY: usize = 0x38;

    // SAFETY: query + read access; the handle is owned below.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
    }
    .ok()?;
    let handle = OwnedHandle(handle);
    let mut basic = PROCESS_BASIC_INFORMATION::default();
    let size = u32::try_from(std::mem::size_of::<PROCESS_BASIC_INFORMATION>()).ok()?;
    let mut returned = 0u32;
    // SAFETY: `basic` is a correctly sized out-buffer.
    let status = unsafe {
        NtQueryInformationProcess(
            handle.0,
            ProcessBasicInformation,
            std::ptr::from_mut(&mut basic).cast::<c_void>(),
            size,
            &mut returned,
        )
    };
    if status.is_err() || basic.PebBaseAddress.is_null() {
        return None;
    }
    let parameters = read_remote_u64(
        &handle,
        basic.PebBaseAddress as usize + PEB_PROCESS_PARAMETERS,
    )?;
    let directory = usize::try_from(parameters).ok()? + PARAMS_CURRENT_DIRECTORY;
    // UNICODE_STRING: u16 Length, u16 MaximumLength, 4 bytes padding, u64 Buffer.
    let lengths = read_remote_u64(&handle, directory)?;
    let length = usize::try_from(lengths & 0xffff).ok()?;
    let text_address = usize::try_from(read_remote_u64(&handle, directory + 8)?).ok()?;
    if length == 0 || length > 32 * 1024 || text_address == 0 {
        return None;
    }
    let mut text = vec![0u16; length / 2];
    // SAFETY: reading `length` bytes into a buffer of exactly that size.
    unsafe {
        ReadProcessMemory(
            handle.0,
            text_address as *const c_void,
            text.as_mut_ptr().cast::<c_void>(),
            length,
            None,
        )
    }
    .ok()?;
    Some(String::from_utf16_lossy(&text))
}

#[cfg(not(target_pointer_width = "64"))]
fn working_dir(_pid: u32) -> Option<String> {
    None
}

fn read_remote_u64(process: &OwnedHandle, address: usize) -> Option<u64> {
    let mut value = 0u64;
    // SAFETY: reading 8 bytes into a u64 owned by this frame.
    unsafe {
        ReadProcessMemory(
            process.0,
            address as *const c_void,
            std::ptr::from_mut(&mut value).cast::<c_void>(),
            8,
            None,
        )
    }
    .ok()?;
    Some(value)
}

/// Visible top-level windows without an owner: an app's main windows.
fn is_main_window(window: HWND) -> bool {
    // SAFETY: plain queries on a window handle supplied by EnumWindows.
    unsafe { IsWindowVisible(window) }.as_bool()
        && unsafe { GetWindow(window, GW_OWNER) }.map_or(true, |owner| owner.is_invalid())
}

fn window_pid(window: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `window` comes from EnumWindows; `pid` is a valid out-pointer.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    pid
}

struct WindowSearch {
    targets: HashSet<u32>,
    found: bool,
}

unsafe extern "system" fn find_main_window(window: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the address of the WindowSearch owned by `has_visible_window`, alive and
    // not otherwise borrowed for the duration of EnumWindows.
    let search = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
    if search.targets.contains(&window_pid(window)) && is_main_window(window) {
        search.found = true;
        return BOOL::from(false);
    }
    BOOL::from(true)
}

pub fn has_visible_window(pids: &[u32]) -> bool {
    let mut search = WindowSearch {
        targets: pids.iter().copied().collect(),
        found: false,
    };
    // EnumWindows reports an "error" when the callback stops early; `found` is what counts.
    // SAFETY: the callback only touches `search`, which outlives this call.
    let _ = unsafe {
        EnumWindows(
            Some(find_main_window),
            LPARAM(std::ptr::from_mut(&mut search) as isize),
        )
    };
    search.found
}

unsafe extern "system" fn close_window_if_targeted(window: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the address of the HashSet passed by `request_close`, alive for the
    // duration of EnumWindows.
    let targets = unsafe { &*(lparam.0 as *const HashSet<u32>) };
    if targets.contains(&window_pid(window)) && is_main_window(window) {
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

// ----- Starting apps ---------------------------------------------------------------------------

use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    AO_NONE, ApplicationActivationManager, IApplicationActivationManager, SEE_MASK_FLAG_NO_UI,
    SEE_MASK_NOASYNC,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Starts a program with its arguments and folder. A background app (no visible window when it
/// was closed) is started hidden, as it was; tray icons still appear, and most such apps also get
/// their "start in background" flag back through `arguments`.
pub fn launch_path(
    path: &str,
    arguments: Option<&str>,
    working_dir: Option<&str>,
    background: bool,
) -> Result<(), ProcessError> {
    let file = HSTRING::from(path);
    let parameters = arguments.map(HSTRING::from);
    let directory = working_dir.map(HSTRING::from);
    let pointer =
        |text: &Option<HSTRING>| text.as_ref().map_or(PCWSTR::null(), |t| PCWSTR(t.as_ptr()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>())
            .map_err(|error| ProcessError::Os(error.to_string()))?,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: pointer(&parameters),
        lpDirectory: pointer(&directory),
        nShow: if background {
            SW_HIDE.0
        } else {
            SW_SHOWNORMAL.0
        },
        ..Default::default()
    };
    // SAFETY: `info` is fully initialised and the strings it points to outlive the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|error| ProcessError::Os(error.message()))
}

/// Starts a Store (packaged) app by its app id, with arguments. Packaged apps cannot be started
/// by path.
pub fn activate_app(app_id: &str, arguments: Option<&str>) -> Result<(), ProcessError> {
    let to_error = |error: windows::core::Error| ProcessError::Os(error.message());
    // SAFETY: initialises COM for this thread; balanced by CoUninitialize below when it succeeds.
    let initialised = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        // SAFETY: standard COM activation of a system class.
        let manager: IApplicationActivationManager =
            unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) }
                .map_err(to_error)?;
        let id = HSTRING::from(app_id);
        let arguments = HSTRING::from(arguments.unwrap_or_default());
        // SAFETY: both strings outlive the call.
        unsafe { manager.ActivateApplication(&id, &arguments, AO_NONE) }
            .map(|_| ())
            .map_err(to_error)
    })();
    if initialised {
        // SAFETY: balances the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

use windows::Win32::System::Com::{IDispatch, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
    SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::{BSTR, Interface};

/// Asks the desktop's Explorer to start the program, exactly as if the user had started it:
/// it runs as the normal (non-admin) user, is not a child of ExamSafe, and gets its arguments,
/// folder and window state. This is the documented way to start an unelevated program.
pub fn launch_via_desktop_shell(
    file: &str,
    arguments: Option<&str>,
    working_dir: Option<&str>,
    background: bool,
) -> Result<(), ProcessError> {
    let to_error = |error: windows::core::Error| ProcessError::Os(error.message());
    // SAFETY: initialises COM for this thread; balanced by CoUninitialize below when it succeeds.
    let initialised = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        // SAFETY: standard COM calls on interfaces we own; every out-value is checked.
        unsafe {
            let windows: IShellWindows =
                CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER).map_err(to_error)?;
            let location = VARIANT::from(i32::try_from(CSIDL_DESKTOP).unwrap_or_default());
            let mut desktop_window = 0i32;
            let desktop: IDispatch = windows
                .FindWindowSW(
                    &location,
                    &VARIANT::default(),
                    SWC_DESKTOP,
                    &mut desktop_window,
                    SWFO_NEEDDISPATCH,
                )
                .map_err(to_error)?;
            let provider: IServiceProvider = desktop.cast().map_err(to_error)?;
            let browser: IShellBrowser = provider
                .QueryService(&SID_STopLevelBrowser)
                .map_err(to_error)?;
            let view = browser.QueryActiveShellView().map_err(to_error)?;
            let background_object: IDispatch =
                view.GetItemObject(SVGIO_BACKGROUND).map_err(to_error)?;
            let folder_view: IShellFolderViewDual = background_object.cast().map_err(to_error)?;
            let shell: IShellDispatch2 = folder_view
                .Application()
                .map_err(to_error)?
                .cast()
                .map_err(to_error)?;
            let show = if background {
                SW_HIDE.0
            } else {
                SW_SHOWNORMAL.0
            };
            shell
                .ShellExecute(
                    &BSTR::from(file),
                    &VARIANT::from(arguments.unwrap_or_default()),
                    &VARIANT::from(working_dir.unwrap_or_default()),
                    &VARIANT::from("open"),
                    &VARIANT::from(show),
                )
                .map_err(to_error)
        }
    })();
    if initialised {
        // SAFETY: balances the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}
