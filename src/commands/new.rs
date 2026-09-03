use crate::error::CliResult;
use az_project::ProjectTopologyKind;
use std::path::PathBuf;

pub fn execute(
    name: String,
    path: Option<PathBuf>,
    lore_url: Option<String>,
    topology: ProjectTopologyKind,
) -> CliResult<()> {
    az_project_scaffold::new::execute_with_options(
        name,
        path,
        az_project_scaffold::new::ProjectCreateOptions {
            lore_url,
            enabled_engine_gems: Vec::new(),
            topology,
        },
    )?;
    Ok(())
}
