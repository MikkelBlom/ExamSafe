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
