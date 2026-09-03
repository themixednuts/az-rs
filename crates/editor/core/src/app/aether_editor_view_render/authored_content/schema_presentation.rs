//! Shared authored-schema visuals for asset and document projections.

/// User-facing label for an authored schema type.
pub(crate) fn schema_display_label(schema_type: &str) -> String {
    az_editor_ui::naming::schema_display_name(schema_type, None).into_owned()
}

/// Stable icon for an authored schema type.
pub(crate) fn schema_icon(schema_type: &str) -> &'static str {
    let schema = schema_type.to_ascii_lowercase();
    if schema.contains("prefab") {
        "widgets"
    } else if schema.contains("material") {
        "gradient"
    } else if schema.contains("mesh") {
        "deployed_code"
    } else if schema.contains("level") || schema.contains("scene") {
        "map"
    } else {
        "data_object"
    }
}

/// Stable color token encoded for the Aether item projection.
pub(crate) fn schema_color(schema_type: &str) -> &'static str {
    let schema = schema_type.to_ascii_lowercase();
    if schema.contains("prefab") {
        "#b78fd6"
    } else if schema.contains("material") {
        "#d6a23b"
    } else if schema.contains("mesh") {
        "#7a8aa0"
    } else if schema.contains("level") || schema.contains("scene") {
        "#e0a060"
    } else {
        "#4188e0"
    }
}
