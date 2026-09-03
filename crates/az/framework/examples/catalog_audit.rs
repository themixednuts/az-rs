//! Read-only audit: how many published catalog entries are reachable by the
//! path they publish, broken down by product extension.

use std::collections::BTreeMap;

use az_framework::asset::AssetCatalog;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: catalog_audit <asset_root> [prefix]");
    let prefix = std::env::args().nth(2).unwrap_or_default();
    let catalog = AssetCatalog::open_native(&root).expect("open native catalog");

    let entries = catalog.entries_with_path_prefix(&prefix);
    let mut hit: BTreeMap<String, usize> = BTreeMap::new();
    let mut miss: BTreeMap<String, usize> = BTreeMap::new();
    let mut miss_examples: BTreeMap<String, String> = BTreeMap::new();

    for entry in &entries {
        let path = entry.relative_path().to_string_lossy().into_owned();
        let kind = path.rsplit('/').next().unwrap_or(&path);
        let kind = match kind.split_once('.') {
            Some((_, ext)) => ext.to_owned(),
            None => "<none>".to_owned(),
        };
        let reachable = catalog
            .get_by_path(&path)
            .is_some_and(|found| found.asset_id() == entry.asset_id());
        if reachable {
            *hit.entry(kind).or_default() += 1;
        } else {
            *miss.entry(kind.clone()).or_default() += 1;
            miss_examples.entry(kind).or_insert(path);
        }
    }

    let total_hit: usize = hit.values().sum();
    let total_miss: usize = miss.values().sum();
    println!(
        "entries={} reachable_by_own_path={total_hit} unreachable={total_miss}",
        entries.len()
    );
    println!("\n== unreachable by extension (top 25) ==");
    let mut rows: Vec<_> = miss.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (kind, n) in rows.iter().take(25) {
        println!(
            "  {n:>8}  .{kind}   e.g. {}",
            miss_examples.get(kind).map_or("", String::as_str)
        );
    }
    println!("\n== reachable by extension (top 15) ==");
    let mut rows: Vec<_> = hit.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (kind, n) in rows.iter().take(15) {
        println!("  {n:>8}  .{kind}");
    }
}
