//! Human-facing authored document names.

use std::borrow::Cow;

use az_proto_project::vnext::{ReflectedFieldDescriptor, ReflectedTypeDescriptor};

const TYPE_SUFFIXES: &[&str] = &[
    ".prefab",
    ".scene",
    ".level",
    ".material",
    ".mat",
    ".azmaterial",
    ".azmaterialtype",
    ".azmat",
];

/// Return the filename stem used by editor chrome.
///
/// Authoring chrome, including the asset browser, conveys type separately via
/// catalog-backed icons/badges, so both serialization (`.ron`) and known
/// compound type suffixes are removed (`e2e_box.prefab.ron` -> `e2e_box`).
#[must_use]
pub fn display_name(value: &str) -> Cow<'_, str> {
    let file_name = value.rsplit(['/', '\\']).next().unwrap_or(value);
    let without_ron = file_name.strip_suffix(".ron").unwrap_or(file_name);
    let stem = TYPE_SUFFIXES
        .iter()
        .find_map(|suffix| without_ron.strip_suffix(suffix))
        .unwrap_or(without_ron);
    Cow::Borrowed(stem)
}

/// Return a document label without exposing generated UUID-backed file names.
///
/// Authored documents normally have a user-provided root-object name. During
/// creation/loading that name can be absent, while the backing path is a
/// generated value such as `prefab-019f...`. In that case the schema label is
/// the only useful primary label; the exact path remains available to debug
/// and tooltip surfaces.
#[must_use]
pub fn document_display_name<'a>(value: &'a str, schema_label: &'a str) -> Cow<'a, str> {
    let name = display_name(value);
    if has_generated_uuid_suffix(name.as_ref()) {
        Cow::Borrowed(schema_label)
    } else {
        name
    }
}

fn has_generated_uuid_suffix(value: &str) -> bool {
    let Some(uuid) = value.get(value.len().saturating_sub(36)..) else {
        return false;
    };
    if value.len() > 36 && value.as_bytes().get(value.len() - 37) != Some(&b'-') {
        return false;
    }
    uuid.bytes().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

/// Return the human-facing name for a stable reflected type path.
///
/// Explicit reflection metadata wins. Reflected types without metadata fall back
/// to the final id segment with word boundaries derived from snake/kebab/camel
/// case (`azoth.render.MeshPlaceholder` -> `Mesh Placeholder`).
#[must_use]
pub fn schema_display_name<'a>(
    schema_id: &'a str,
    explicit_label: Option<&'a str>,
) -> Cow<'a, str> {
    if let Some(label) = explicit_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        return Cow::Borrowed(label);
    }

    let short = schema_id
        .rsplit(['.', ':', '/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(schema_id);
    Cow::Owned(split_schema_words(short))
}

/// Resolve a reflected type label from its editor attributes and short path.
#[must_use]
pub fn reflected_type_display_name(descriptor: &ReflectedTypeDescriptor) -> Cow<'_, str> {
    descriptor
        .editor_attributes
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(
            || Cow::Borrowed(descriptor.short_path.as_str()),
            Cow::Borrowed,
        )
}

/// Resolve a reflected field label from its editor attributes and stable name.
#[must_use]
pub fn reflected_field_display_name(descriptor: &ReflectedFieldDescriptor) -> Cow<'_, str> {
    schema_display_name(
        &descriptor.name,
        descriptor.editor_attributes.label.as_deref(),
    )
}

fn split_schema_words(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len() + 4);
    let mut word_start = true;

    for (index, ch) in chars.iter().copied().enumerate() {
        if matches!(ch, '_' | '-' | ' ') {
            if !output.ends_with(' ') && !output.is_empty() {
                output.push(' ');
            }
            word_start = true;
            continue;
        }

        let previous = index
            .checked_sub(1)
            .and_then(|index| chars.get(index))
            .copied();
        let next = chars.get(index + 1).copied();
        let camel_boundary = ch.is_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_uppercase() && next.is_some_and(char::is_lowercase))
            });
        if camel_boundary && !output.ends_with(' ') {
            output.push(' ');
            word_start = true;
        }

        if word_start {
            output.extend(ch.to_uppercase());
            word_start = false;
        } else {
            output.push(ch);
        }
    }

    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_serialization_and_compound_type_suffixes() {
        assert_eq!(display_name("prefabs/e2e_box.prefab.ron"), "e2e_box");
        assert_eq!(display_name("levels/main.scene.ron"), "main");
        assert_eq!(display_name("settings.ron"), "settings");
    }

    #[test]
    fn keeps_unrelated_asset_extensions() {
        assert_eq!(display_name("textures/e2e_test.png"), "e2e_test.png");
    }

    #[test]
    fn generated_document_ids_fall_back_to_the_schema_label() {
        assert_eq!(
            document_display_name(
                "prefabs/prefab-019f4db7-e43d-7333-8c12-f571e1e928a1.prefab.ron",
                "Prefab"
            ),
            "Prefab"
        );
        assert_eq!(
            document_display_name("prefabs/e2e_box.prefab.ron", "Prefab"),
            "e2e_box"
        );
    }

    #[test]
    fn schema_names_prefer_metadata_and_derive_readable_fallbacks() {
        assert_eq!(
            schema_display_name("azoth.transform.Transform", Some("Local Transform")),
            "Local Transform"
        );
        assert_eq!(
            schema_display_name("azoth.transform.Transform", None),
            "Transform"
        );
        assert_eq!(
            schema_display_name("azoth.render.MeshPlaceholder", None),
            "Mesh Placeholder"
        );
        assert_eq!(
            schema_display_name("project.damage_zone", None),
            "Damage Zone"
        );
    }

    #[test]
    fn reflected_names_prefer_editor_labels_and_use_native_fallbacks() {
        let mut ty = ReflectedTypeDescriptor {
            type_path: "azoth::transform::Transform".to_owned(),
            short_path: "Transform".to_owned(),
            kind: az_proto_project::vnext::ReflectedTypeKind::Struct,
            fields: Vec::new(),
            variants: Vec::new(),
            editor_attributes: az_proto_project::vnext::EditorAttributes::default(),
            type_data_flags: Vec::new(),
            applicability: az_proto_project::vnext::ApplicabilityDescriptor::default(),
            reflected_default: None,
        };
        assert_eq!(reflected_type_display_name(&ty), "Transform");
        ty.editor_attributes.label = Some("Local Transform".to_owned());
        assert_eq!(reflected_type_display_name(&ty), "Local Transform");

        let mut field = ReflectedFieldDescriptor {
            name: "angular_velocity".to_owned(),
            type_path: "glam::Vec3".to_owned(),
            editor_attributes: az_proto_project::vnext::EditorAttributes::default(),
        };
        assert_eq!(reflected_field_display_name(&field), "Angular Velocity");
        field.editor_attributes.label = Some("Spin".to_owned());
        assert_eq!(reflected_field_display_name(&field), "Spin");
    }
}
