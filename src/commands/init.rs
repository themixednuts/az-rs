use crate::error::CliResult;
use std::path::PathBuf;

pub fn execute(
    path: Option<PathBuf>,
    name: Option<String>,
    lore_url: Option<String>,
) -> CliResult<()> {
    az_project_scaffold::init::execute(path, name, lore_url)?;
    Ok(())
}
