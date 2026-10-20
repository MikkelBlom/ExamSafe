//! Command-line mode: `ExamSafe.exe --cli <command>`. Drives the exact same flow and service as
//! the window, so it doubles as the end-to-end test harness.
//!
//! Commands:
//!   scan               list catalog apps that are running, and catalog services and scheduled
//!                      tasks with their state and what exam mode would do
//!   inventory          also list startup items, listening ports and network adapters
//!   make-safe --yes    turn them off, verify, and record them for restore (--yes = confirm)
//!   restore            put back exactly what exam mode changed
//!   status             is exam mode on, and what is recorded
//!
//! Exit codes: 0 done, 1 failed, 2 usage error, 3 confirmation needed (add --yes).

use std::collections::VecDeque;
use std::io::Write;

use examsafe_core::advanced::port_owner_key;
use examsafe_core::apps::{ClosedApp, LaunchTarget};
use examsafe_core::flow::{Effect, Event, Flow, Phase};
use examsafe_core::inventory::{is_virtual_adapter, startup_owner};
use examsafe_core::service::{ExamService, Findings, run_effect};
use examsafe_core::system::{ServiceFinding, ServiceState, TaskFinding};

const USAGE: &str =
    "usage: ExamSafe.exe --cli scan | inventory | make-safe --yes | restore | status";

/// Writes one line to stdout. A console that went away is not actionable, so write errors are
/// deliberately ignored here (and only here).
fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

pub fn run(args: &[String], service: &ExamService) -> u8 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["scan"] => scan(service),
        ["inventory"] => {
            let code = scan(service);
            inventory(service, &service.scan().unwrap_or_default());
            code
        }
        ["status"] => status(service),
        ["make-safe"] => {
            let code = scan(service);
            if code == 0 {
                say("Nothing was changed. Add --yes to turn off what is listed above.");
                3
            } else {
                code
            }
        }
        ["make-safe", "--yes"] => drive(service, false),
        ["restore"] => drive(service, true),
        _ => {
            say(USAGE);
            2
        }
    }
}

/// Items the user chose to leave alone (in the window's Advanced view). Unreadable preferences
/// fall back to "turn off everything on the list", the safe side.
fn left_open(service: &ExamService) -> Vec<String> {
    service.left_open().unwrap_or_else(|error| {
        say(&format!(
            "Could not read preferences, turning off everything on the list: {error}"
        ));
        Vec::new()
    })
}

fn service_line(service: &ServiceFinding, left_alone: bool) -> String {
    let now = format!(
        "{}, {}",
        service.info.state.label(),
        service.info.start_type.label()
    );
    let plan = if left_alone {
        "left alone (your choice)".to_owned()
    } else if service.needs_change() && service.info.state == ServiceState::Stopped {
        format!("will set {}", service.target.label())
    } else if service.needs_change() {
        format!("will stop + set {}", service.target.label())
    } else {
        "already off".to_owned()
    };
    format!(
        "{:<44} {:<22} {now}; {plan}",
        service.label(),
        service.category
    )
}

fn task_line(task: &TaskFinding, left_alone: bool) -> String {
    let now = if task.info.enabled {
        "Enabled"
    } else {
        "Disabled"
    };
    let plan = if left_alone {
        "left alone (your choice)"
    } else if task.needs_change() {
        "will disable"
    } else {
        "already off"
    };
    format!("{:<44} {:<22} {now}; {plan}", task.label(), task.category)
}

fn scan(service: &ExamService) -> u8 {
    let left_open = left_open(service);
    let findings = match service.scan() {
        Ok(findings) => findings,
        Err(error) => {
            say(&format!("Scan failed: {error}"));
            return 1;
        }
    };
    if findings.is_empty() {
        say("Nothing from the exam list is running or installed.");
        return 0;
    }
    for app in &findings.apps {
        let note = if left_open.contains(&app.entry_id) {
            ", left open (your choice)"
        } else if app.launch.is_some() {
            ", will reopen"
        } else {
            ""
        };
        say(&format!(
            "{:<44} {:<22} {} process(es){note}",
            app.name,
            app.category,
            app.pids.len(),
        ));
    }
    for item in &findings.services {
        say(&service_line(item, left_open.contains(&item.key())));
    }
    for item in &findings.tasks {
        say(&task_line(item, left_open.contains(&item.key())));
    }
    0
}

/// Read-only report of startup items, listening ports and network adapters.
fn inventory(service: &ExamService, findings: &Findings) {
    let catalog = service.catalog();
    let inventory = service.inventory();
    say("");
    say("Startup items:");
    match inventory.startup_items() {
        Ok(items) => {
            for item in items {
                let owner = startup_owner(catalog, &item).unwrap_or("-");
                let state = if item.enabled { "on" } else { "off" };
                say(&format!(
                    "  {:<34} {state:<4} {:<26} app: {owner} | {}",
                    item.name, item.location, item.command
                ));
            }
        }
        Err(error) => say(&format!("  {error}")),
    }
    say("");
    say("Listening ports:");
    match inventory.listening_ports() {
        Ok(ports) => {
            for port in ports {
                let owner = port_owner_key(catalog, findings, &port).map(|(_, name)| name);
                let owner = owner.as_deref().unwrap_or("-");
                let reach = if port.is_network_facing() {
                    "network"
                } else {
                    "this PC only"
                };
                say(&format!(
                    "  {} {:<6} {:<40} {:<14} {} | app: {owner}",
                    port.protocol.label(),
                    port.port,
                    port.address,
                    reach,
                    port.process
                ));
            }
        }
        Err(error) => say(&format!("  {error}")),
    }
    say("");
    say("Network adapters:");
    match inventory.network_adapters() {
        Ok(adapters) => {
            for adapter in adapters {
                let kind = if is_virtual_adapter(catalog, &adapter) {
                    "VPN/virtual"
                } else {
                    "-"
                };
                let state = if adapter.up { "up" } else { "down" };
                say(&format!(
                    "  {:<34} {state:<5} {kind:<12} {}",
                    adapter.name, adapter.description
                ));
            }
        }
        Err(error) => say(&format!("  {error}")),
    }
}

fn status(service: &ExamService) -> u8 {
    match service.load_record() {
        Ok(None) => say("Exam mode: off"),
        Ok(Some(record)) => {
            say(&format!(
                "Exam mode: on ({} app(s), {} service(s), {} scheduled task(s) recorded)",
                record.closed_apps.len(),
                record.services.len(),
                record.tasks.len()
            ));
            for app in &record.closed_apps {
                say(&format!("  {}", describe_closed(app)));
            }
            for service in &record.services {
                say(&format!(
                    "  {} (service) -> {}{}",
                    service.display_name,
                    service.start_type.label(),
                    if service.was_running {
                        ", running"
                    } else {
                        ", stopped"
                    }
                ));
            }
            for task in &record.tasks {
                say(&format!(
                    "  {} (scheduled task) -> {}",
                    task.path,
                    if task.enabled { "enabled" } else { "disabled" }
                ));
            }
        }
        Err(error) => {
            say(&format!("Could not read the exam-mode record: {error}"));
            return 1;
        }
    }
    0
}

/// Runs the flow to completion. `restore` = start from exam mode and restore; otherwise make
/// safe, answering the confirm step with yes.
fn drive(service: &ExamService, restore: bool) -> u8 {
    let exam_mode_active = match service.load_record() {
        Ok(record) => record.is_some(),
        Err(error) => {
            say(&format!("Could not read the exam-mode record: {error}"));
            return 1;
        }
    };
    if restore && !exam_mode_active {
        say("Exam mode is off - nothing to restore.");
        return 0;
    }
    if !restore && exam_mode_active {
        say("Exam mode is already on. Run 'restore' first.");
        return 1;
    }

    let left_open = left_open(service);
    let mut flow = Flow::new();
    let mut selection = Findings::default();
    let mut queue = VecDeque::from([Event::Launched { exam_mode_active }, Event::PrimaryPressed]);
    while let Some(event) = queue.pop_front() {
        let effects = match flow.handle(event) {
            Ok(effects) => effects,
            Err(error) => {
                say(&format!("Internal error: {error}"));
                return 1;
            }
        };
        say(&format!("- {:?}", flow.phase()));
        for effect in effects {
            if effect == Effect::Quit {
                continue;
            }
            match run_effect(service, effect, &left_open, &selection, unix_now()) {
                Ok(output) => {
                    if let Some(found) = output.findings {
                        // Same rule as the window: items the user leaves alone are not selected.
                        selection = found.selection(&left_open);
                    }
                    queue.extend(output.event);
                }
                Err(reason) => {
                    queue.push_back(Event::Failed { reason });
                    break;
                }
            }
        }
        if flow.phase() == Phase::Confirm {
            say(&format!("  turning off: {}", selection.labels().join(", ")));
            queue.push_back(Event::PrimaryPressed);
        }
    }

    match flow.phase() {
        Phase::Ready if flow.issues() == 0 => {
            say("Done: nothing on the exam list needed turning off. Nothing to restore later.");
            0
        }
        Phase::Ready => {
            say(&format!(
                "Done: {} item(s) turned off and verified. Run 'restore' after the exam.",
                flow.issues()
            ));
            0
        }
        Phase::Restored => {
            say("Done: everything put back, exam mode off.");
            0
        }
        phase => {
            say(&format!(
                "Stopped in {phase:?}: {}",
                flow.failure().unwrap_or("unknown reason")
            ));
            1
        }
    }
}

/// One journal line: what will be started on restore, and how.
fn describe_closed(app: &ClosedApp) -> String {
    let Some(spec) = app.launch_spec() else {
        return format!("{}: not reopened automatically", app.name);
    };
    let target = match &spec.target {
        LaunchTarget::Path(path) => path.clone(),
        LaunchTarget::AppId(id) => format!("Store app {id}"),
    };
    let mut line = format!("{} -> {target}", app.name);
    if let Some(arguments) = &spec.arguments {
        line.push_str(&format!(" | args: {arguments}"));
    }
    if let Some(dir) = &spec.working_dir {
        line.push_str(&format!(" | in: {dir}"));
    }
    line.push_str(if spec.background {
        " | in the background"
    } else {
        " | with its window"
    });
    line
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
