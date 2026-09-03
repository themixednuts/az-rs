//! Standalone `AZoth` editor entrypoint.
//!
//! Only the entry lives here. The startup sequence and its tests are in
//! `az_editor::startup`, which explains why: this target's test harness links
//! the GPUI Windows platform and cannot load on a server image.

use std::process::ExitCode;

fn main() -> ExitCode {
    az_editor::startup::run_from_env()
}
