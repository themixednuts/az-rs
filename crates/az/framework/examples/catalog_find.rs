//! Read-only: print the published identity of entries under a path prefix.
use az_framework::asset::AssetCatalog;

fn main() {
    let root = std::env::args().nth(1).expect("root");
    let prefix = std::env::args().nth(2).expect("prefix");
    let needle = std::env::args().nth(3).unwrap_or_default();
    let catalog = AssetCatalog::open_native(&root).expect("open native catalog");
    for entry in catalog.entries_with_path_prefix(&prefix) {
        let path = entry.relative_path().to_string_lossy().into_owned();
        if !needle.is_empty() && !path.contains(&needle) {
            continue;
        }
        let id = entry.asset_id();
        println!(
            "{:?}:{:x}  registered_by_path={}  {}",
            id.guid,
            id.sub_id,
            catalog.get_by_path(&path).is_some(),
            path
        );
    }
}
