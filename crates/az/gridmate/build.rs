use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // Generate debug-only native TypeRegistry validation data. Runtime lookup
    // comes from the host's composed ClassRegistration entries, not this file.
    emit_type_registry_debug_data();
}

// ---------------------------------------------------------------------------
// AZ TypeRegistry debug validation data generation
// ---------------------------------------------------------------------------
//
// Reads an optional project type-registry snapshot and emits
// `$OUT_DIR/type_registry_debug_data.rs`, which is included only from
// `src/az/type_registry.rs` under `cfg(debug_assertions)`.
//
// The JSON schema is a top-level object whose `data.typesByUuid` is an array of
// pairs `[uuid_string, entry]` where each entry contains:
//   - `name`: human-readable class name (may be "")
//   - `uuid`: same as the key (kept for redundancy)
//   - `index`: native class-table position (dense 0..N-1). State-fragment
//              type info uses this registry index.
//   - `typeIndex`: compact type id (sparse). IMessage envelopes and other
//                  explicitly type-indexed values use this separate identity.
//
// What we emit for debug validation:
//   * `STATIC_ENTRIES: &[StaticEntry]` — sorted/densified by native `index`.
//     Used only by debug validation helpers.
//   * `BY_UUID: phf::Map<[u8; 16], &StaticEntry>` — perfect-hash UUID lookup.
//     Used to compare Rust descriptor constants with native snapshot rows.
//   * `BY_TYPE_INDEX: phf::Map<u32, [u8; 16]>` — compact type id lookup.
//   * `UUID_INDEX_SORTED: &[(u128, u32, u32)]` — `(uuid, class_index, type_index)`,
//     sorted by `uuid`. Backs `const fn` lookups (which `phf::Map` cannot do
//     in stable Rust as of 0.13).

fn emit_type_registry_debug_data() {
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set in build.rs"),
    );
    let workspace_root = workspace_root(&manifest_dir).expect("workspace root not found");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set in build.rs"));
    let out_path = out_dir.join("type_registry_debug_data.rs");

    println!("cargo:rerun-if-env-changed=AZOTH_TYPEREGISTRY");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("DEBUG").map_or(true, |value| value != "true") {
        write_empty(&out_path);
        return;
    }

    let Some(typeregistry_path) = typeregistry_path(workspace_root) else {
        eprintln!(
            "[gridmate build] no type registry selected; emitting empty debug TypeRegistry table"
        );
        write_empty(&out_path);
        return;
    };

    // If the snapshot is missing, emit an empty table. Crates depending on
    // gridmate still build; only the polymorphic-decode path loses its index
    // table (it falls back to "raw UUID required" on the wire, which is the
    // protocol-legal fallback anyway). Do not watch a missing path: Cargo
    // treats a nonexistent rerun-if-changed target as always dirty, forcing a
    // rebuild of this crate on every invocation. Snapshot selection changes
    // are still picked up through AZOTH_TYPEREGISTRY (rerun-if-env-changed).
    if !typeregistry_path.is_file() {
        eprintln!(
            "[gridmate build] type-registry snapshot not found at {}; emitting empty debug TypeRegistry table",
            typeregistry_path.display()
        );
        write_empty(&out_path);
        return;
    }

    println!("cargo:rerun-if-changed={}", typeregistry_path.display());

    let entries = load_type_entries(&typeregistry_path);
    let output = render_type_registry_debug_data(&entries);
    fs::write(&out_path, output).expect("write type_registry_debug_data.rs");
    eprintln!(
        "[gridmate build] emitted {} class table entries to {}",
        entries.len(),
        out_path.display()
    );
}
fn load_type_entries(typeregistry_path: &Path) -> Vec<TypeEntry> {
    let raw = fs::read_to_string(typeregistry_path).expect("read type-registry snapshot");
    let json: serde_json::Value = serde_json::from_str(&raw).expect("parse type-registry snapshot");
    let rows = json
        .pointer("/data/typesByUuid")
        .and_then(serde_json::Value::as_array)
        .expect("type-registry snapshot: missing /data/typesByUuid array");
    let mut entries = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| parse_type_entry(row_index, row))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("[gridmate build] invalid type-registry snapshot: {error}"));
    entries.sort_by_key(|e| e.class_index);
    validate_dense_class_indices(&entries);
    entries
}

fn parse_type_entry(row_index: usize, row: &serde_json::Value) -> Result<TypeEntry, String> {
    let pair = row
        .as_array()
        .ok_or_else(|| format!("row {row_index}: expected a two-element `[uuid, entry]` array"))?;
    if pair.len() != 2 {
        return Err(format!(
            "row {row_index}: expected two elements, found {}",
            pair.len()
        ));
    }
    let row_uuid = pair[0]
        .as_str()
        .ok_or_else(|| format!("row {row_index}: UUID key must be a string"))?;
    let entry = pair[1]
        .as_object()
        .ok_or_else(|| format!("row {row_index} ({row_uuid}): entry must be an object"))?;
    let uuid = entry
        .get("uuid")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("row {row_index} ({row_uuid}): `uuid` must be a string"))?;
    if row_uuid != uuid {
        return Err(format!(
            "row {row_index}: UUID key `{row_uuid}` disagrees with entry UUID `{uuid}`"
        ));
    }
    let name = entry
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("row {row_index} ({uuid}): `name` must be a string"))?;
    let class_index = required_u32(entry, "index", row_index, uuid)?;
    let type_index = required_u32(entry, "typeIndex", row_index, uuid)?;
    let uuid_bytes =
        parse_uuid(uuid).ok_or_else(|| format!("row {row_index}: `{uuid}` is not a valid UUID"))?;
    Ok(TypeEntry {
        uuid_bytes,
        name: name.to_owned(),
        class_index,
        type_index,
    })
}

fn required_u32(
    entry: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    row_index: usize,
    uuid: &str,
) -> Result<u32, String> {
    let value = entry
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            format!("row {row_index} ({uuid}): `{field}` must be an unsigned integer")
        })?;
    u32::try_from(value)
        .map_err(|_| format!("row {row_index} ({uuid}): `{field}` value {value} exceeds u32"))
}

fn validate_dense_class_indices(entries: &[TypeEntry]) {
    // Sanity check: class_index must be dense 0..N. If it isn't, the binary
    // was dumped with a non-contiguous table; fail loud rather than silently
    // mis-decode polymorphic values at runtime.
    for (slot, entry) in entries.iter().enumerate() {
        let class_index = usize::try_from(entry.class_index)
            .expect("type-registry snapshot: class index exceeds usize");
        let uuid = format_uuid(&entry.uuid_bytes);
        assert!(
            class_index == slot,
            "[gridmate build] non-contiguous class_index at slot {slot}: got {class_index} \
             (uuid={uuid}). The typeregistry snapshot is from a different binary build than \
             expected."
        );
    }
}

fn render_type_registry_debug_data(entries: &[TypeEntry]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated by build.rs from the selected type-registry snapshot - do not edit.\n",
    );
    out.push_str("//\n");
    out.push_str("// Each entry is one row in the runtime class table. The slice is sorted\n");
    out.push_str("// and densified by `class_index`, so `STATIC_ENTRIES[i].class_index == i`.\n");
    out.push('\n');
    write_static_entries(&mut out, entries);
    write_uuid_map(&mut out, entries);
    write_type_index_map(&mut out, entries);
    write_uuid_index_sorted(&mut out, entries);
    out
}

fn write_static_entries(out: &mut String, entries: &[TypeEntry]) {
    out.push_str("pub(super) const STATIC_ENTRIES: &[StaticEntry] = &[\n");
    for entry in entries {
        let uuid = format_uuid_array(&entry.uuid_bytes);
        let name = &entry.name;
        let class_index = entry.class_index;
        let type_index = entry.type_index;
        writeln!(
            out,
            "    StaticEntry {{ uuid: {uuid}, name: {name:?}, class_index: {class_index}, type_index: {type_index} }},"
        )
        .expect("writing to a String cannot fail");
    }
    out.push_str("];\n\n");
}

fn write_uuid_map(out: &mut String, entries: &[TypeEntry]) {
    // ------------------------------------------------------------------
    // phf::Map<[u8; 16], usize>  — UUID -> STATIC_ENTRIES index.
    // We store the index rather than a `&'static StaticEntry` because phf
    // cannot embed references into its generated tables on stable Rust.
    // ------------------------------------------------------------------
    out.push_str("/// Perfect-hash map from UUID bytes to the index in `STATIC_ENTRIES`.\n");
    out.push_str("/// Use `&STATIC_ENTRIES[*idx]` to get the row.\n");
    out.push_str("pub(super) static BY_UUID: ::phf::Map<[u8; 16], usize> = ");
    // `phf_codegen::Map::entry` takes `&dyn Display`, so the formatted
    // strings have to outlive the builder call.
    let uuid_values: Vec<String> = entries
        .iter()
        .map(|entry| {
            let class_index = usize::try_from(entry.class_index)
                .expect("type-registry snapshot: class index exceeds usize");
            format!("{class_index}usize")
        })
        .collect();
    let mut uuid_map = phf_codegen::Map::<[u8; 16]>::new();
    for (entry, value) in entries.iter().zip(uuid_values.iter()) {
        uuid_map.entry(entry.uuid_bytes, value);
    }
    out.push_str(&uuid_map.build().to_string());
    out.push_str(";\n\n");
}

fn write_type_index_map(out: &mut String, entries: &[TypeEntry]) {
    // ------------------------------------------------------------------
    // phf::Map<u32, [u8; 16]>  — type_index -> UUID. Skip type_index == 0
    // because that's the "no compact id" sentinel and would alias many rows.
    // ------------------------------------------------------------------
    out.push_str("/// Perfect-hash map from compact native `type_index` to UUID bytes.\n");
    out.push_str("/// Skips `type_index == 0` because it is the no-compact-id sentinel.\n");
    out.push_str("pub(super) static BY_TYPE_INDEX: ::phf::Map<u32, [u8; 16]> = ");
    let mut seen_type_indices = HashSet::new();
    let mut ti_pairs: Vec<(u32, String)> = Vec::new();
    for entry in entries {
        if entry.type_index == 0 {
            continue;
        }
        let type_index = entry.type_index;
        assert!(
            seen_type_indices.insert(type_index),
            "[gridmate build] type_index {type_index} aliases two UUIDs in the type-registry snapshot"
        );
        ti_pairs.push((entry.type_index, format_uuid_array(&entry.uuid_bytes)));
    }
    let mut ti_map = phf_codegen::Map::<u32>::new();
    for (type_index, uuid) in &ti_pairs {
        ti_map.entry(*type_index, uuid);
    }
    out.push_str(&ti_map.build().to_string());
    out.push_str(";\n\n");
}

fn write_uuid_index_sorted(out: &mut String, entries: &[TypeEntry]) {
    // ------------------------------------------------------------------
    // Sorted-by-UUID slice for debug validation. Runtime registration comes
    // from the composed `ClassRegistration` entries; this generated table must not
    // drive derive output or release protocol behavior.
    // ------------------------------------------------------------------
    let mut by_uuid_sorted: Vec<&TypeEntry> = entries.iter().collect();
    by_uuid_sorted.sort_by_key(|e| u128::from_be_bytes(e.uuid_bytes));

    out.push_str("/// Sorted by `uuid` (big-endian as `u128`) for debug validation.\n");
    out.push_str("pub(super) const UUID_INDEX_SORTED: &[(u128, u32, u32)] = &[\n");
    for entry in &by_uuid_sorted {
        let key = u128::from_be_bytes(entry.uuid_bytes);
        let class_index = entry.class_index;
        let type_index = entry.type_index;
        writeln!(
            out,
            "    (0x{key:032x}u128, {class_index}u32, {type_index}u32),"
        )
        .expect("writing to a String cannot fail");
    }
    out.push_str("];\n");
}

fn typeregistry_path(workspace_root: &Path) -> Option<PathBuf> {
    if let Some(path) = env::var_os("AZOTH_TYPEREGISTRY").map(PathBuf::from) {
        return Some(path);
    }

    env::var_os("CARGO_FEATURE_NATIVE_TYPE_REGISTRY_DEBUG")
        .is_some()
        .then(|| workspace_root.join("resources").join("typeregistry.json"))
}

fn workspace_root(manifest_dir: &Path) -> Option<&Path> {
    manifest_dir.ancestors().find(|candidate| {
        candidate.join("Cargo.toml").is_file() && candidate.join("resources").is_dir()
    })
}

fn write_empty(out_path: &Path) {
    let stub = r"pub(super) const STATIC_ENTRIES: &[StaticEntry] = &[];
pub(super) static BY_UUID: ::phf::Map<[u8; 16], usize> = ::phf::phf_map! {};
pub(super) static BY_TYPE_INDEX: ::phf::Map<u32, [u8; 16]> = ::phf::phf_map! {};
pub(super) const UUID_INDEX_SORTED: &[(u128, u32, u32)] = &[];
";
    fs::write(out_path, stub).expect("write empty type_registry_debug_data.rs");
}

struct TypeEntry {
    uuid_bytes: [u8; 16],
    name: String,
    class_index: u32,
    type_index: u32,
}

fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let cleaned: String = s.chars().filter(|c| *c != '-').collect();
    if cleaned.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        bytes[i] = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn format_uuid_array(bytes: &[u8; 16]) -> String {
    let mut output = String::from("[");
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        write!(output, "0x{byte:02x}").expect("writing to a String cannot fail");
    }
    output.push(']');
    output
}
