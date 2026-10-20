//! The Advanced view's rows: every app, service, scheduled task, startup item, listening port and
//! network adapter ExamSafe knows about, with its state, what exam mode will do with it, and the
//! preference key its toggle controls. Pure, so the wording is unit tested.

use crate::apps::Catalog;
use crate::inventory::{
    ListeningPort, NetworkAdapter, Protocol, StartupItem, is_virtual_adapter, port_owner,
    startup_owner,
};
use crate::service::Findings;
use crate::system::{ServiceState, task_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Apps,
    Services,
    Tasks,
    Startup,
    Ports,
    Adapters,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
            Self::Services => "Services",
            Self::Tasks => "Scheduled tasks",
            Self::Startup => "Startup items",
            Self::Ports => "Listening ports",
            Self::Adapters => "Network adapters",
        }
    }
}

/// What the read-only parts of Advanced show. Filled in the background; empty until then.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InventorySnapshot {
    pub startup: Vec<StartupItem>,
    pub ports: Vec<ListeningPort>,
    pub adapters: Vec<NetworkAdapter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancedRow {
    pub section: Section,
    /// The preference its toggle controls. `None`: shown only, no toggle.
    pub key: Option<String>,
    pub name: String,
    pub detail: String,
    pub state: String,
    /// What exam mode will do with it, in plain words.
    pub action: String,
    /// Toggle position: on = ExamSafe acts on it, off = left alone (remembered).
    pub enabled: bool,
}

const NOT_CHANGED: &str = "Shown only: this version doesn't change these";

pub fn advanced_rows(
    catalog: &Catalog,
    findings: &Findings,
    inventory: &InventorySnapshot,
    left_alone: &[String],
) -> Vec<AdvancedRow> {
    let alone = |key: &str| left_alone.iter().any(|k| k == key);
    let mut rows = Vec::new();

    for app in &findings.apps {
        let action = if alone(&app.entry_id) {
            "Left open (your choice)".to_owned()
        } else {
            match &app.launch {
                Some(spec) if spec.background => {
                    "Close now; start again in the background afterwards".to_owned()
                }
                Some(_) => "Close now; reopen afterwards".to_owned(),
                None => "Close now".to_owned(),
            }
        };
        rows.push(AdvancedRow {
            section: Section::Apps,
            key: Some(app.entry_id.clone()),
            name: app.name.clone(),
            detail: app.category.clone(),
            state: format!(
                "Running · {} process{}",
                app.pids.len(),
                if app.pids.len() == 1 { "" } else { "es" }
            ),
            action,
            enabled: !alone(&app.entry_id),
        });
    }

    for service in &findings.services {
        let key = service.key();
        let action = if alone(&key) {
            "Left as it is (your choice)".to_owned()
        } else if service.needs_change() {
            let stop = if service.info.state == ServiceState::Stopped {
                ""
            } else {
                "Stop and "
            };
            let set = if stop.is_empty() { "Set" } else { "set" };
            format!(
                "{stop}{set} to {}; back to {} afterwards",
                service.target.label(),
                service.info.start_type.label()
            )
        } else {
            "Already off: nothing to do".to_owned()
        };
        rows.push(AdvancedRow {
            section: Section::Services,
            enabled: !alone(&key),
            key: Some(key),
            name: service.info.display_name.clone(),
            detail: format!("{} · {}", service.category, service.info.name),
            state: format!(
                "{} · {}",
                service.info.state.label(),
                service.info.start_type.label()
            ),
            action,
        });
    }

    for task in &findings.tasks {
        let key = task.key();
        let action = if alone(&key) {
            "Left as it is (your choice)"
        } else if task.needs_change() {
            "Disable now; enable again afterwards"
        } else {
            "Already disabled: nothing to do"
        };
        let state = match (task.info.enabled, task.info.running) {
            (_, true) => "Running",
            (true, false) => "Enabled",
            (false, false) => "Disabled",
        };
        rows.push(AdvancedRow {
            section: Section::Tasks,
            enabled: !alone(&key),
            key: Some(key),
            name: task_name(&task.info.path).to_owned(),
            detail: format!("{} · {}", task.category, task.info.path),
            state: state.to_owned(),
            action: action.to_owned(),
        });
    }

    for item in &inventory.startup {
        let action = match startup_owner(catalog, item) {
            Some(app) => format!("{NOT_CHANGED} ({app} itself is handled as an app)"),
            None => NOT_CHANGED.to_owned(),
        };
        rows.push(AdvancedRow {
            section: Section::Startup,
            key: None,
            name: item.name.clone(),
            detail: item.location.clone(),
            state: if item.enabled { "On" } else { "Off" }.to_owned(),
            action,
            enabled: item.enabled,
        });
    }

    for port in &inventory.ports {
        let owner = port_owner_key(catalog, findings, port);
        // UDP sockets are mostly Windows' own; only show those that belong to a listed item.
        if port.protocol == Protocol::Udp && owner.is_none() {
            continue;
        }
        let reach = if port.is_network_facing() {
            "reachable from the network"
        } else {
            "this PC only"
        };
        let (key, action, enabled) = match owner {
            Some((key, name)) if alone(&key) => (
                Some(key),
                format!("Stays open ({name} is left alone)"),
                false,
            ),
            Some((key, name)) => (Some(key), format!("Closes when {name} is turned off"), true),
            None => (None, "Not changed".to_owned(), true),
        };
        rows.push(AdvancedRow {
            section: Section::Ports,
            key,
            name: format!("{} {}", port.protocol.label(), port.port),
            detail: format!("{} · {}", port.process, port.address),
            state: format!("Listening · {reach}"),
            action,
            enabled,
        });
    }

    for adapter in &inventory.adapters {
        let action = if is_virtual_adapter(catalog, adapter) {
            format!("VPN/virtual: exam software may log it. {NOT_CHANGED}")
        } else {
            "Not changed".to_owned()
        };
        rows.push(AdvancedRow {
            section: Section::Adapters,
            key: None,
            name: adapter.name.clone(),
            detail: adapter.description.clone(),
            state: if adapter.up {
                "Connected"
            } else {
                "Not connected"
            }
            .to_owned(),
            action,
            enabled: adapter.up,
        });
    }
    rows
}

/// The listed item a port belongs to: a catalog service running in that process, or a running
/// catalog app. Returns (preference key, display name).
pub fn port_owner_key(
    catalog: &Catalog,
    findings: &Findings,
    port: &ListeningPort,
) -> Option<(String, String)> {
    if port.pid > 4
        && let Some(service) = findings.services.iter().find(|s| s.info.pid == port.pid)
    {
        return Some((service.key(), service.info.display_name.clone()));
    }
    if let Some(app) = findings.apps.iter().find(|a| a.pids.contains(&port.pid)) {
        return Some((app.entry_id.clone(), app.name.clone()));
    }
    // A catalog app's process that the scan did not select (e.g. a helper of a listed app).
    port_owner(catalog, port).and_then(|name| {
        catalog
            .entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| (e.id.clone(), e.name.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{AppFinding, LaunchSpec, LaunchTarget};
    use crate::system::{ServiceFinding, ServiceInfo, StartType, TaskFinding, TaskInfo};

    fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{"version":1,"virtual_adapters":["WireGuard"],"entries":[
                {"id":"slack","name":"Slack","category":"Chat","processes":["slack"]},
                {"id":"tv","name":"TeamViewer","category":"Remote access","services":["TeamViewer"]}
            ]}"#,
        )
        .unwrap()
    }

    fn findings() -> Findings {
        Findings {
            apps: vec![AppFinding {
                entry_id: "slack".into(),
                name: "Slack".into(),
                category: "Chat".into(),
                pids: vec![10, 11],
                root_pids: vec![10],
                launch: Some(LaunchSpec {
                    background: true,
                    ..LaunchSpec::new(LaunchTarget::Path("C:\\slack.exe".into()))
                }),
            }],
            services: vec![ServiceFinding {
                entry_id: "tv".into(),
                app_name: "TeamViewer".into(),
                category: "Remote access".into(),
                info: ServiceInfo {
                    name: "TeamViewer".into(),
                    display_name: "TeamViewer".into(),
                    start_type: StartType::AutomaticDelayed,
                    state: ServiceState::Running,
                    pid: 77,
                },
                target: StartType::Disabled,
            }],
            tasks: vec![TaskFinding {
                entry_id: "tv".into(),
                app_name: "TeamViewer".into(),
                category: "Remote access".into(),
                info: TaskInfo {
                    path: "\\TeamViewer\\Update".into(),
                    enabled: false,
                    running: false,
                },
            }],
        }
    }

    fn inventory() -> InventorySnapshot {
        InventorySnapshot {
            startup: vec![StartupItem {
                name: "Slack".into(),
                command: "C:\\slack.exe --startup".into(),
                location: "Your Run key".into(),
                enabled: true,
            }],
            ports: vec![
                ListeningPort {
                    protocol: Protocol::Tcp,
                    address: "0.0.0.0".into(),
                    port: 5939,
                    pid: 77,
                    process: "TeamViewer_Service.exe".into(),
                },
                ListeningPort {
                    protocol: Protocol::Tcp,
                    address: "127.0.0.1".into(),
                    port: 9000,
                    pid: 999,
                    process: "node.exe".into(),
                },
                ListeningPort {
                    protocol: Protocol::Udp,
                    address: "0.0.0.0".into(),
                    port: 5353,
                    pid: 1234,
                    process: "svchost.exe".into(),
                },
            ],
            adapters: vec![NetworkAdapter {
                name: "bdvpn".into(),
                description: "WireGuard Tunnel".into(),
                up: true,
            }],
        }
    }

    fn row<'a>(rows: &'a [AdvancedRow], name: &str) -> &'a AdvancedRow {
        rows.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn every_section_is_listed_in_order_with_state_and_action() {
        let rows = advanced_rows(&catalog(), &findings(), &inventory(), &[]);
        let sections: Vec<Section> = rows.iter().map(|r| r.section).collect();
        assert_eq!(
            sections,
            vec![
                Section::Apps,
                Section::Services,
                Section::Tasks,
                Section::Startup,
                Section::Ports,
                Section::Ports,
                Section::Adapters
            ],
            "Windows' own UDP port is not listed"
        );
        let slack = row(&rows, "Slack");
        assert_eq!(slack.state, "Running · 2 processes");
        assert_eq!(
            slack.action,
            "Close now; start again in the background afterwards"
        );
        let tv = row(&rows, "TeamViewer");
        assert_eq!(tv.state, "Running · Automatic (delayed)");
        assert_eq!(
            tv.action,
            "Stop and set to Disabled; back to Automatic (delayed) afterwards"
        );
        assert_eq!(
            row(&rows, "Update").action,
            "Already disabled: nothing to do"
        );
        let vpn = row(&rows, "bdvpn");
        assert!(vpn.action.starts_with("VPN/virtual"));
        assert_eq!(vpn.key, None, "shown only: no toggle");
    }

    #[test]
    fn ports_follow_the_toggle_of_the_item_that_owns_them() {
        let rows = advanced_rows(&catalog(), &findings(), &inventory(), &[]);
        let tv_port = row(&rows, "TCP 5939");
        assert_eq!(tv_port.key.as_deref(), Some("service:TeamViewer"));
        assert_eq!(tv_port.action, "Closes when TeamViewer is turned off");
        assert_eq!(tv_port.state, "Listening · reachable from the network");
        let node = row(&rows, "TCP 9000");
        assert_eq!(node.key, None);
        assert_eq!(node.state, "Listening · this PC only");

        let left = vec!["service:TeamViewer".to_owned()];
        let rows = advanced_rows(&catalog(), &findings(), &inventory(), &left);
        let tv_port = row(&rows, "TCP 5939");
        assert!(!tv_port.enabled);
        assert_eq!(tv_port.action, "Stays open (TeamViewer is left alone)");
        let tv = row(&rows, "TeamViewer");
        assert!(!tv.enabled);
        assert_eq!(tv.action, "Left as it is (your choice)");
    }

    #[test]
    fn startup_items_name_the_app_that_handles_them() {
        let rows = advanced_rows(&catalog(), &findings(), &inventory(), &[]);
        let startup = rows.iter().find(|r| r.section == Section::Startup).unwrap();
        assert!(startup.action.contains("Slack itself is handled as an app"));
        assert_eq!(startup.state, "On");
    }

    #[test]
    fn left_open_app_says_so() {
        let rows = advanced_rows(
            &catalog(),
            &findings(),
            &InventorySnapshot::default(),
            &["slack".to_owned()],
        );
        assert_eq!(row(&rows, "Slack").action, "Left open (your choice)");
        assert!(!row(&rows, "Slack").enabled);
    }
}
