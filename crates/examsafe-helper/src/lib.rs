//! Privileged helper logic for ExamSafe.
//!
//! The ExamSafe executable relaunches itself with administrator rights as
//! `examsafe.exe --helper --request <hex>`; `main` sees the flag and calls [`run`] *before* any UI
//! is created. It performs one action, writes a JSON response to the validated response path,
//! and the process exits. Nothing here ever runs in the background.
//!
//! Kept as its own crate (not inside the app) so the privileged code path stays small, UI-free
//! and separately testable — enforced by `tools/check-architecture.ps1`.

use examsafe_core::protocol::{
    HelperAction, HelperRequest, HelperResponse, PROTOCOL_VERSION, decode_request,
};
use examsafe_core::system::SystemChange;
use examsafe_platform::elevation::is_elevated;

/// Runs helper mode. `args` are the arguments after the helper-mode flag.
/// Returns the process exit code: 0 on success, 2 on any failure (details on stderr).
pub fn run(args: &[String]) -> u8 {
    match handle(args) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("examsafe helper: {message}");
            2
        }
    }
}

fn handle(args: &[String]) -> Result<(), String> {
    let encoded = parse_args(args)?;
    let request = decode_request(encoded).map_err(|error| error.to_string())?;
    let response = execute(&request);
    let json = serde_json::to_string(&response).map_err(|error| error.to_string())?;
    std::fs::write(&request.response_path, json)
        .map_err(|error| format!("could not write the response: {error}"))
}

fn parse_args(args: &[String]) -> Result<&str, String> {
    match args {
        [flag, value] if flag == "--request" => Ok(value),
        _ => Err("usage: examsafe --helper --request <hex>".to_owned()),
    }
}

fn execute(request: &HelperRequest) -> HelperResponse {
    let elevated = is_elevated();
    let mut failures = Vec::new();
    let (ok, message) = match (&request.action, &elevated) {
        (_, Err(error)) => (false, format!("could not read own rights: {error}")),
        (HelperAction::Ping, Ok(true)) => (true, "Responded with administrator rights".to_owned()),
        (HelperAction::Ping, Ok(false)) => {
            (true, "Responded without administrator rights".to_owned())
        }
        (HelperAction::ApplyChanges { changes }, Ok(_)) => {
            failures = apply_all(changes, apply_change);
            if failures.is_empty() {
                (true, format!("{} change(s) applied", changes.len()))
            } else {
                (false, failures.join("; "))
            }
        }
    };
    HelperResponse {
        protocol: PROTOCOL_VERSION,
        id: request.id.clone(),
        ok,
        elevated: elevated.unwrap_or(false),
        message,
        failures,
    }
}

/// Applies every change in order, continuing past failures (each change is independent and
/// states its full end state). Returns "Item: reason" per failure.
fn apply_all(
    changes: &[SystemChange],
    apply: impl Fn(&SystemChange) -> Result<(), String>,
) -> Vec<String> {
    changes
        .iter()
        .filter_map(|change| {
            apply(change)
                .err()
                .map(|reason| format!("{}: {reason}", change.subject()))
        })
        .collect()
}

#[cfg(windows)]
fn apply_change(change: &SystemChange) -> Result<(), String> {
    examsafe_platform::system::apply_change(change)
}

#[cfg(not(windows))]
fn apply_change(_change: &SystemChange) -> Result<(), String> {
    Err("not supported on this platform yet".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_request_flag() {
        let args = vec!["--request".to_owned(), "00ff".to_owned()];
        assert_eq!(parse_args(&args), Ok("00ff"));
    }

    #[test]
    fn rejects_anything_else() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--other".to_owned(), "x".to_owned()]).is_err());
        assert!(parse_args(&["--request".to_owned()]).is_err());
    }

    #[test]
    fn bad_request_fails_with_exit_code_2() {
        assert_eq!(run(&["--request".to_owned(), "zz".to_owned()]), 2);
    }

    #[test]
    fn applies_every_change_and_names_the_failures() {
        let changes = vec![
            SystemChange::Task {
                path: "\\A".into(),
                enabled: false,
            },
            SystemChange::Task {
                path: "\\B".into(),
                enabled: false,
            },
            SystemChange::Task {
                path: "\\C".into(),
                enabled: false,
            },
        ];
        let failures = apply_all(&changes, |change| {
            if change.subject() == "\\B" {
                Err("access denied".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(failures, vec!["\\B: access denied"]);
    }

    #[test]
    fn ping_answers_with_the_request_id() {
        let request = HelperRequest {
            protocol: PROTOCOL_VERSION,
            id: "id-1".into(),
            action: HelperAction::Ping,
            response_path: String::new(),
        };
        let response = execute(&request);
        assert_eq!(response.id, "id-1");
        assert_eq!(response.protocol, PROTOCOL_VERSION);
        if cfg!(windows) {
            assert!(response.ok, "{}", response.message);
        }
    }
}
