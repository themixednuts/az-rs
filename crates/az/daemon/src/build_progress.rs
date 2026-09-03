//! Real build-percent feedback by streaming `cargo --message-format=json`.
//!
//! `cargo` already parallelizes internally, so the daemon runs ONE coalesced
//! invocation and counts completed compilation units off its JSON stream:
//! each `compiler-artifact` or `build-script-executed` record is one finished
//! unit. Unknown message kinds are tolerated (forward-compat with new cargo
//! versions). The denominator is the previous build's unit count, cached on
//! disk next to the shared service target dir; the first-ever build streams an
//! `Unknown` fraction plus a live "compiling <crate>" message, and subsequent
//! builds report an exact percentage.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use az_work::Fraction;
use serde::{Deserialize, Serialize};

const DIAGNOSTIC_TAIL_MAX_LINES: usize = 120;
const DIAGNOSTIC_HEADLINE_MAX_CHARS: usize = 220;

/// One semantic event distilled from a line of cargo JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CargoBuildEvent {
    /// A compilation unit (or build script) finished. Carries the target name
    /// for the live "compiling <crate>" message, if cargo provided one.
    UnitFinished { target: Option<String> },
    /// A rendered compiler diagnostic to forward to the console verbatim.
    Diagnostic { rendered: String },
    /// `build-finished`; carries cargo's own success flag.
    Finished { success: bool },
    /// A recognized-but-irrelevant or unknown message kind; ignored.
    Other,
}

/// Retains the tail of rendered Cargo diagnostics for the final build error.
#[derive(Debug, Default, Clone)]
pub struct DiagnosticTail {
    lines: VecDeque<String>,
}

impl DiagnosticTail {
    /// Add one rendered diagnostic chunk from Cargo.
    pub fn push_rendered(&mut self, rendered: &str) {
        for line in rendered.lines() {
            let line = line.trim_end();
            if line.trim().is_empty() {
                continue;
            }
            self.lines.push_back(truncate_diagnostic_line(
                line,
                DIAGNOSTIC_HEADLINE_MAX_CHARS,
            ));
            while self.lines.len() > DIAGNOSTIC_TAIL_MAX_LINES {
                self.lines.pop_front();
            }
        }
    }

    /// Format a suffix suitable for appending to the daemon error message.
    #[must_use]
    pub fn finish(&self) -> String {
        if self.lines.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nCargo diagnostics:\n{}",
                self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
            )
        }
    }
}

/// A one-line progress message from a rendered compiler diagnostic.
#[must_use]
pub fn rendered_diagnostic_headline(rendered: &str) -> Option<String> {
    rendered
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| truncate_diagnostic_line(line, DIAGNOSTIC_HEADLINE_MAX_CHARS))
}

/// Parse a single line of `cargo --message-format=json` output.
///
/// Hand-parsed with [`serde_json::Value`] rather than the `cargo_metadata`
/// crate so a future cargo message kind never breaks the build (we match on
/// the `reason` string and treat anything unknown as [`CargoBuildEvent::Other`]).
/// Non-JSON lines (cargo occasionally interleaves plain text) parse to `Other`.
#[must_use]
pub fn parse_cargo_line(line: &str) -> CargoBuildEvent {
    let line = line.trim();
    if line.is_empty() {
        return CargoBuildEvent::Other;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return CargoBuildEvent::Other;
    };
    match value.get("reason").and_then(|r| r.as_str()) {
        Some("compiler-artifact") => CargoBuildEvent::UnitFinished {
            target: value
                .get("target")
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string),
        },
        Some("build-script-executed") => CargoBuildEvent::UnitFinished { target: None },
        Some("compiler-message") => {
            let rendered = value
                .get("message")
                .and_then(|m| m.get("rendered"))
                .and_then(|r| r.as_str())
                .unwrap_or_default()
                .to_string();
            CargoBuildEvent::Diagnostic { rendered }
        }
        Some("build-finished") => CargoBuildEvent::Finished {
            success: value
                .get("success")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        _ => CargoBuildEvent::Other,
    }
}

fn truncate_diagnostic_line(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return line.to_string();
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

/// Running tally of finished compilation units against a cached denominator.
#[derive(Debug, Clone, Copy)]
pub struct UnitCounter {
    done: u64,
    total: Option<u64>,
}

impl UnitCounter {
    /// Start a counter. `total` is the cached denominator from the previous
    /// build, or `None` for a first-ever build (reported as `Unknown`).
    #[must_use]
    pub const fn new(total: Option<u64>) -> Self {
        Self { done: 0, total }
    }

    /// Record one finished unit. Returns the new fraction (clamped to total).
    pub fn advance(&mut self) -> Fraction {
        self.done = self.done.saturating_add(1);
        self.fraction()
    }

    /// Current fraction without advancing.
    #[must_use]
    pub fn fraction(&self) -> Fraction {
        self.total
            .map_or(Fraction::Unknown { done: self.done }, |total| {
                Fraction::exact(self.done, total)
            })
    }

    /// Units finished so far (the next build's denominator).
    #[must_use]
    pub const fn done(&self) -> u64 {
        self.done
    }
}

/// On-disk cache of the previous build's unit count, keyed by cargo version so
/// a cargo upgrade re-learns the denominator instead of trusting a stale one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildUnitCache {
    pub unit_count: u64,
    pub cargo_version: String,
}

/// Cache file path next to the shared service target dir.
#[must_use]
pub fn unit_cache_path(service_target_dir: &Path) -> PathBuf {
    service_target_dir.join(".azoth-build-units.json")
}

/// Read the cached denominator if it matches the current cargo version.
#[must_use]
pub fn read_cached_unit_count(service_target_dir: &Path, cargo_version: &str) -> Option<u64> {
    let path = unit_cache_path(service_target_dir);
    let bytes = std::fs::read(&path).ok()?;
    let cache: BuildUnitCache = serde_json::from_slice(&bytes).ok()?;
    (cache.cargo_version == cargo_version).then_some(cache.unit_count)
}

/// Persist the denominator for the next build. Best-effort: a write failure is
/// not fatal (the next build merely reports `Unknown`).
pub fn write_cached_unit_count(service_target_dir: &Path, unit_count: u64, cargo_version: &str) {
    let path = unit_cache_path(service_target_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cache = BuildUnitCache {
        unit_count,
        cargo_version: cargo_version.to_string(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = std::fs::write(&path, bytes);
    }
}

/// Best-effort `cargo --version` string (used as the cache key). Falls back to
/// a constant when cargo cannot be probed so caching is simply disabled.
#[must_use]
pub fn cargo_version() -> String {
    let mut command = std::process::Command::new("cargo");
    command.arg("--version");
    az_work::owned_command_output(&mut command)
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A captured slice of real `cargo --message-format=json` output, including
    // an unknown future kind that must be tolerated.
    const SAMPLE: &str = r#"{"reason":"compiler-artifact","package_id":"a 0.1.0","target":{"name":"a"},"filenames":["x.rlib"]}
{"reason":"build-script-executed","package_id":"b 0.1.0"}
{"reason":"future-cargo-kind-we-do-not-know","extra":{"nested":1}}
{"reason":"compiler-message","message":{"rendered":"warning: unused import\n"},"target":{"name":"a"}}
{"reason":"compiler-artifact","package_id":"c 0.1.0","target":{"name":"c-bin"}}
not even json, cargo sometimes interleaves plain text
{"reason":"build-finished","success":true}"#;

    #[test]
    fn parses_unit_count_and_tolerates_unknown_kinds() {
        let mut counter = UnitCounter::new(Some(2));
        let mut finished = None;
        let mut diagnostics = Vec::new();
        let mut last_target = None;

        for line in SAMPLE.lines() {
            match parse_cargo_line(line) {
                CargoBuildEvent::UnitFinished { target } => {
                    if let Some(t) = &target {
                        last_target = Some(t.clone());
                    }
                    counter.advance();
                }
                CargoBuildEvent::Diagnostic { rendered } => diagnostics.push(rendered),
                CargoBuildEvent::Finished { success } => finished = Some(success),
                CargoBuildEvent::Other => {}
            }
        }

        // Three units finished: two compiler-artifact + one build-script.
        assert_eq!(counter.done(), 3);
        // Denominator was 2, so the exact fraction clamps to 100%.
        assert_eq!(counter.fraction().to_basis_points(), 10_000);
        assert_eq!(finished, Some(true));
        assert_eq!(diagnostics, vec!["warning: unused import\n".to_string()]);
        assert_eq!(last_target.as_deref(), Some("c-bin"));
    }

    #[test]
    fn first_ever_build_reports_unknown() {
        let mut counter = UnitCounter::new(None);
        assert!(matches!(counter.advance(), Fraction::Unknown { done: 1 }));
        assert_eq!(counter.fraction().to_basis_points(), 0);
    }

    #[test]
    fn unit_cache_round_trips_and_honors_version() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path();
        assert_eq!(read_cached_unit_count(target, "cargo 1.0"), None);

        write_cached_unit_count(target, 42, "cargo 1.0");
        assert_eq!(read_cached_unit_count(target, "cargo 1.0"), Some(42));
        // A cargo upgrade invalidates the cache.
        assert_eq!(read_cached_unit_count(target, "cargo 2.0"), None);
    }

    #[test]
    fn non_json_and_empty_lines_are_other() {
        assert_eq!(parse_cargo_line(""), CargoBuildEvent::Other);
        assert_eq!(parse_cargo_line("   "), CargoBuildEvent::Other);
        assert_eq!(parse_cargo_line("plain text"), CargoBuildEvent::Other);
        assert_eq!(parse_cargo_line("{}"), CargoBuildEvent::Other);
    }

    #[test]
    fn diagnostic_tail_formats_recent_rendered_lines() {
        let mut tail = DiagnosticTail::default();
        tail.push_rendered("warning: first\n\n  note: kept\n");
        tail.push_rendered("error[E0433]: cannot find module or crate `az_core`\n");

        let diagnostics = tail.finish();
        assert!(diagnostics.contains("Cargo diagnostics:"));
        assert!(diagnostics.contains("warning: first"));
        assert!(diagnostics.contains("note: kept"));
        assert!(diagnostics.contains("error[E0433]: cannot find module or crate `az_core`"));
    }

    #[test]
    fn rendered_diagnostic_headline_uses_first_non_empty_line() {
        assert_eq!(
            rendered_diagnostic_headline("\n  error[E0433]: unresolved import\n  --> src/lib.rs:1"),
            Some("error[E0433]: unresolved import".to_string())
        );
        assert_eq!(rendered_diagnostic_headline("\n \n"), None);
    }
}
