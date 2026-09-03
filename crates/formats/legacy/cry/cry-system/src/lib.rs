//! `CrySystem` configuration formats.

use std::{
    fmt, io,
    path::{Path, PathBuf},
    str,
};

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("CrySystem config is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),
}

pub type ConfigParseError = ParseError;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse CrySystem config {path:?}: {source}")]
    Parse { path: PathBuf, source: ParseError },
}

pub const CONFIG_EXTENSION: &str = "cfg";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile<'a> {
    lines: Vec<ConfigLine<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLine<'a> {
    Blank,
    Comment(ConfigComment<'a>),
    Section(ConfigSection<'a>),
    Assignment(ConfigAssignment<'a>),
    Command(ConfigCommand<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigComment<'a> {
    marker: CommentMarker,
    text: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentMarker {
    Semicolon,
    DoubleDash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigSection<'a> {
    name: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigAssignment<'a> {
    key: &'a str,
    value: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigCommand<'a> {
    name: &'a str,
    args: &'a str,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigSummary {
    pub lines: usize,
    pub assignments: usize,
    pub sections: usize,
    pub commands: usize,
    pub comments: usize,
    pub blanks: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigTotals {
    pub files: usize,
    pub lines: usize,
    pub assignments: usize,
    pub sections: usize,
    pub commands: usize,
    pub comments: usize,
    pub blanks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileSummary {
    pub source: String,
    pub summary: ConfigSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConfigInspection {
    pub rows: Vec<ConfigFileSummary>,
    pub totals: ConfigTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigInspectionReport<'a> {
    inspection: &'a ConfigInspection,
    limit: usize,
}

impl<'a> ConfigFile<'a> {
    /// Parses a `CrySystem` config from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidUtf8`] if `bytes` is not valid UTF-8. Line
    /// classification itself never fails: an unrecognised line is parsed as a
    /// [`ConfigLine::Command`].
    pub fn parse_bytes(bytes: &'a [u8]) -> Result<Self, ParseError> {
        Ok(Self::parse_str(str::from_utf8(bytes)?))
    }

    /// Parses a `CrySystem` config from an already-decoded string.
    ///
    pub fn parse_str(input: &'a str) -> Self {
        Self {
            lines: input.lines().map(parse_line).collect(),
        }
    }

    #[must_use]
    #[inline]
    pub fn lines(&self) -> &[ConfigLine<'a>] {
        &self.lines
    }

    /// Borrowing iterator over the parsed lines, in file order.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, ConfigLine<'a>> {
        self.lines.iter()
    }

    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn assignments(&self) -> impl Iterator<Item = ConfigAssignment<'a>> + '_ {
        self.lines.iter().filter_map(|line| match line {
            ConfigLine::Assignment(assignment) => Some(*assignment),
            ConfigLine::Blank
            | ConfigLine::Comment(_)
            | ConfigLine::Section(_)
            | ConfigLine::Command(_) => None,
        })
    }

    pub fn sections(&self) -> impl Iterator<Item = ConfigSection<'a>> + '_ {
        self.lines.iter().filter_map(|line| match line {
            ConfigLine::Section(section) => Some(*section),
            ConfigLine::Blank
            | ConfigLine::Comment(_)
            | ConfigLine::Assignment(_)
            | ConfigLine::Command(_) => None,
        })
    }

    pub fn commands(&self) -> impl Iterator<Item = ConfigCommand<'a>> + '_ {
        self.lines.iter().filter_map(|line| match line {
            ConfigLine::Command(command) => Some(*command),
            ConfigLine::Blank
            | ConfigLine::Comment(_)
            | ConfigLine::Section(_)
            | ConfigLine::Assignment(_) => None,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ConfigSummary {
        ConfigSummary::from_config(self)
    }
}

impl<'a> IntoIterator for ConfigFile<'a> {
    type IntoIter = std::vec::IntoIter<ConfigLine<'a>>;
    type Item = ConfigLine<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.lines.into_iter()
    }
}

impl<'file, 'a> IntoIterator for &'file ConfigFile<'a> {
    type IntoIter = std::slice::Iter<'file, ConfigLine<'a>>;
    type Item = &'file ConfigLine<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

impl<'a> ConfigComment<'a> {
    #[must_use]
    #[inline]
    pub const fn marker(&self) -> CommentMarker {
        self.marker
    }

    #[must_use]
    #[inline]
    pub const fn text(&self) -> &'a str {
        self.text
    }
}

impl<'a> ConfigSection<'a> {
    #[must_use]
    #[inline]
    pub const fn name(&self) -> &'a str {
        self.name
    }
}

impl<'a> ConfigAssignment<'a> {
    #[must_use]
    #[inline]
    pub const fn key(&self) -> &'a str {
        self.key
    }

    #[must_use]
    #[inline]
    pub const fn value(&self) -> &'a str {
        self.value
    }
}

impl<'a> ConfigCommand<'a> {
    #[must_use]
    #[inline]
    pub const fn name(&self) -> &'a str {
        self.name
    }

    #[must_use]
    #[inline]
    pub const fn args(&self) -> &'a str {
        self.args
    }
}

impl ConfigSummary {
    #[must_use]
    pub fn from_config(config: &ConfigFile<'_>) -> Self {
        let mut summary = Self::default();
        for line in config.lines() {
            summary.lines += 1;
            match line {
                ConfigLine::Blank => summary.blanks += 1,
                ConfigLine::Comment(_) => summary.comments += 1,
                ConfigLine::Section(_) => summary.sections += 1,
                ConfigLine::Assignment(_) => summary.assignments += 1,
                ConfigLine::Command(_) => summary.commands += 1,
            }
        }
        summary
    }
}

impl fmt::Display for ConfigSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} assignments, {} sections, {} commands, {} comments",
            self.assignments, self.sections, self.commands, self.comments
        )
    }
}

impl ConfigTotals {
    pub const fn add_summary(&mut self, summary: ConfigSummary) {
        self.files += 1;
        self.lines += summary.lines;
        self.assignments += summary.assignments;
        self.sections += summary.sections;
        self.commands += summary.commands;
        self.comments += summary.comments;
        self.blanks += summary.blanks;
    }
}

impl ConfigInspection {
    pub fn add_file_summary(&mut self, row: ConfigFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> ConfigInspectionReport<'_> {
        ConfigInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for ConfigTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  lines: {}", self.lines)?;
        writeln!(f, "  assignments: {}", self.assignments)?;
        writeln!(f, "  sections: {}", self.sections)?;
        writeln!(f, "  commands: {}", self.commands)?;
        writeln!(f, "  comments: {}", self.comments)?;
        writeln!(f, "  blanks: {}", self.blanks)
    }
}

impl fmt::Display for ConfigInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.summary)?;
            }

            if self.inspection.rows.len() > self.limit {
                writeln!(
                    f,
                    "... {} more files",
                    self.inspection.rows.len() - self.limit
                )?;
            }
        }

        write!(f, "{}", self.inspection.totals)
    }
}

/// Counts the line kinds in one config file's bytes.
///
/// # Errors
///
/// Returns any error [`ConfigFile::parse_bytes`] returns — in practice
/// [`ParseError::InvalidUtf8`] when `bytes` is not valid UTF-8.
pub fn summarize_config_file(bytes: &[u8]) -> Result<ConfigSummary, ParseError> {
    ConfigFile::parse_bytes(bytes).map(|config| config.summary())
}

/// Summarises one config file's bytes, labelling the row with `path`.
///
/// `path` is only used as the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_config_file`] returns — in practice
/// [`ParseError::InvalidUtf8`] when `bytes` is not valid UTF-8.
pub fn inspect_config_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<ConfigFileSummary, ParseError> {
    Ok(ConfigFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_config_file(bytes)?,
    })
}

/// Reads a config file from disk and summarises its line kinds.
///
/// # Errors
///
/// Returns [`ConfigInspectionError::Read`] if `path` cannot be read (missing
/// file, permissions), or [`ConfigInspectionError::Parse`] if the bytes are not
/// valid UTF-8. Both variants carry the offending path.
pub fn inspect_config_path(
    path: impl AsRef<Path>,
) -> Result<ConfigFileSummary, ConfigInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| ConfigInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_config_file(path, &bytes).map_err(|source| ConfigInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and summarises every config file in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_config_path`] returns for the first path that
/// fails — [`ConfigInspectionError::Read`] for an unreadable file, or
/// [`ConfigInspectionError::Parse`] for one that is not valid UTF-8.
pub fn inspect_config_files<I, P>(paths: I) -> Result<ConfigInspection, ConfigInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = ConfigInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_config_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_config_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(CONFIG_EXTENSION)
}

#[must_use]
pub fn is_config_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_config_extension)
}

#[must_use]
pub fn is_config_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_config_extension)
}

fn parse_line(line: &str) -> ConfigLine<'_> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ConfigLine::Blank;
    }

    if let Some(text) = trimmed.strip_prefix(';') {
        return ConfigLine::Comment(ConfigComment {
            marker: CommentMarker::Semicolon,
            text: text.trim_start(),
        });
    }

    if let Some(text) = trimmed.strip_prefix("--") {
        return ConfigLine::Comment(ConfigComment {
            marker: CommentMarker::DoubleDash,
            text: text.trim_start(),
        });
    }

    if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return ConfigLine::Section(ConfigSection { name: name.trim() });
    }

    if let Some((key, value)) = trimmed.split_once('=') {
        return ConfigLine::Assignment(ConfigAssignment {
            key: key.trim(),
            value: value.trim(),
        });
    }

    let (name, args) = trimmed
        .split_once(char::is_whitespace)
        .map_or((trimmed, ""), |(name, args)| (name, args.trim_start()));
    ConfigLine::Command(ConfigCommand { name, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assignments_comments_sections_and_commands() {
        let input = r#"; comment
[default]
= 7
sys_game_name = "Example Game"
bind o "r_measureOverdraw 0"
-- dash comment
"#;
        let config = ConfigFile::parse_str(input);

        assert_eq!(
            config.lines(),
            &[
                ConfigLine::Comment(ConfigComment {
                    marker: CommentMarker::Semicolon,
                    text: "comment"
                }),
                ConfigLine::Section(ConfigSection { name: "default" }),
                ConfigLine::Assignment(ConfigAssignment {
                    key: "",
                    value: "7"
                }),
                ConfigLine::Assignment(ConfigAssignment {
                    key: "sys_game_name",
                    value: "\"Example Game\""
                }),
                ConfigLine::Command(ConfigCommand {
                    name: "bind",
                    args: "o \"r_measureOverdraw 0\""
                }),
                ConfigLine::Comment(ConfigComment {
                    marker: CommentMarker::DoubleDash,
                    text: "dash comment"
                }),
            ]
        );

        let summary = config.summary();
        let mut totals = ConfigTotals::default();
        totals.add_summary(summary);

        assert_eq!(
            summary,
            ConfigSummary {
                lines: 6,
                assignments: 2,
                sections: 1,
                commands: 1,
                comments: 2,
                blanks: 0
            }
        );
        assert_eq!(totals.files, 1);
        assert_eq!(totals.lines, 6);
        assert_eq!(
            summary.to_string(),
            "2 assignments, 1 sections, 1 commands, 2 comments"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  lines: 6\n  assignments: 2\n  sections: 1\n  commands: 1\n  comments: 2\n  blanks: 0\n"
        );

        let mut inspection = ConfigInspection::default();
        inspection.add_file_summary(
            inspect_config_file("config/system.cfg", input.as_bytes()).expect("inspect config"),
        );
        assert_eq!(
            inspection.report(20).to_string(),
            "config/system.cfg: 2 assignments, 1 sections, 1 commands, 2 comments\n  files: 1\n  lines: 6\n  assignments: 2\n  sections: 1\n  commands: 1\n  comments: 2\n  blanks: 0\n"
        );

        assert!(is_config_name("system.CFG"));
        assert!(is_config_path(Path::new("game.cfg")));
        assert!(!is_config_name("game.xml"));
    }

    #[test]
    fn parses_hash_prefixed_audio_commands_as_commands() {
        let config = ConfigFile::parse_str("#Sound.DeactivateAudioDevice()");

        assert_eq!(
            config.commands().next(),
            Some(ConfigCommand {
                name: "#Sound.DeactivateAudioDevice()",
                args: ""
            })
        );
    }

    #[test]
    fn inspect_config_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "az-rs-cry-system-{}-system.cfg",
            std::process::id()
        ));
        std::fs::write(&path, b"[default]\nsys_game_name = Example Game\n").expect("write config");

        let inspection = inspect_config_files([&path]).expect("inspect config files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.lines, 2);
        assert_eq!(inspection.totals.assignments, 1);
        assert_eq!(inspection.totals.sections, 1);

        std::fs::remove_file(path).expect("remove config");
    }
}
