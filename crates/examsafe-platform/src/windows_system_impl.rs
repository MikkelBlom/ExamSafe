//! Win32 calls for services, scheduled tasks, startup entries, listening ports and network
//! adapters. Every `unsafe` block is small and wrapped by a safe function.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::time::{Duration, Instant};

use examsafe_core::inventory::{ListeningPort, NetworkAdapter, Protocol};
use examsafe_core::system::{ServiceInfo, ServiceState, StartType, TaskInfo};
use windows::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SERVICE_ALREADY_RUNNING,
    ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_NOT_ACTIVE, ERROR_SUCCESS, NO_ERROR, VARIANT_FALSE,
    VARIANT_TRUE,
};
use windows::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
    GetExtendedTcpTable, GetExtendedUdpTable, IP_ADAPTER_ADDRESSES_LH, MIB_TCP6ROW_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID,
    TCP_TABLE_OWNER_PID_LISTENER, UDP_TABLE_OWNER_PID,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, REG_BINARY, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegEnumValueW,
    RegOpenKeyExW,
};
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
    ENUM_SERVICE_STATUS_PROCESSW, ENUM_SERVICE_TYPE, EnumServicesStatusExW, OpenSCManagerW,
    OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfig2W, QueryServiceConfigW,
    QueryServiceStatus, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_ENUMERATE_SERVICE, SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_CHANGE_CONFIG,
    SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONTROL_STOP, SERVICE_DELAYED_AUTO_START_INFO,
    SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_ERROR, SERVICE_NO_CHANGE, SERVICE_PAUSED,
    SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_START_TYPE,
    SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED, SERVICE_SYSTEM_START,
    SERVICE_WIN32, StartServiceW,
};
use windows::Win32::System::TaskScheduler::{
    IRegisteredTask, ITaskFolder, ITaskService, TASK_ENUM_HIDDEN, TASK_STATE_RUNNING, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, HSTRING, PCWSTR, PWSTR};

fn win32_error(code: u32, what: &str) -> String {
    format!(
        "{what}: {}",
        windows::core::Error::from(HRESULT::from_win32(code)).message()
    )
}

fn wide_ptr_to_string(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: Windows hands out NUL-terminated strings that stay valid while their buffer lives.
    unsafe { text.to_string() }.unwrap_or_default()
}

// ----- Services --------------------------------------------------------------------------------

/// Closes a service handle when dropped.
struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own this handle and close it exactly once.
            let _ = unsafe { CloseServiceHandle(self.0) };
        }
    }
}

fn open_manager(access: u32) -> Result<ServiceHandle, String> {
    // SAFETY: plain call; the handle is owned by ServiceHandle.
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
        .map(ServiceHandle)
        .map_err(|error| format!("could not open the service manager: {}", error.message()))
}

fn open_service(
    manager: &ServiceHandle,
    name: &str,
    access: u32,
) -> Result<ServiceHandle, windows::core::Error> {
    let name = HSTRING::from(name);
    // SAFETY: valid manager handle and NUL-terminated name.
    unsafe { OpenServiceW(manager.0, &name, access) }.map(ServiceHandle)
}

fn to_state(state: u32) -> ServiceState {
    match state {
        s if s == SERVICE_STOPPED.0 => ServiceState::Stopped,
        s if s == SERVICE_RUNNING.0 => ServiceState::Running,
        s if s == SERVICE_PAUSED.0 => ServiceState::Paused,
        _ => ServiceState::Pending,
    }
}

/// All Win32 services (not drivers) with their start type and state.
pub fn list_services() -> Result<Vec<ServiceInfo>, String> {
    let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)?;
    // u64 elements keep the records (which hold pointers) aligned.
    let mut buffer: Vec<u64> = Vec::new();
    let mut needed = 0u32;
    let mut count = 0u32;
    let mut resume = 0u32;
    let mut entries: Vec<(String, String, ServiceState, u32)> = Vec::new();
    loop {
        // SAFETY: a byte view of the same allocation, for its full length.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 8)
        };
        // SAFETY: buffer and size describe the same allocation; out-params are valid.
        let result = unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                (!bytes.is_empty()).then_some(bytes),
                &mut needed,
                &mut count,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        let more = match result {
            Ok(()) => false,
            Err(error) if error.code() == HRESULT::from_win32(ERROR_MORE_DATA.0) => true,
            Err(error) => return Err(format!("could not list services: {}", error.message())),
        };
        let records = buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>();
        for index in 0..count as usize {
            // SAFETY: Windows wrote `count` records at the start of `buffer`; the strings they
            // point to live inside the same buffer.
            let record = unsafe { &*records.add(index) };
            entries.push((
                wide_ptr_to_string(record.lpServiceName),
                wide_ptr_to_string(record.lpDisplayName),
                to_state(record.ServiceStatusProcess.dwCurrentState.0),
                record.ServiceStatusProcess.dwProcessId,
            ));
        }
        if !more {
            break;
        }
        buffer = vec![0u64; (needed as usize).div_ceil(8) + 128];
        count = 0;
    }
    Ok(entries
        .into_iter()
        .map(|(name, display_name, state, pid)| {
            let start_type = open_service(&manager, &name, SERVICE_QUERY_CONFIG)
                .ok()
                .and_then(|service| query_start_type(&service).ok())
                // Unreadable config (rare, protected services): report as manual, which never
                // causes ExamSafe to act on it by itself.
                .unwrap_or(StartType::Manual);
            ServiceInfo {
                name,
                display_name,
                start_type,
                state,
                pid,
            }
        })
        .collect())
}

fn query_config(
    service: &ServiceHandle,
) -> Result<(ENUM_SERVICE_TYPE, SERVICE_START_TYPE), String> {
    let mut needed = 0u32;
    // SAFETY: size query with no buffer.
    let _ = unsafe { QueryServiceConfigW(service.0, None, 0, &mut needed) };
    if needed == 0 {
        return Err("could not read the service configuration".to_owned());
    }
    // u64 elements keep QUERY_SERVICE_CONFIGW aligned.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    let config = buffer.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
    // SAFETY: the buffer holds at least `needed` bytes.
    unsafe { QueryServiceConfigW(service.0, Some(config), needed, &mut needed) }
        .map_err(|error| error.message())?;
    // SAFETY: filled in by the successful call above.
    let config = unsafe { &*config };
    Ok((config.dwServiceType, config.dwStartType))
}

fn query_delayed(service: &ServiceHandle) -> bool {
    let mut info = [0u8; std::mem::size_of::<SERVICE_DELAYED_AUTO_START_INFO>()];
    let mut needed = 0u32;
    // SAFETY: the buffer is exactly the size of the structure.
    let ok = unsafe {
        QueryServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            Some(&mut info),
            &mut needed,
        )
    }
    .is_ok();
    ok && info.iter().any(|&byte| byte != 0)
}

fn query_start_type(service: &ServiceHandle) -> Result<StartType, String> {
    let (_, start) = query_config(service)?;
    Ok(match start {
        s if s == SERVICE_BOOT_START => StartType::Boot,
        s if s == SERVICE_SYSTEM_START => StartType::System,
        s if s == SERVICE_AUTO_START && query_delayed(service) => StartType::AutomaticDelayed,
        s if s == SERVICE_AUTO_START => StartType::Automatic,
        s if s == SERVICE_DISABLED => StartType::Disabled,
        _ => StartType::Manual,
    })
}

fn query_state(service: &ServiceHandle) -> Result<ServiceState, String> {
    let mut status = SERVICE_STATUS::default();
    // SAFETY: valid handle and out-param.
    unsafe { QueryServiceStatus(service.0, &mut status) }.map_err(|error| error.message())?;
    Ok(to_state(status.dwCurrentState.0))
}

fn wait_for_state(
    service: &ServiceHandle,
    wanted: ServiceState,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = query_state(service)?;
        if state == wanted {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "still {} after {} s",
                state.label().to_lowercase(),
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// How long a service gets to start or stop.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);

/// Sets the exact start type (delayed start included), then starts or stops the service and
/// waits until it is there. A service that does not exist (uninstalled) needs nothing.
/// Only ordinary Win32 services can be changed; drivers are refused.
pub fn apply_service(name: &str, start_type: StartType, running: bool) -> Result<(), String> {
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    let access = SERVICE_QUERY_CONFIG
        | SERVICE_CHANGE_CONFIG
        | SERVICE_QUERY_STATUS
        | SERVICE_START
        | SERVICE_STOP;
    let service = match open_service(&manager, name, access) {
        Ok(service) => service,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_DOES_NOT_EXIST.0) => {
            return Ok(());
        }
        Err(error) => return Err(error.message()),
    };
    let (kind, _) = query_config(&service)?;
    if kind.0 & SERVICE_WIN32.0 == 0 {
        return Err("not an ordinary service (drivers are never changed)".to_owned());
    }
    let (start, delayed) = match start_type {
        StartType::Boot | StartType::System => {
            return Err("boot and system start types are never set".to_owned());
        }
        StartType::Automatic => (SERVICE_AUTO_START, Some(false)),
        StartType::AutomaticDelayed => (SERVICE_AUTO_START, Some(true)),
        StartType::Manual => (SERVICE_DEMAND_START, None),
        StartType::Disabled => (SERVICE_DISABLED, None),
    };
    // SAFETY: valid handle; null strings mean "no change".
    unsafe {
        ChangeServiceConfigW(
            service.0,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            start,
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
    .map_err(|error| format!("could not set the start type: {}", error.message()))?;
    if let Some(delayed) = delayed {
        let info = SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: delayed.into(),
        };
        // SAFETY: `info` is the structure this info level expects and outlives the call.
        unsafe {
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                Some(std::ptr::from_ref(&info).cast::<c_void>()),
            )
        }
        .map_err(|error| format!("could not set delayed start: {}", error.message()))?;
    }

    if running {
        // SAFETY: valid handle, no arguments.
        match unsafe { StartServiceW(service.0, None) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_ALREADY_RUNNING.0) => {}
            Err(error) => return Err(format!("could not start it: {}", error.message())),
        }
        wait_for_state(&service, ServiceState::Running, SERVICE_TIMEOUT)
    } else {
        let mut status = SERVICE_STATUS::default();
        // SAFETY: valid handle and out-param.
        match unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_NOT_ACTIVE.0) => {}
            Err(error) => return Err(format!("could not stop it: {}", error.message())),
        }
        wait_for_state(&service, ServiceState::Stopped, SERVICE_TIMEOUT)
    }
}

// ----- Scheduled tasks -------------------------------------------------------------------------

/// Runs `body` with COM initialised on this thread (multithreaded apartment, as Task Scheduler
/// recommends for background work).
fn with_com<T>(body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    // SAFETY: balanced by CoUninitialize below when it succeeds.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let result = body();
    if initialised {
        // SAFETY: balances the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

fn task_service() -> Result<ITaskService, String> {
    let to_error = |error: windows::core::Error| format!("Task Scheduler: {}", error.message());
    // SAFETY: standard COM activation and connection to the local Task Scheduler.
    unsafe {
        let service: ITaskService =
            CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(to_error)?;
        let none = VARIANT::default();
        service
            .Connect(&none, &none, &none, &none)
            .map_err(to_error)?;
        Ok(service)
    }
}

fn collect_tasks(folder: &ITaskFolder, out: &mut Vec<TaskInfo>) {
    // Folders or tasks the user may not read are skipped: they cannot be matched anyway.
    // SAFETY: COM calls on a live folder; collections are 1-based.
    unsafe {
        if let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) {
            let count = tasks.Count().unwrap_or(0);
            for index in 1..=count {
                if let Ok(task) = tasks.get_Item(&VARIANT::from(index)) {
                    let path = task.Path().map(|p| p.to_string()).unwrap_or_default();
                    let enabled = task.Enabled().is_ok_and(|e| e.as_bool());
                    let running = task.State().is_ok_and(|s| s == TASK_STATE_RUNNING);
                    out.push(TaskInfo {
                        path,
                        enabled,
                        running,
                    });
                }
            }
        }
        if let Ok(folders) = folder.GetFolders(0) {
            let count = folders.Count().unwrap_or(0);
            for index in 1..=count {
                if let Ok(child) = folders.get_Item(&VARIANT::from(index)) {
                    collect_tasks(&child, out);
                }
            }
        }
    }
}

/// Every scheduled task the current user can see.
pub fn list_tasks() -> Result<Vec<TaskInfo>, String> {
    with_com(|| {
        let service = task_service()?;
        // SAFETY: COM call on a connected service.
        let root = unsafe { service.GetFolder(&BSTR::from("\\")) }
            .map_err(|error| format!("Task Scheduler: {}", error.message()))?;
        let mut tasks = Vec::new();
        collect_tasks(&root, &mut tasks);
        Ok(tasks)
    })
}

/// Enables or disables a task; disabling also stops a running instance. A task that does not
/// exist (deleted) needs nothing.
pub fn apply_task(path: &str, enabled: bool) -> Result<(), String> {
    with_com(|| {
        let service = task_service()?;
        let (folder_path, name) = match path.rfind('\\') {
            Some(0) => ("\\", &path[1..]),
            Some(index) => (&path[..index], &path[index + 1..]),
            None => ("\\", path),
        };
        // SAFETY: COM calls on a connected service.
        unsafe {
            let Ok(folder) = service.GetFolder(&BSTR::from(folder_path)) else {
                return Ok(());
            };
            let task: IRegisteredTask = match folder.GetTask(&BSTR::from(name)) {
                Ok(task) => task,
                Err(_) => return Ok(()),
            };
            task.SetEnabled(if enabled { VARIANT_TRUE } else { VARIANT_FALSE })
                .map_err(|error| error.message())?;
            if !enabled && task.State().is_ok_and(|s| s == TASK_STATE_RUNNING) {
                task.Stop(0)
                    .map_err(|error| format!("could not stop it: {}", error.message()))?;
            }
        }
        Ok(())
    })
}

// ----- Startup entries (registry) --------------------------------------------------------------

/// Closes a registry key when dropped.
struct KeyHandle(HKEY);

impl Drop for KeyHandle {
    fn drop(&mut self) {
        // SAFETY: we own this key and close it exactly once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

pub enum RegistryValue {
    Text(String),
    Binary(Vec<u8>),
}

/// All values of a key (`root` is HKCU or HKLM). A missing key gives an empty list.
pub fn read_registry_values(current_user: bool, subkey: &str) -> Vec<(String, RegistryValue)> {
    let root = if current_user {
        windows::Win32::System::Registry::HKEY_CURRENT_USER
    } else {
        windows::Win32::System::Registry::HKEY_LOCAL_MACHINE
    };
    let mut key = HKEY::default();
    let path = HSTRING::from(subkey);
    // SAFETY: valid root key, NUL-terminated path, out-param owned below.
    if unsafe { RegOpenKeyExW(root, &path, None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let key = KeyHandle(key);
    let mut values = Vec::new();
    let mut index = 0u32;
    loop {
        let mut name = vec![0u16; 16_384];
        let mut name_len = u32::try_from(name.len()).unwrap_or(0);
        let mut kind = 0u32;
        let mut data = vec![0u8; 8192];
        let mut data_len = u32::try_from(data.len()).unwrap_or(0);
        // SAFETY: buffers and lengths describe the same allocations.
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        index += 1;
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_MORE_DATA || status != NO_ERROR {
            // Oversized values are not startup commands we can use; skip them.
            if status == ERROR_MORE_DATA || status == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            break;
        }
        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        let bytes = &data[..data_len as usize];
        let kind = REG_VALUE_TYPE(kind);
        let value = if kind == REG_SZ || kind == REG_EXPAND_SZ {
            let wide: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .take_while(|&c| c != 0)
                .collect();
            RegistryValue::Text(String::from_utf16_lossy(&wide))
        } else if kind == REG_BINARY {
            RegistryValue::Binary(bytes.to_vec())
        } else {
            continue;
        };
        values.push((name, value));
    }
    values
}

// ----- Listening ports -------------------------------------------------------------------------

fn table(fetch: impl Fn(Option<*mut c_void>, &mut u32) -> u32) -> Result<Vec<u64>, String> {
    let mut size = 0u32;
    let _ = fetch(None, &mut size);
    for _ in 0..4 {
        let mut buffer = vec![0u64; (size as usize).div_ceil(8) + 16];
        let mut capacity = u32::try_from(buffer.len() * 8).unwrap_or(u32::MAX);
        let status = fetch(Some(buffer.as_mut_ptr().cast::<c_void>()), &mut capacity);
        if status == NO_ERROR.0 {
            return Ok(buffer);
        }
        if status != ERROR_INSUFFICIENT_BUFFER.0 {
            return Err(win32_error(status, "could not read the port table"));
        }
        size = capacity;
    }
    Err("the port table kept growing".to_owned())
}

/// Rows of a MIB_*TABLE_OWNER_PID: a u32 count followed by the rows (aligned to 4 bytes).
fn rows<T: Copy>(buffer: &[u64]) -> Vec<T> {
    let base = buffer.as_ptr().cast::<u8>();
    // SAFETY: the table starts with its row count.
    let count = unsafe { *base.cast::<u32>() } as usize;
    let max = (buffer.len() * 8 - 4) / std::mem::size_of::<T>();
    let first = base.wrapping_add(4).cast::<T>();
    // SAFETY: Windows wrote `count` rows right after the count; `max` guards the buffer end.
    (0..count.min(max))
        .map(|index| unsafe { first.add(index).read_unaligned() })
        .collect()
}

fn port_from_network(raw: u32) -> u16 {
    // The low 16 bits hold the port in network byte order.
    u16::from_be(u16::try_from(raw & 0xffff).unwrap_or_default())
}

pub fn list_listening_ports() -> Result<Vec<(Protocol, String, u16, u32)>, String> {
    let mut ports = Vec::new();
    let tcp4 = table(|buffer, size| {
        // SAFETY: buffer/size come from `table`, which sizes them together.
        unsafe {
            GetExtendedTcpTable(
                buffer,
                size,
                false,
                u32::from(AF_INET.0),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        }
    })?;
    for row in rows::<MIB_TCPROW_OWNER_PID>(&tcp4) {
        let address = std::net::Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
        ports.push((
            Protocol::Tcp,
            address.to_string(),
            port_from_network(row.dwLocalPort),
            row.dwOwningPid,
        ));
    }
    let tcp6 = table(|buffer, size| {
        // SAFETY: as above.
        unsafe {
            GetExtendedTcpTable(
                buffer,
                size,
                false,
                u32::from(AF_INET6.0),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        }
    })?;
    for row in rows::<MIB_TCP6ROW_OWNER_PID>(&tcp6) {
        let address = std::net::Ipv6Addr::from(row.ucLocalAddr);
        ports.push((
            Protocol::Tcp,
            address.to_string(),
            port_from_network(row.dwLocalPort),
            row.dwOwningPid,
        ));
    }
    let udp4 = table(|buffer, size| {
        // SAFETY: as above.
        unsafe {
            GetExtendedUdpTable(
                buffer,
                size,
                false,
                u32::from(AF_INET.0),
                UDP_TABLE_OWNER_PID,
                0,
            )
        }
    })?;
    for row in rows::<MIB_UDPROW_OWNER_PID>(&udp4) {
        let address = std::net::Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
        ports.push((
            Protocol::Udp,
            address.to_string(),
            port_from_network(row.dwLocalPort),
            row.dwOwningPid,
        ));
    }
    let udp6 = table(|buffer, size| {
        // SAFETY: as above.
        unsafe {
            GetExtendedUdpTable(
                buffer,
                size,
                false,
                u32::from(AF_INET6.0),
                UDP_TABLE_OWNER_PID,
                0,
            )
        }
    })?;
    for row in rows::<MIB_UDP6ROW_OWNER_PID>(&udp6) {
        let address = std::net::Ipv6Addr::from(row.ucLocalAddr);
        ports.push((
            Protocol::Udp,
            address.to_string(),
            port_from_network(row.dwLocalPort),
            row.dwOwningPid,
        ));
    }
    Ok(ports)
}

/// Kept separate so the platform layer can attach process names.
pub fn listening_port(
    protocol: Protocol,
    address: String,
    port: u16,
    pid: u32,
    process: String,
) -> ListeningPort {
    ListeningPort {
        protocol,
        address,
        port,
        pid,
        process,
    }
}

// ----- Network adapters ------------------------------------------------------------------------

pub fn list_adapters() -> Result<Vec<NetworkAdapter>, String> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = 16 * 1024u32;
    for _ in 0..4 {
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: buffer and size describe the same allocation.
        let status = unsafe {
            GetAdaptersAddresses(u32::from(AF_UNSPEC.0), flags, None, Some(first), &mut size)
        };
        if status == ERROR_INSUFFICIENT_BUFFER.0
            || status == windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW.0
        {
            continue;
        }
        if status != NO_ERROR.0 {
            return Err(win32_error(status, "could not list network adapters"));
        }
        let mut adapters = Vec::new();
        let mut current = first;
        while !current.is_null() {
            // SAFETY: a linked list inside `buffer`, written by the successful call above.
            let adapter = unsafe { &*current };
            adapters.push(NetworkAdapter {
                name: wide_ptr_to_string(adapter.FriendlyName),
                description: wide_ptr_to_string(adapter.Description),
                up: adapter.OperStatus == IfOperStatusUp,
            });
            current = adapter.Next;
        }
        return Ok(adapters);
    }
    Err("the adapter list kept growing".to_owned())
}
