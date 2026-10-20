//! [`SystemInventory`] for Windows, and applying service / scheduled-task changes (called by the
//! elevated helper only).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use examsafe_core::inventory::{ListeningPort, NetworkAdapter, StartupItem};
use examsafe_core::ports::{InventoryError, SystemInventory};
use examsafe_core::system::{ServiceInfo, SystemChange, TaskInfo};

use crate::windows_system_impl::{self as win, RegistryValue};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsSystemInventory;

fn error(what: &'static str) -> impl Fn(String) -> InventoryError {
    move |reason| InventoryError { what, reason }
}

impl SystemInventory for WindowsSystemInventory {
    fn services(&self) -> Result<Vec<ServiceInfo>, InventoryError> {
        win::list_services().map_err(error("services"))
    }

    fn tasks(&self) -> Result<Vec<TaskInfo>, InventoryError> {
        win::list_tasks().map_err(error("scheduled tasks"))
    }

    fn startup_items(&self) -> Result<Vec<StartupItem>, InventoryError> {
        Ok(startup_items())
    }

    fn listening_ports(&self) -> Result<Vec<ListeningPort>, InventoryError> {
        let names: HashMap<u32, String> = crate::windows_impl::list_processes()
            .map(|processes| processes.into_iter().map(|p| (p.pid, p.exe_name)).collect())
            .unwrap_or_default();
        let mut ports: Vec<ListeningPort> = win::list_listening_ports()
            .map_err(error("listening ports"))?
            .into_iter()
            .map(|(protocol, address, port, pid)| {
                let process = names.get(&pid).cloned().unwrap_or_else(|| match pid {
                    0 | 4 => "System".to_owned(),
                    _ => format!("pid {pid}"),
                });
                win::listening_port(protocol, address, port, pid, process)
            })
            .collect();
        // One row per (port, protocol, process, address): Windows lists a socket per
        // interface and per open handle.
        ports.sort_by(|a, b| {
            (a.port, a.protocol.label(), a.pid, &a.address).cmp(&(
                b.port,
                b.protocol.label(),
                b.pid,
                &b.address,
            ))
        });
        ports.dedup_by(|a, b| {
            a.port == b.port && a.protocol == b.protocol && a.pid == b.pid && a.address == b.address
        });
        Ok(ports)
    }

    fn network_adapters(&self) -> Result<Vec<NetworkAdapter>, InventoryError> {
        win::list_adapters().map_err(error("network adapters"))
    }
}

/// Applies one change. Protected items (security and exam software) are refused here too, so
/// even a hand-made helper request cannot touch them.
pub fn apply_change(change: &SystemChange) -> Result<(), String> {
    if change.is_protected() {
        return Err("protected (security or exam software) - ExamSafe never changes it".to_owned());
    }
    match change {
        SystemChange::Service {
            name,
            start_type,
            running,
        } => win::apply_service(name, *start_type, *running),
        SystemChange::Task { path, enabled } => win::apply_task(path, *enabled),
    }
}

// ----- Startup items ---------------------------------------------------------------------------

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW64: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// Task Manager's switch: a binary value whose first byte has the low bit set when disabled.
/// No value means enabled.
fn approved(current_user: bool, list: &str) -> HashMap<String, bool> {
    win::read_registry_values(current_user, &format!(r"{APPROVED}\{list}"))
        .into_iter()
        .filter_map(|(name, value)| match value {
            RegistryValue::Binary(bytes) => Some((
                name.to_lowercase(),
                bytes.first().is_none_or(|b| b & 1 == 0),
            )),
            RegistryValue::Text(_) => None,
        })
        .collect()
}

fn run_key_items(current_user: bool, key: &str, list: &str, location: &str) -> Vec<StartupItem> {
    let switches = approved(current_user, list);
    win::read_registry_values(current_user, key)
        .into_iter()
        .filter_map(|(name, value)| match value {
            RegistryValue::Text(command) => Some(StartupItem {
                enabled: switches.get(&name.to_lowercase()).copied().unwrap_or(true),
                name,
                command,
                location: location.to_owned(),
            }),
            RegistryValue::Binary(_) => None,
        })
        .collect()
}

fn folder_items(folder: Option<PathBuf>, current_user: bool, location: &str) -> Vec<StartupItem> {
    let Some(folder) = folder else {
        return Vec::new();
    };
    let switches = approved(current_user, "StartupFolder");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && !is_desktop_ini(path))
        .map(|path| {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            StartupItem {
                name: path
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                enabled: switches.get(&file.to_lowercase()).copied().unwrap_or(true),
                command: path.to_string_lossy().into_owned(),
                location: location.to_owned(),
            }
        })
        .collect()
}

fn is_desktop_ini(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("desktop.ini"))
}

fn startup_folder(base_var: &str) -> Option<PathBuf> {
    std::env::var_os(base_var)
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
}

/// Run keys (user and machine, 64- and 32-bit) and both Startup folders.
pub fn startup_items() -> Vec<StartupItem> {
    let mut items = run_key_items(true, RUN, "Run", "Your Run key");
    items.extend(run_key_items(false, RUN, "Run", "Machine Run key"));
    items.extend(run_key_items(
        false,
        RUN_WOW64,
        "Run32",
        "Machine Run key (32-bit)",
    ));
    items.extend(folder_items(
        startup_folder("APPDATA"),
        true,
        "Your Startup folder",
    ));
    items.extend(folder_items(
        startup_folder("PROGRAMDATA"),
        false,
        "All users' Startup folder",
    ));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use examsafe_core::system::{ServiceState, StartType};

    #[test]
    fn lists_real_services_with_start_types() {
        let services = WindowsSystemInventory.services().unwrap();
        assert!(services.len() > 50, "only {} services", services.len());
        let rpc = services
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("RpcSs"))
            .expect("RpcSs is always installed");
        assert_eq!(rpc.state, ServiceState::Running);
        assert_eq!(rpc.start_type, StartType::Automatic);
        assert!(
            services
                .iter()
                .any(|s| s.start_type == StartType::AutomaticDelayed),
            "some service uses delayed start on every Windows install"
        );
    }

    #[test]
    fn lists_real_scheduled_tasks() {
        let tasks = WindowsSystemInventory.tasks().unwrap();
        assert!(!tasks.is_empty());
        assert!(tasks.iter().all(|t| t.path.starts_with('\\')));
    }

    #[test]
    fn lists_ports_and_adapters() {
        let ports = WindowsSystemInventory.listening_ports().unwrap();
        assert!(!ports.is_empty(), "Windows always listens on something");
        let adapters = WindowsSystemInventory.network_adapters().unwrap();
        assert!(
            adapters
                .iter()
                .any(|a| a.description.contains("Loopback") || a.up)
        );
    }

    #[test]
    fn reads_startup_items_without_failing() {
        // Contents vary per PC; it must simply not fail or return blank names.
        assert!(startup_items().iter().all(|item| !item.name.is_empty()));
    }

    #[test]
    fn protected_changes_are_refused_before_touching_windows() {
        let change = SystemChange::Service {
            name: "WinDefend".into(),
            start_type: StartType::Disabled,
            running: false,
        };
        assert!(apply_change(&change).unwrap_err().contains("protected"));
    }

    #[test]
    fn missing_service_or_task_needs_nothing() {
        assert_eq!(
            apply_change(&SystemChange::Task {
                path: "\\ExamSafe-does-not-exist\\Nope".into(),
                enabled: true
            }),
            Ok(())
        );
    }
}
