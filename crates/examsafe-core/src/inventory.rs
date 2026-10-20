//! Read-only inventory shown in Advanced: startup items, listening ports and network adapters.
//! This version reports them (with which catalog app they belong to); it does not change them.

use crate::apps::{Catalog, process_matches};

/// Something Windows starts at sign-in (Run keys or the Startup folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    /// Where it is registered, e.g. `HKCU Run` or `Startup folder`.
    pub location: String,
    /// The Task Manager "Enabled/Disabled" switch.
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// A port a process listens on (TCP listening sockets and bound UDP sockets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListeningPort {
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    pub pid: u32,
    /// Executable name of the owning process, when known.
    pub process: String,
}

impl ListeningPort {
    /// Listening on all addresses or a non-loopback one: reachable from the network.
    pub fn is_network_facing(&self) -> bool {
        !(self.address.starts_with("127.") || self.address == "::1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAdapter {
    pub name: String,
    pub description: String,
    pub up: bool,
}

/// The catalog app a startup item starts, judged by the program in its command, or by the
/// item's own name (Startup-folder shortcuts are named after the app, e.g. `Wispr Flow.lnk`).
pub fn startup_owner<'a>(catalog: &'a Catalog, item: &StartupItem) -> Option<&'a str> {
    let program = program_name(&item.command).unwrap_or_default();
    catalog
        .entries
        .iter()
        .find(|entry| {
            entry.processes.iter().any(|pattern| {
                process_matches(pattern, &program) || process_matches(pattern, &item.name)
            })
        })
        .map(|entry| entry.name.as_str())
}

/// The catalog app owning a port, judged by the owning process.
pub fn port_owner<'a>(catalog: &'a Catalog, port: &ListeningPort) -> Option<&'a str> {
    catalog
        .entries
        .iter()
        .find(|entry| {
            entry
                .processes
                .iter()
                .any(|pattern| process_matches(pattern, &port.process))
        })
        .map(|entry| entry.name.as_str())
}

/// A VPN or virtual adapter (by the catalog's adapter patterns, matched anywhere in the name or
/// description). Exam monitoring software may log these.
pub fn is_virtual_adapter(catalog: &Catalog, adapter: &NetworkAdapter) -> bool {
    let text = format!("{} {}", adapter.name, adapter.description).to_lowercase();
    catalog
        .virtual_adapters
        .iter()
        .any(|pattern| text.contains(&pattern.to_lowercase()))
}

/// The executable file name in a command line, e.g. `"C:\A B\x.exe" -s` -> `x.exe`.
pub fn program_name(command: &str) -> Option<String> {
    let line = command.trim();
    let program = match line.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => {
            // Unquoted paths may contain spaces: cut after ".exe" when present.
            let lower = line.to_lowercase();
            match lower.find(".exe") {
                Some(end) => &line[..end + 4],
                None => line.split_whitespace().next().unwrap_or_default(),
            }
        }
    };
    let file = program.rsplit(['\\', '/']).next().unwrap_or(program).trim();
    (!file.is_empty()).then(|| file.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{"version":1,"virtual_adapters":["WireGuard","VirtualBox"],"entries":[
                {"id":"discord","name":"Discord","category":"Chat","processes":["Discord","Update"]},
                {"id":"tv","name":"TeamViewer","category":"Remote","processes":["TeamViewer_Service"]}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn program_names_from_command_lines() {
        assert_eq!(
            program_name(
                "\"C:\\Users\\me\\AppData\\Local\\Discord\\Update.exe\" --processStart Discord.exe"
            )
            .as_deref(),
            Some("Update.exe")
        );
        assert_eq!(
            program_name("C:\\Program Files\\App\\app.exe -tray").as_deref(),
            Some("app.exe")
        );
        assert_eq!(program_name("notepad").as_deref(), Some("notepad"));
        assert_eq!(program_name("  "), None);
    }

    #[test]
    fn startup_items_and_ports_are_tied_to_catalog_apps() {
        let item = StartupItem {
            name: "Discord".into(),
            command: "\"C:\\Discord\\Update.exe\" --processStart Discord.exe".into(),
            location: "HKCU Run".into(),
            enabled: true,
        };
        assert_eq!(startup_owner(&catalog(), &item), Some("Discord"));
        let shortcut = StartupItem {
            name: "Discord".into(),
            command: "C:\\Users\\me\\Startup\\Discord.lnk".into(),
            location: "Your Startup folder".into(),
            enabled: true,
        };
        assert_eq!(startup_owner(&catalog(), &shortcut), Some("Discord"));
        let port = ListeningPort {
            protocol: Protocol::Tcp,
            address: "0.0.0.0".into(),
            port: 5939,
            pid: 4,
            process: "TeamViewer_Service.exe".into(),
        };
        assert_eq!(port_owner(&catalog(), &port), Some("TeamViewer"));
        assert!(port.is_network_facing());
        let local = ListeningPort {
            address: "127.0.0.1".into(),
            ..port
        };
        assert!(!local.is_network_facing());
    }

    #[test]
    fn virtual_adapters_by_pattern() {
        let vpn = NetworkAdapter {
            name: "bdvpnservice_2".into(),
            description: "WireGuard Tunnel".into(),
            up: true,
        };
        let wifi = NetworkAdapter {
            name: "Wi-Fi".into(),
            description: "Intel(R) Wi-Fi 7".into(),
            up: true,
        };
        assert!(is_virtual_adapter(&catalog(), &vpn));
        assert!(!is_virtual_adapter(&catalog(), &wifi));
    }
}
