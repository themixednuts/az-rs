use std::{error::Error, fs, path::Path};

use az_architecture_guard::{
    ADR_0022_DELETION_INVENTORY_FILE, DeletionTargetInventory, encode_deletion_target_inventory,
    scan_adr_0022_deletion_targets,
};

fn main() -> Result<(), Box<dyn Error>> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("architecture guard lives under crates/az");
    let inventory = DeletionTargetInventory::new(scan_adr_0022_deletion_targets(workspace_root)?);
    let output = workspace_root.join(ADR_0022_DELETION_INVENTORY_FILE);
    fs::write(&output, encode_deletion_target_inventory(&inventory)?)?;
    println!(
        "wrote {} entries to {}",
        inventory.entries.len(),
        output.display()
    );
    Ok(())
}
