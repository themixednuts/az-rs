use crate::error::CliResult;
use az_project::ProjectTopologyKind;
use std::io::Write;
use std::path::PathBuf;

pub fn set(topology: ProjectTopologyKind, path: Option<PathBuf>) -> CliResult<()> {
    let project_root = path.unwrap_or_else(|| PathBuf::from("."));
    let change = az_project_scaffold::topology::set(&project_root, topology)?;
    az_project_scaffold::project_contract::sync_project_contract(&project_root)?;
    println!("Project topology set to {}.", change.topology);
    print_packages("Added role packages", &change.added_packages);
    print_packages(
        "Inactive-for-topology packages retained",
        &change.inactive_packages,
    );
    Ok(())
}

pub fn prune(path: Option<PathBuf>, force: bool) -> CliResult<()> {
    let project_root = path.unwrap_or_else(|| PathBuf::from("."));
    let plan = az_project_scaffold::topology::plan_prune(&project_root)?;
    if plan.is_empty() {
        println!(
            "No inactive role packages to prune for topology {}.",
            plan.topology
        );
        return Ok(());
    }

    print_packages(
        "Inactive role packages selected for deletion",
        &plan.packages,
    );
    if !force && !confirm_prune()? {
        println!("Topology prune cancelled; no packages were removed.");
        return Ok(());
    }

    let outcome = az_project_scaffold::topology::prune(&project_root, true)?;
    print_packages("Pruned role packages", &outcome.packages);
    Ok(())
}

fn confirm_prune() -> CliResult<bool> {
    print!("Delete these unmodified, unreferenced packages? [y/N] ");
    std::io::stdout().flush()?;
    let mut response = String::new();
    std::io::stdin().read_line(&mut response)?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn print_packages(label: &str, packages: &[String]) {
    if packages.is_empty() {
        return;
    }
    println!("{label}:");
    for package in packages {
        println!("  {package}");
    }
}
