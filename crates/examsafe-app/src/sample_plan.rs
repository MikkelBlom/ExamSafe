//! Sample plan for the prototype's advanced view, based on the 2026-09-30 survey of Mikkel's PC.
//! Replaced by the real planner once the detection engine exists.

pub struct SampleItem {
    pub category: &'static str,
    pub name: &'static str,
    pub method: &'static str,
}

pub const SAMPLE_PLAN: &[SampleItem] = &[
    SampleItem {
        category: "AI",
        name: "Claude",
        method: "Close app, stop Cowork service",
    },
    SampleItem {
        category: "AI",
        name: "Microsoft Copilot",
        method: "Close app, disable startup",
    },
    SampleItem {
        category: "AI",
        name: "Wispr Flow",
        method: "Close app",
    },
    SampleItem {
        category: "AI",
        name: "Claude & Cline in VS Code",
        method: "Disable extensions",
    },
    SampleItem {
        category: "AI",
        name: "Claude in Chrome",
        method: "Block extension by policy",
    },
    SampleItem {
        category: "Remote access",
        name: "TeamViewer",
        method: "Stop and disable service",
    },
    SampleItem {
        category: "Remote access",
        name: "ASUS GlideX",
        method: "Stop and disable services",
    },
    SampleItem {
        category: "Remote access",
        name: "Chrome Remote Desktop",
        method: "Block extension by policy",
    },
    SampleItem {
        category: "Device link",
        name: "Phone Link",
        method: "Close app",
    },
    SampleItem {
        category: "Chat",
        name: "Slack",
        method: "Close app",
    },
    SampleItem {
        category: "Chat",
        name: "Discord",
        method: "Disable startup",
    },
    SampleItem {
        category: "File sync",
        name: "Google Drive",
        method: "Close app, disable startup",
    },
    SampleItem {
        category: "Network",
        name: "WireGuard tunnel",
        method: "Disable adapter",
    },
    SampleItem {
        category: "Virtualization",
        name: "WSL & VirtualBox adapter",
        method: "Shut down WSL, disable adapter",
    },
    SampleItem {
        category: "Work",
        name: "SQL Server",
        method: "Stop and disable services",
    },
    SampleItem {
        category: "Work",
        name: "PMS Vulnerability Watch",
        method: "Disable scheduled task",
    },
];
