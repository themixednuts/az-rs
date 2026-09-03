//! Argument surface for the standalone `az-editor` binary.
//!
//! Parsing lives in the library, not in `src/bin/az-editor.rs`, because the
//! binary's own test harness cannot load on a Windows server image: linking
//! the GUI entry pulls in the GPUI Windows platform, whose import table names
//! a `DirectComposition` entry those images do not export. The library harness
//! links none of that, so the arguments stay covered there.

use std::io::{self, IsTerminal as _};
use std::path::PathBuf;

use az_proto_core::EndpointKind;
use clap::{ArgAction, Parser, ValueEnum, error::ErrorKind};

use crate::{EditorError, EditorResult};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct EditorCli {
    pub verbose: u8,
    pub quiet: bool,
    pub color: ColorArg,
    pub viewport_diagnostic: bool,
    pub viewport_present_policy: ViewportPresentPolicyArg,
    pub ui_present_policy: UiPresentPolicyArg,
    pub trace_ui: bool,
    pub theme_dir: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
    pub asset_processor_project_root: Option<PathBuf>,
    pub session: Option<String>,
    pub daemon_endpoint_kind: Option<EndpointKind>,
    pub daemon_endpoint: Option<String>,
}

#[derive(Debug, Default, Parser, PartialEq, Eq)]
#[command(
    name = "az-editor",
    version,
    about = "Launch the standalone AZoth editor and optionally attach it to a project session",
    after_help = "Environment:\n  RUST_LOG                              Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR                              Disable automatic color output.\n  AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC      Default for --viewport-diagnostic.\n  AZOTH_EDITOR_VIEWPORT_PRESENT_POLICY  Default for --viewport-present-policy.\n  AZOTH_EDITOR_UI_PRESENT_POLICY        Default for --ui-present-policy.\n  AZOTH_EDITOR_TRACE_UI                 Default for --trace-ui.\n  AZOTH_EDITOR_THEME_DIR                Default for --theme-dir."
)]
pub struct RawEditorCli {
    /// Increase default log verbosity (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Restrict default diagnostics to errors. `RUST_LOG` can add directives.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,

    /// When to colorize diagnostic output.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,

    /// Show diagnostic viewport geometry and render a diagnostic test scene.
    #[arg(
        long,
        env = "AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC",
        value_name = "BOOL",
        value_parser = parse_bool_arg,
        num_args = 0..=1,
        default_missing_value = "true",
        default_value_t = false
    )]
    viewport_diagnostic: bool,

    /// Presentation policy and maximum queued-frame latency for the viewport.
    #[arg(
        long,
        env = "AZOTH_EDITOR_VIEWPORT_PRESENT_POLICY",
        value_enum,
        default_value_t = ViewportPresentPolicyArg::Immediate2
    )]
    viewport_present_policy: ViewportPresentPolicyArg,

    /// Presentation policy for the GPUI chrome swapchain.
    #[arg(
        long,
        env = "AZOTH_EDITOR_UI_PRESENT_POLICY",
        value_enum,
        default_value_t = UiPresentPolicyArg::Auto
    )]
    ui_present_policy: UiPresentPolicyArg,

    /// Mirror deduplicated editor UI render-state traces to stderr.
    #[arg(
        long,
        env = "AZOTH_EDITOR_TRACE_UI",
        value_name = "BOOL",
        value_parser = parse_bool_arg,
        num_args = 0..=1,
        default_missing_value = "true",
        default_value_t = false
    )]
    trace_ui: bool,

    /// Directory containing editor theme files.
    #[arg(long, env = "AZOTH_EDITOR_THEME_DIR", value_name = "DIR")]
    theme_dir: Option<PathBuf>,

    /// Project directory to open. Omit it to show the project launcher.
    #[arg(
        value_name = "DIR",
        conflicts_with_all = ["path", "asset_processor_project_root"]
    )]
    project_root: Option<PathBuf>,

    /// Project directory to open. This is the named form of `<PROJECT_ROOT>`.
    #[arg(
        long = "project",
        alias = "path",
        value_name = "DIR",
        conflicts_with = "asset_processor_project_root"
    )]
    path: Option<PathBuf>,

    /// Project directory whose asset-processing session should be opened directly.
    #[arg(
        long = "asset-processor",
        value_name = "PROJECT_ROOT",
        conflicts_with_all = ["project_root", "path"]
    )]
    asset_processor_project_root: Option<PathBuf>,

    /// Lore workspace session to open inside the selected project.
    #[arg(long)]
    session: Option<String>,

    /// Transport used to connect to the project daemon.
    #[arg(long = "daemon-endpoint-kind", value_parser = parse_endpoint_kind_arg)]
    daemon_endpoint_kind: Option<EndpointKind>,

    /// Explicit project-daemon address. Omit it to use project discovery.
    #[arg(long)]
    daemon_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    /// Colorize diagnostics only on a terminal when `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always colorize diagnostic output.
    Always,
    /// Never colorize diagnostic output.
    Never,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ViewportPresentPolicyArg {
    /// FIFO presentation with one queued frame.
    #[value(name = "fifo-1")]
    Fifo1,
    /// FIFO presentation with two queued frames.
    #[value(name = "fifo-2")]
    Fifo2,
    /// Mailbox presentation with one queued frame.
    #[value(name = "mailbox-1")]
    Mailbox1,
    /// Mailbox presentation with two queued frames.
    #[value(name = "mailbox-2")]
    Mailbox2,
    /// Immediate presentation with one queued frame.
    #[value(name = "immediate-1")]
    Immediate1,
    /// Immediate presentation with two queued frames.
    #[default]
    #[value(name = "immediate-2")]
    Immediate2,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum UiPresentPolicyArg {
    /// Select the normal low-latency policy for the active compositor.
    #[default]
    Auto,
    /// Disable tearing and use the waitable swapchain path.
    Waitable,
}

impl UiPresentPolicyArg {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Waitable => "waitable",
        }
    }
}

impl ViewportPresentPolicyArg {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Fifo1 => "fifo-1",
            Self::Fifo2 => "fifo-2",
            Self::Mailbox1 => "mailbox-1",
            Self::Mailbox2 => "mailbox-2",
            Self::Immediate1 => "immediate-1",
            Self::Immediate2 => "immediate-2",
        }
    }
}

impl ColorArg {
    pub(crate) fn stderr_ansi(self) -> bool {
        match self {
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

pub const fn default_log_directives(verbosity: u8, quiet: bool) -> &'static str {
    match (quiet, verbosity) {
        (true, _) => "error",
        (false, 0) => "warn",
        (false, 1) => "info",
        (false, 2) => "debug",
        (false, _) => "trace",
    }
}

impl EditorCli {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> EditorResult<Self> {
        let raw = parse_raw_editor_cli(args)?;
        Ok(Self::from(raw))
    }
}

impl From<RawEditorCli> for EditorCli {
    fn from(raw: RawEditorCli) -> Self {
        Self {
            verbose: raw.verbose,
            quiet: raw.quiet,
            color: raw.color,
            viewport_diagnostic: raw.viewport_diagnostic,
            viewport_present_policy: raw.viewport_present_policy,
            ui_present_policy: raw.ui_present_policy,
            trace_ui: raw.trace_ui,
            theme_dir: raw.theme_dir,
            project_root: raw.path.or(raw.project_root),
            asset_processor_project_root: raw.asset_processor_project_root,
            session: raw.session,
            daemon_endpoint_kind: raw.daemon_endpoint_kind,
            daemon_endpoint: raw.daemon_endpoint,
        }
    }
}

fn parse_bool_arg(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err("expected one of 1, true, yes, on, 0, false, no, or off".to_owned()),
    }
}

fn parse_raw_editor_cli(args: impl IntoIterator<Item = String>) -> EditorResult<RawEditorCli> {
    let args = std::iter::once("az-editor".to_string()).chain(args);
    RawEditorCli::try_parse_from(args).map_err(|error| {
        if matches!(
            error.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        ) {
            let _ = error.print();
            std::process::exit(0);
        }

        EditorError::InvalidArgument(error.to_string())
    })
}

fn parse_endpoint_kind_arg(value: &str) -> Result<EndpointKind, String> {
    match value {
        "windows-named-pipe" => Ok(EndpointKind::WindowsNamedPipe),
        "unix-domain-socket" => Ok(EndpointKind::UnixDomainSocket),
        "tcp" => Ok(EndpointKind::Tcp),
        "in-process" => Err(
            "`in-process` daemon endpoints are test-only; az-editor must attach through IPC"
                .to_string(),
        ),
        _ => Err(format!("unsupported endpoint kind `{value}`")),
    }
}

pub fn validate_unbound_launcher_args(
    session: Option<&str>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<&str>,
) -> EditorResult<()> {
    if session.is_some() {
        return Err(EditorError::InvalidArgument(
            "`az-editor --session <name>` requires `--project <DIR>`; omit both to show the project launcher".to_string(),
        ));
    }
    if daemon_endpoint_kind.is_some() || daemon_endpoint.is_some() {
        return Err(EditorError::InvalidArgument(
            "daemon endpoint options require `--project <DIR>` because daemon attach is project-bound".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_daemon_endpoint_kind_without_address() {
        let cli = EditorCli::parse([
            "--project".to_string(),
            "projects/example".to_string(),
            "--session".to_string(),
            "editor-work".to_string(),
            "--daemon-endpoint-kind".to_string(),
            "tcp".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.daemon_endpoint_kind, Some(EndpointKind::Tcp));
        assert_eq!(cli.daemon_endpoint, None);
    }

    #[test]
    fn parse_accepts_positional_project_root_without_manual_parser() {
        let cli = EditorCli::parse([
            "projects/example".to_string(),
            "--session".to_string(),
            "editor-work".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.project_root, Some(PathBuf::from("projects/example")));
        assert_eq!(cli.session.as_deref(), Some("editor-work"));
    }

    #[test]
    fn parse_accepts_asset_processor_shell_project_root() {
        let cli = EditorCli::parse([
            "--asset-processor".to_string(),
            "projects/example".to_string(),
            "--session".to_string(),
            "editor-work".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.project_root, None);
        assert_eq!(
            cli.asset_processor_project_root,
            Some(PathBuf::from("projects/example"))
        );
        assert_eq!(cli.session.as_deref(), Some("editor-work"));
    }

    #[test]
    fn parse_rejects_conflicting_project_root_forms() {
        let error = EditorCli::parse([
            "--project".to_string(),
            "projects/example".to_string(),
            "projects/other".to_string(),
        ])
        .unwrap_err();

        assert!(
            error.to_string().contains("cannot be used with"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_rejects_asset_processor_with_workspace_project_root() {
        let error = EditorCli::parse([
            "--asset-processor".to_string(),
            "projects/example".to_string(),
            "--project".to_string(),
            "projects/other".to_string(),
        ])
        .unwrap_err();

        assert!(
            error.to_string().contains("cannot be used with"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_rejects_in_process_daemon_endpoint_kind() {
        let error = EditorCli::parse([
            "--project".to_string(),
            "projects/example".to_string(),
            "--daemon-endpoint-kind".to_string(),
            "in-process".to_string(),
        ])
        .unwrap_err();

        assert!(
            error.to_string().contains("test-only")
                && error.to_string().contains("attach through IPC"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unbound_launcher_rejects_project_bound_attach_args() {
        let error = validate_unbound_launcher_args(Some("main"), None, None).unwrap_err();
        assert!(matches!(error, EditorError::InvalidArgument(message)
            if message.contains("--session") && message.contains("--project")));

        let error =
            validate_unbound_launcher_args(None, Some(EndpointKind::Tcp), None).unwrap_err();
        assert!(matches!(error, EditorError::InvalidArgument(message)
            if message.contains("daemon endpoint") && message.contains("--project")));
    }
}
