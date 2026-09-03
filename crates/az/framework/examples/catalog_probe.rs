//! Read-only probe: answer what the published catalog holds for one asset id
//! and for the paths the world runtime derives from it.

use az_asset::AssetId;
use az_framework::asset::AssetCatalog;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: catalog_probe <asset_root> <guid>:<sub_id_hex> [paths...]");
    let id_arg = std::env::args().nth(2).expect("asset id");
    let catalog = AssetCatalog::open_native(&root).expect("open native catalog");

    let (guid, sub) = id_arg.split_once(':').expect("guid:sub_id");
    let guid: uuid::Uuid = guid.trim_matches(['{', '}']).parse().expect("uuid");
    let sub_id = u32::from_str_radix(sub, 16).expect("hex sub id");
    let asset = AssetId::new(guid, sub_id);

    println!("== get_by_id({guid}:{sub_id:x}) ==");
    match catalog.get_by_id(asset) {
        None => println!("  MISS"),
        Some(entry) => {
            println!("  relative_path = {}", entry.relative_path().display());
            println!("  source_path   = {}", entry.source_path().display());
            let aliases: Vec<_> = entry
                .catalog_aliases()
                .map(|p| p.display().to_string())
                .collect();
            println!("  aliases({})   = {aliases:?}", aliases.len());
        }
    }

    for path in std::env::args().skip(3) {
        match catalog.get_by_path(&path) {
            None => println!("== get_by_path({path}) == MISS"),
            Some(entry) => println!(
                "== get_by_path({path}) == HIT id={:?} relative={}",
                catalog.id_for_path(&path),
                entry.relative_path().display()
            ),
        }
    }
}
