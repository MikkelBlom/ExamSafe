//! Platform-agnostic domain logic for ExamSafe.
//!
//! Rules for this crate (enforced by `tools/check-architecture.ps1` in CI):
//! - no I/O, no OS APIs, no UI — everything here is deterministic and unit tested;
//! - talks to the outside world only through the traits in [`ports`], which the
//!   `examsafe-platform` crate implements.

pub mod apps;
pub mod exam_mode;
pub mod flow;
pub mod ports;
pub mod preferences;
pub mod protocol;
pub mod service;
pub mod steps;
