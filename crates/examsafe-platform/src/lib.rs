//! OS adapters for ExamSafe.
//!
//! Implements the ports from `examsafe-core` for the current OS. All `unsafe` code in the project
//! lives in the `*_impl` modules of this crate, behind safe functions.

pub mod elevation;
pub mod helper_client;
pub mod paths;
#[cfg(windows)]
pub mod processes;
pub mod store;

#[cfg(windows)]
mod windows_impl;
