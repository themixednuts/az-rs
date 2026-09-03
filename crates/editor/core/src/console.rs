//! Editor-owned console command execution.
//!
//! The UI crate owns the console panel and log state. This module owns the
//! attached-session command boundary so console commands run through the
//! session supervisor instead of from the editor process directly.

use az_editor_ui::panels::{Console, ConsoleState, LogLevel};
use gpui::App;
use tracing::{error, info};

use crate::attach::EditorAttachSession;
use crate::error::{EditorError, EditorResult};
use crate::session_supervisor::SessionSupervisorClient;

const CONSOLE_EXEC_OUTPUT_LIMIT_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Register editor-shell action handlers for console commands.
pub fn install_console_action_handlers(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::ExecuteConsoleCommand, cx| {
            if let Err(err) = submit_console_command(cx, &action.command) {
                error!(error = %err, "failed to handle console command action");
            }
        },
    );
}

/// Submit a console command from GPUI and run it through the attached session.
///
/// # Errors
///
/// Returns [`EditorError::InvalidConsoleCommand`] if `command` does not parse
/// (see [`parse_console_command`]), or
/// [`EditorError::MissingAttachedSession`] if no session is attached. Both are
/// also logged to the console before being returned. A blank command is a
/// no-op and succeeds.
pub fn submit_console_command(cx: &mut App, command: &str) -> EditorResult<()> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Ok(());
    }

    Console::execute_command(cx, command.clone());
    let parsed = match parse_console_command(&command) {
        Ok(parsed) => parsed,
        Err(err) => {
            Console::log_global(cx, LogLevel::Error, err.to_string());
            return Err(err);
        }
    };
    let Some(session) = cx.try_global::<EditorAttachSession>().cloned() else {
        let err = EditorError::MissingAttachedSession {
            operation: "console command execution",
        };
        Console::log_global(cx, LogLevel::Error, err.to_string());
        return Err(err);
    };

    spawn_session_console_command(cx, session, parsed);
    Ok(())
}

fn spawn_session_console_command(cx: &App, session: EditorAttachSession, command: ConsoleCommand) {
    let session_slug = session.session_slug.clone();
    let program = command.program.clone();
    cx.spawn(async move |cx| {
        let result = async {
            let supervisor = SessionSupervisorClient::connect_for_session(
                &session.session_supervisor,
                session.session_id,
            )
            .await?;
            supervisor
                .exec_command(
                    &session.session_slug,
                    command.program,
                    command.args,
                    CONSOLE_EXEC_OUTPUT_LIMIT_BYTES,
                )
                .await
        }
        .await;

        cx.update(move |cx| {
            let state = cx.default_global::<ConsoleState>();
            match result {
                Ok(result) => {
                    log_command_output(state, LogLevel::Info, &result.stdout);
                    log_command_output(state, LogLevel::Warn, &result.stderr);
                    if result.stdout_truncated {
                        state.log(LogLevel::Warn, "[stdout truncated]");
                    }
                    if result.stderr_truncated {
                        state.log(LogLevel::Warn, "[stderr truncated]");
                    }
                    if result.success {
                        state.log(LogLevel::Info, "command completed");
                    } else {
                        state.log(
                            LogLevel::Error,
                            format!(
                                "command failed: {}",
                                command_exit_label(result.exited, result.exit_code)
                            ),
                        );
                    }
                }
                Err(err) => {
                    state.log(LogLevel::Error, format!("command failed: {err}"));
                }
            }
            cx.refresh_windows();
        });

        info!(
            session = %session_slug,
            program = %program,
            "handled editor console command"
        );
    })
    .detach();
}

fn log_command_output(state: &mut ConsoleState, level: LogLevel, output: &str) {
    for line in output.lines() {
        if !line.is_empty() {
            state.log(level, line);
        }
    }
}

fn command_exit_label(exited: bool, exit_code: i32) -> String {
    if exited {
        format!("exit code {exit_code}")
    } else {
        "process terminated without an exit code".to_string()
    }
}

/// Split a console command line into a program and its arguments, honouring
/// single quotes, double quotes and backslash escapes.
///
/// # Errors
///
/// Returns [`EditorError::InvalidConsoleCommand`] if the line ends in a
/// trailing escape (inside or outside double quotes), leaves a quote
/// unterminated, or contains no program token.
pub fn parse_console_command(command: &str) -> EditorResult<ConsoleCommand> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut chars = command.chars();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    token.push(ch);
                }
            }
            Some('"') => match ch {
                '"' => quote = None,
                '\\' => {
                    let escaped = chars.next().ok_or_else(|| {
                        invalid_console_command(command, "trailing escape in double-quoted text")
                    })?;
                    token.push(escaped);
                }
                _ => token.push(ch),
            },
            Some(_) => unreachable!("quote is limited to single or double quote"),
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => {
                    let escaped = chars.next().ok_or_else(|| {
                        invalid_console_command(command, "trailing escape in command")
                    })?;
                    token.push(escaped);
                }
                ch if ch.is_whitespace() => {
                    if !token.is_empty() {
                        tokens.push(std::mem::take(&mut token));
                    }
                }
                _ => token.push(ch),
            },
        }
    }

    if let Some(quote) = quote {
        return Err(invalid_console_command(
            command,
            &format!("unterminated {quote} quote"),
        ));
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    let Some(program) = tokens.first().cloned() else {
        return Err(invalid_console_command(command, "missing program"));
    };

    Ok(ConsoleCommand {
        program,
        args: tokens.into_iter().skip(1).collect(),
    })
}

fn invalid_console_command(command: &str, message: &str) -> EditorError {
    EditorError::InvalidConsoleCommand {
        command: command.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_console_command_with_quotes_and_escapes() {
        let parsed =
            parse_console_command(r#"cargo test -p "project tools" -- "quoted arg" path\ value"#)
                .unwrap();

        assert_eq!(parsed.program, "cargo");
        assert_eq!(
            parsed.args,
            vec![
                "test",
                "-p",
                "project tools",
                "--",
                "quoted arg",
                "path value",
            ]
        );
    }

    #[test]
    fn rejects_unterminated_console_command_quote() {
        let error = parse_console_command(r#"cargo "unterminated"#).unwrap_err();

        assert!(error.to_string().contains("unterminated"));
    }
}
