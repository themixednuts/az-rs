//! Typed editor presentation metadata carried by Bevy reflection.
//!
//! Static presentation is stored as custom reflection attributes on types,
//! fields, and variants. The same [`EditorTypeAttributes`] value may also be
//! inserted as Bevy `TypeData` when metadata is registered after a foreign type.

use std::collections::BTreeMap;

use bevy_reflect::{GetTypeRegistration, Reflect, TypePath, TypeRegistry};

/// Static editor metadata attached to a reflected type or enum variant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct EditorTypeAttributes {
    pub label: Option<String>,
    pub description: Option<String>,
    pub group: Option<String>,
    pub icon: Option<String>,
    pub hidden: bool,
    pub read_only: bool,
    /// Opaque action identities understood only by project-host callbacks.
    pub action_ids: Vec<String>,
}

impl EditorTypeAttributes {
    #[must_use]
    pub fn labeled(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn with_action(mut self, action_id: impl Into<String>) -> Self {
        self.action_ids.push(action_id.into());
        self
    }
}

/// Numeric presentation constraints kept as source-stable strings.
///
/// Projecting text instead of `f64` preserves integer ranges and exact decimal
/// spellings across process boundaries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct EditorNumericRange {
    pub minimum: Option<String>,
    pub maximum: Option<String>,
    pub step: Option<String>,
    pub suffix: Option<String>,
}

/// Non-numeric validation constraints used by reflected editor fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct EditorFieldConstraints {
    pub minimum_length: Option<u32>,
    pub maximum_length: Option<u32>,
    pub allowed_strings: Vec<String>,
    pub allowed_variants: Vec<String>,
}

/// Renderer selection for a reflected field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub enum EditorWidget {
    #[default]
    Default,
    Number,
    Slider,
    Checkbox,
    Toggle,
    Dropdown {
        choices: Vec<String>,
    },
    AssetPicker {
        asset_type_path: String,
    },
    ObjectPicker {
        object_type_path: String,
    },
    Multiline {
        rows: Option<u32>,
    },
    Color,
    Vector {
        dimensions: u8,
    },
}

impl EditorWidget {
    /// Stable transport spelling used by the vNext registry projection.
    #[must_use]
    pub fn projection_name(&self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Number => "number".to_owned(),
            Self::Slider => "slider".to_owned(),
            Self::Checkbox => "checkbox".to_owned(),
            Self::Toggle => "toggle".to_owned(),
            Self::Dropdown { choices } => format!("dropdown:{}", choices.join("|")),
            Self::AssetPicker { asset_type_path } => format!("asset:{asset_type_path}"),
            Self::ObjectPicker { object_type_path } => format!("object:{object_type_path}"),
            Self::Multiline { rows } => {
                format!(
                    "multiline:{}",
                    rows.map_or_else(String::new, |rows| rows.to_string())
                )
            }
            Self::Color => "color".to_owned(),
            Self::Vector { dimensions } => format!("vector:{dimensions}"),
        }
    }
}

/// Static editor metadata attached to a reflected field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct EditorFieldAttributes {
    pub label: Option<String>,
    pub description: Option<String>,
    pub group: Option<String>,
    pub icon: Option<String>,
    pub widget: EditorWidget,
    pub range: Option<EditorNumericRange>,
    pub constraints: EditorFieldConstraints,
    pub hidden: bool,
    pub read_only: bool,
}

impl EditorFieldAttributes {
    #[must_use]
    pub fn new(label: impl Into<String>, widget: EditorWidget) -> Self {
        Self {
            label: Some(label.into()),
            widget,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_range(mut self, range: EditorNumericRange) -> Self {
        self.range = Some(range);
        self
    }

    #[must_use]
    pub fn with_constraints(mut self, constraints: EditorFieldConstraints) -> Self {
        self.constraints = constraints;
        self
    }
}

/// Registers primitive and common container metadata used by editor widgets.
///
/// This is intentionally a set of normal Bevy registrations. It is not an
/// inventory and callers may freely add project-specific closed generic types.
pub fn register_editor_builtins(registry: &mut TypeRegistry) {
    register_builtin::<bool>(registry, "Boolean", "bool");
    register_builtin::<i8>(registry, "8-bit Signed Integer", "number");
    register_builtin::<i16>(registry, "16-bit Signed Integer", "number");
    register_builtin::<i32>(registry, "32-bit Signed Integer", "number");
    register_builtin::<i64>(registry, "64-bit Signed Integer", "number");
    register_builtin::<i128>(registry, "128-bit Signed Integer", "number");
    register_builtin::<isize>(registry, "Signed Integer", "number");
    register_builtin::<u8>(registry, "8-bit Unsigned Integer", "number");
    register_builtin::<u16>(registry, "16-bit Unsigned Integer", "number");
    register_builtin::<u32>(registry, "32-bit Unsigned Integer", "number");
    register_builtin::<u64>(registry, "64-bit Unsigned Integer", "number");
    register_builtin::<u128>(registry, "128-bit Unsigned Integer", "number");
    register_builtin::<usize>(registry, "Unsigned Integer", "number");
    register_builtin::<f32>(registry, "32-bit Float", "number");
    register_builtin::<f64>(registry, "64-bit Float", "number");
    register_builtin::<String>(registry, "String", "text");

    register_builtin::<Vec<String>>(registry, "String List", "list");
    register_builtin::<Vec<f32>>(registry, "Float List", "list");
    register_builtin::<BTreeMap<String, String>>(registry, "String Map", "map");
    register_builtin::<BTreeMap<String, f32>>(registry, "Float Map", "map");
    register_builtin::<Option<String>>(registry, "Optional String", "optional");
    register_builtin::<Option<f32>>(registry, "Optional Float", "optional");

    register_builtin::<glam::Vec2>(registry, "2D Vector", "vector:2");
    register_builtin::<glam::Vec3>(registry, "3D Vector", "vector:3");
    register_builtin::<glam::Vec4>(registry, "4D Vector", "vector:4");
    register_builtin::<glam::Quat>(registry, "Quaternion", "vector:4");
}

fn register_builtin<T>(registry: &mut TypeRegistry, label: &str, widget: &str)
where
    T: GetTypeRegistration + TypePath,
{
    registry.register::<T>();
    let registration = registry
        .get_mut(std::any::TypeId::of::<T>())
        .expect("a just-registered reflected type must be present");
    registration.insert(EditorTypeAttributes {
        label: Some(label.to_owned()),
        group: Some("Built-in".to_owned()),
        description: Some(format!("Editor renderer: {widget}")),
        ..EditorTypeAttributes::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Reflect)]
    #[reflect(@EditorTypeAttributes::labeled("Fixture").in_group("Tests"))]
    struct Fixture {
        #[reflect(@EditorFieldAttributes::new(
            "Amount",
            EditorWidget::Slider,
        ).with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: Some("1".to_owned()),
            step: Some("0.1".to_owned()),
            suffix: Some("%".to_owned()),
        }))]
        amount: f32,
        #[reflect(@EditorFieldAttributes::new(
            "Choice",
            EditorWidget::Dropdown {
                choices: vec!["First".to_owned(), "Second".to_owned()],
            },
        ).with_constraints(EditorFieldConstraints {
            minimum_length: Some(1),
            maximum_length: Some(16),
            allowed_strings: vec!["First".to_owned(), "Second".to_owned()],
            allowed_variants: vec!["First".to_owned()],
        }))]
        choice: String,
    }

    #[test]
    fn bevy_custom_attributes_keep_typed_editor_metadata() {
        let bevy_reflect::TypeInfo::Struct(info) = <Fixture as bevy_reflect::Typed>::type_info()
        else {
            panic!("fixture should reflect as a struct");
        };
        assert_eq!(
            info.get_attribute::<EditorTypeAttributes>()
                .and_then(|attributes| attributes.label.as_deref()),
            Some("Fixture")
        );
        let field = info.field("amount").expect("amount field");
        let attributes = field
            .get_attribute::<EditorFieldAttributes>()
            .expect("field attributes");
        assert_eq!(attributes.widget, EditorWidget::Slider);
        assert_eq!(
            attributes
                .range
                .as_ref()
                .and_then(|range| range.step.as_deref()),
            Some("0.1")
        );
        let constraints = &info
            .field("choice")
            .expect("choice field")
            .get_attribute::<EditorFieldAttributes>()
            .expect("choice attributes")
            .constraints;
        assert_eq!(constraints.minimum_length, Some(1));
        assert_eq!(constraints.maximum_length, Some(16));
        assert_eq!(constraints.allowed_strings, ["First", "Second"]);
        assert_eq!(constraints.allowed_variants, ["First"]);
    }

    #[test]
    fn builtins_are_normal_bevy_registrations_with_editor_type_data() {
        let mut registry = TypeRegistry::default();
        register_editor_builtins(&mut registry);

        let registration = registry
            .get(std::any::TypeId::of::<glam::Vec3>())
            .expect("Vec3 registration");
        assert_eq!(
            registration
                .data::<EditorTypeAttributes>()
                .and_then(|attributes| attributes.label.as_deref()),
            Some("3D Vector")
        );
    }
}
