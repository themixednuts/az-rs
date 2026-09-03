//! Cap'n Proto wire projection for the ADR 0022 vNext neutral vocabulary.

use std::collections::BTreeMap;
#[cfg(test)]
use std::io::Cursor;

use az_proto_authoring::authoring_capnp;
use az_proto_core::Capability;
use capnp::Error;
#[cfg(test)]
use capnp::{message, serialize_packed};

use crate::{capnp_bounded_index, capnp_list_index, project_capnp, vnext};

#[cfg(test)]
fn round_trip_type_registry_snapshot(
    snapshot: &vnext::TypeRegistrySnapshot,
) -> Result<vnext::TypeRegistrySnapshot, Error> {
    let mut message = message::Builder::new_default();
    snapshot.to_capnp(message.init_root::<project_capnp::type_registry_snapshot::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::TypeRegistrySnapshot::from_capnp(
        reader.get_root::<project_capnp::type_registry_snapshot::Reader<'_>>()?,
    )
}

fn write_type_registry_snapshot(
    snapshot: &vnext::TypeRegistrySnapshot,
    mut builder: project_capnp::type_registry_snapshot::Builder<'_>,
) -> Result<(), Error> {
    builder.set_schema_catalog_hash(&snapshot.schema_catalog_hash);
    let mut types = builder
        .reborrow()
        .init_types(capnp_list_index(snapshot.types.len())?);
    for (index, descriptor) in snapshot.types.iter().enumerate() {
        (descriptor).to_capnp(types.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_type_registry_snapshot(
    reader: project_capnp::type_registry_snapshot::Reader<'_>,
) -> Result<vnext::TypeRegistrySnapshot, Error> {
    Ok(vnext::TypeRegistrySnapshot {
        schema_catalog_hash: reader.get_schema_catalog_hash()?.to_vec(),
        types: reader
            .get_types()?
            .iter()
            .map(read_type_descriptor)
            .collect::<Result<_, _>>()?,
    })
}

fn write_type_descriptor(
    descriptor: &vnext::ReflectedTypeDescriptor,
    mut builder: project_capnp::reflected_type_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_type_path(&descriptor.type_path);
    builder.set_short_path(&descriptor.short_path);
    (descriptor.kind).to_capnp(builder.reborrow().init_kind());
    let mut fields = builder
        .reborrow()
        .init_fields(capnp_list_index(descriptor.fields.len())?);
    for (index, field) in descriptor.fields.iter().enumerate() {
        (field).to_capnp(fields.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut variants = builder
        .reborrow()
        .init_variants(capnp_list_index(descriptor.variants.len())?);
    for (index, variant) in descriptor.variants.iter().enumerate() {
        (variant).to_capnp(variants.reborrow().get(capnp_list_index(index)?))?;
    }
    (descriptor.editor_attributes).to_capnp(builder.reborrow().init_editor_attributes())?;
    write_text_list(
        &descriptor.type_data_flags,
        builder
            .reborrow()
            .init_type_data_flags(capnp_list_index(descriptor.type_data_flags.len())?),
    );
    (descriptor.applicability).to_capnp(builder.reborrow().init_applicability())?;
    if let Some(value) = &descriptor.reflected_default {
        (value).to_capnp(builder.reborrow().init_reflected_default())?;
    }
    Ok(())
}

fn read_type_descriptor(
    reader: project_capnp::reflected_type_descriptor::Reader<'_>,
) -> Result<vnext::ReflectedTypeDescriptor, Error> {
    Ok(vnext::ReflectedTypeDescriptor {
        type_path: text(reader.get_type_path()?)?,
        short_path: text(reader.get_short_path()?)?,
        kind: vnext::ReflectedTypeKind::from_capnp(reader.get_kind()?)?,
        fields: reader
            .get_fields()?
            .iter()
            .map(read_field_descriptor)
            .collect::<Result<_, _>>()?,
        variants: reader
            .get_variants()?
            .iter()
            .map(read_variant_descriptor)
            .collect::<Result<_, _>>()?,
        editor_attributes: vnext::EditorAttributes::from_capnp(reader.get_editor_attributes()?)?,
        type_data_flags: read_text_list(reader.get_type_data_flags()?)?,
        applicability: vnext::ApplicabilityDescriptor::from_capnp(reader.get_applicability()?)?,
        reflected_default: reader
            .has_reflected_default()
            .then(|| vnext::ReflectedValueEnvelope::from_capnp(reader.get_reflected_default()?))
            .transpose()?,
    })
}

fn write_applicability_descriptor(
    descriptor: &vnext::ApplicabilityDescriptor,
    mut builder: project_capnp::applicability_descriptor::Builder<'_>,
) -> Result<(), Error> {
    write_text_list(
        &descriptor.provides,
        builder
            .reborrow()
            .init_provides(capnp_list_index(descriptor.provides.len())?),
    );
    write_text_list(
        &descriptor.requires,
        builder
            .reborrow()
            .init_requires(capnp_list_index(descriptor.requires.len())?),
    );
    write_text_list(
        &descriptor.incompatible,
        builder
            .reborrow()
            .init_incompatible(capnp_list_index(descriptor.incompatible.len())?),
    );
    builder.set_default_available(descriptor.default_available);
    Ok(())
}

fn read_applicability_descriptor(
    reader: project_capnp::applicability_descriptor::Reader<'_>,
) -> Result<vnext::ApplicabilityDescriptor, Error> {
    Ok(vnext::ApplicabilityDescriptor {
        provides: read_text_list(reader.get_provides()?)?,
        requires: read_text_list(reader.get_requires()?)?,
        incompatible: read_text_list(reader.get_incompatible()?)?,
        default_available: reader.get_default_available(),
    })
}

fn write_type_kind(
    kind: &vnext::ReflectedTypeKind,
    mut builder: project_capnp::reflected_type_kind::Builder<'_>,
) {
    match kind {
        vnext::ReflectedTypeKind::Struct => builder.set_struct_kind(()),
        vnext::ReflectedTypeKind::TupleStruct => builder.set_tuple_struct(()),
        vnext::ReflectedTypeKind::Tuple => builder.set_tuple(()),
        vnext::ReflectedTypeKind::List => builder.set_list(()),
        vnext::ReflectedTypeKind::Array { capacity } => builder.set_array(*capacity),
        vnext::ReflectedTypeKind::Map => builder.set_map(()),
        vnext::ReflectedTypeKind::Set => builder.set_set(()),
        vnext::ReflectedTypeKind::Enum => builder.set_enum_kind(()),
        vnext::ReflectedTypeKind::Optional => builder.set_optional(()),
        vnext::ReflectedTypeKind::Bool => builder.set_bool_kind(()),
        vnext::ReflectedTypeKind::SignedInteger { bits } => builder.set_signed_integer(*bits),
        vnext::ReflectedTypeKind::UnsignedInteger { bits } => builder.set_unsigned_integer(*bits),
        vnext::ReflectedTypeKind::Float { bits } => builder.set_float_kind(*bits),
        vnext::ReflectedTypeKind::String => builder.set_string_kind(()),
        vnext::ReflectedTypeKind::Opaque => builder.set_opaque(()),
    }
}

fn read_type_kind(
    reader: project_capnp::reflected_type_kind::Reader<'_>,
) -> Result<vnext::ReflectedTypeKind, Error> {
    use project_capnp::reflected_type_kind::Which;
    Ok(match reader.which()? {
        Which::StructKind(()) => vnext::ReflectedTypeKind::Struct,
        Which::TupleStruct(()) => vnext::ReflectedTypeKind::TupleStruct,
        Which::Tuple(()) => vnext::ReflectedTypeKind::Tuple,
        Which::List(()) => vnext::ReflectedTypeKind::List,
        Which::Array(capacity) => vnext::ReflectedTypeKind::Array { capacity },
        Which::Map(()) => vnext::ReflectedTypeKind::Map,
        Which::Set(()) => vnext::ReflectedTypeKind::Set,
        Which::EnumKind(()) => vnext::ReflectedTypeKind::Enum,
        Which::Optional(()) => vnext::ReflectedTypeKind::Optional,
        Which::BoolKind(()) => vnext::ReflectedTypeKind::Bool,
        Which::SignedInteger(bits) => vnext::ReflectedTypeKind::SignedInteger { bits },
        Which::UnsignedInteger(bits) => vnext::ReflectedTypeKind::UnsignedInteger { bits },
        Which::FloatKind(bits) => vnext::ReflectedTypeKind::Float { bits },
        Which::StringKind(()) => vnext::ReflectedTypeKind::String,
        Which::Opaque(()) => vnext::ReflectedTypeKind::Opaque,
    })
}

fn write_field_descriptor(
    field: &vnext::ReflectedFieldDescriptor,
    mut builder: project_capnp::reflected_field_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_name(&field.name);
    builder.set_type_path(&field.type_path);
    (field.editor_attributes).to_capnp(builder.reborrow().init_editor_attributes())
}

fn read_field_descriptor(
    reader: project_capnp::reflected_field_descriptor::Reader<'_>,
) -> Result<vnext::ReflectedFieldDescriptor, Error> {
    Ok(vnext::ReflectedFieldDescriptor {
        name: text(reader.get_name()?)?,
        type_path: text(reader.get_type_path()?)?,
        editor_attributes: vnext::EditorAttributes::from_capnp(reader.get_editor_attributes()?)?,
    })
}

fn write_variant_descriptor(
    variant: &vnext::ReflectedVariantDescriptor,
    mut builder: project_capnp::reflected_variant_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_name(&variant.name);
    let mut fields = builder
        .reborrow()
        .init_fields(capnp_list_index(variant.fields.len())?);
    for (index, field) in variant.fields.iter().enumerate() {
        (field).to_capnp(fields.reborrow().get(capnp_list_index(index)?))?;
    }
    (variant.editor_attributes).to_capnp(builder.reborrow().init_editor_attributes())
}

fn read_variant_descriptor(
    reader: project_capnp::reflected_variant_descriptor::Reader<'_>,
) -> Result<vnext::ReflectedVariantDescriptor, Error> {
    Ok(vnext::ReflectedVariantDescriptor {
        name: text(reader.get_name()?)?,
        fields: reader
            .get_fields()?
            .iter()
            .map(read_field_descriptor)
            .collect::<Result<_, _>>()?,
        editor_attributes: vnext::EditorAttributes::from_capnp(reader.get_editor_attributes()?)?,
    })
}

fn write_editor_attributes(
    attributes: &vnext::EditorAttributes,
    mut builder: project_capnp::editor_attributes::Builder<'_>,
) -> Result<(), Error> {
    write_optional_text(attributes.label.as_deref(), builder.reborrow().init_label());
    write_optional_text(
        attributes.description.as_deref(),
        builder.reborrow().init_description(),
    );
    write_optional_text(
        attributes.category.as_deref(),
        builder.reborrow().init_category(),
    );
    write_optional_text(attributes.icon.as_deref(), builder.reborrow().init_icon());
    write_optional_text(
        attributes.widget.as_deref(),
        builder.reborrow().init_widget(),
    );
    if let Some(range) = &attributes.range {
        (range).to_capnp(builder.reborrow().init_range());
    }
    builder.set_read_only(attributes.read_only);
    builder.set_hidden(attributes.hidden);
    write_text_list(
        &attributes.action_ids,
        builder
            .reborrow()
            .init_action_ids(capnp_list_index(attributes.action_ids.len())?),
    );
    (attributes.constraints).to_capnp(builder.reborrow().init_constraints())?;
    Ok(())
}

fn read_editor_attributes(
    reader: project_capnp::editor_attributes::Reader<'_>,
) -> Result<vnext::EditorAttributes, Error> {
    Ok(vnext::EditorAttributes {
        label: read_optional_text(reader.get_label()?)?,
        description: read_optional_text(reader.get_description()?)?,
        category: read_optional_text(reader.get_category()?)?,
        icon: read_optional_text(reader.get_icon()?)?,
        widget: read_optional_text(reader.get_widget()?)?,
        range: reader
            .has_range()
            .then(|| vnext::NumericRange::from_capnp(reader.get_range()?))
            .transpose()?,
        read_only: reader.get_read_only(),
        hidden: reader.get_hidden(),
        action_ids: read_text_list(reader.get_action_ids()?)?,
        constraints: vnext::FieldConstraints::from_capnp(reader.get_constraints()?)?,
    })
}

fn write_field_constraints(
    constraints: &vnext::FieldConstraints,
    mut builder: project_capnp::field_constraints::Builder<'_>,
) -> Result<(), Error> {
    write_optional_u32(
        constraints.minimum_length,
        builder.reborrow().init_minimum_length(),
    );
    write_optional_u32(
        constraints.maximum_length,
        builder.reborrow().init_maximum_length(),
    );
    write_text_list(
        &constraints.allowed_strings,
        builder
            .reborrow()
            .init_allowed_strings(capnp_list_index(constraints.allowed_strings.len())?),
    );
    write_text_list(
        &constraints.allowed_variants,
        builder
            .reborrow()
            .init_allowed_variants(capnp_list_index(constraints.allowed_variants.len())?),
    );
    Ok(())
}

fn read_field_constraints(
    reader: project_capnp::field_constraints::Reader<'_>,
) -> Result<vnext::FieldConstraints, Error> {
    Ok(vnext::FieldConstraints {
        minimum_length: read_optional_u32(reader.get_minimum_length()?)?,
        maximum_length: read_optional_u32(reader.get_maximum_length()?)?,
        allowed_strings: read_text_list(reader.get_allowed_strings()?)?,
        allowed_variants: read_text_list(reader.get_allowed_variants()?)?,
    })
}

fn write_numeric_range(
    range: &vnext::NumericRange,
    mut builder: project_capnp::numeric_range::Builder<'_>,
) {
    write_optional_text(range.minimum.as_deref(), builder.reborrow().init_minimum());
    write_optional_text(range.maximum.as_deref(), builder.reborrow().init_maximum());
    write_optional_text(range.step.as_deref(), builder.reborrow().init_step());
    write_optional_text(range.suffix.as_deref(), builder.reborrow().init_suffix());
}

fn read_numeric_range(
    reader: project_capnp::numeric_range::Reader<'_>,
) -> Result<vnext::NumericRange, Error> {
    Ok(vnext::NumericRange {
        minimum: read_optional_text(reader.get_minimum()?)?,
        maximum: read_optional_text(reader.get_maximum()?)?,
        step: read_optional_text(reader.get_step()?)?,
        suffix: read_optional_text(reader.get_suffix()?)?,
    })
}

fn write_value_envelope(
    envelope: &vnext::ReflectedValueEnvelope,
    builder: authoring_capnp::reflected_value_envelope::Builder<'_>,
) -> Result<(), Error> {
    az_proto_authoring::write_reflected_value_envelope(envelope, builder)
}

fn read_value_envelope(
    reader: authoring_capnp::reflected_value_envelope::Reader<'_>,
) -> Result<vnext::ReflectedValueEnvelope, Error> {
    az_proto_authoring::read_reflected_value_envelope(reader)
}

fn write_reflected_path(
    path: &vnext::ReflectedPath,
    mut builder: project_capnp::reflected_path::Builder<'_>,
) -> Result<(), Error> {
    builder.set_component_type_path(&path.component_type_path);
    let mut segments = builder
        .reborrow()
        .init_segments(capnp_list_index(path.segments.len())?);
    for (index, segment) in path.segments.iter().enumerate() {
        let mut target = segments.reborrow().get(capnp_list_index(index)?);
        match segment {
            vnext::ReflectedPathSegment::Field(value) => target.set_field(value),
            vnext::ReflectedPathSegment::Variant(value) => target.set_variant(value),
            vnext::ReflectedPathSegment::TupleIndex(value) => target.set_tuple_index(*value),
            vnext::ReflectedPathSegment::ListIndex(value) => target.set_list_index(*value),
        }
    }
    Ok(())
}

fn read_reflected_path(
    reader: project_capnp::reflected_path::Reader<'_>,
) -> Result<vnext::ReflectedPath, Error> {
    use project_capnp::reflected_path::segment::Which;
    let segments = reader
        .get_segments()?
        .iter()
        .map(|segment| {
            Ok(match segment.which()? {
                Which::Field(value) => vnext::ReflectedPathSegment::Field(text(value?)?),
                Which::Variant(value) => vnext::ReflectedPathSegment::Variant(text(value?)?),
                Which::TupleIndex(value) => vnext::ReflectedPathSegment::TupleIndex(value),
                Which::ListIndex(value) => vnext::ReflectedPathSegment::ListIndex(value),
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(vnext::ReflectedPath {
        component_type_path: text(reader.get_component_type_path()?)?,
        segments,
    })
}

fn write_value_target(
    target: &vnext::PrefabValueTarget,
    mut builder: project_capnp::prefab_value_target::Builder<'_>,
) -> Result<(), Error> {
    write_text_list(
        &target.instance_alias_chain,
        builder
            .reborrow()
            .init_instance_alias_chain(capnp_list_index(target.instance_alias_chain.len())?),
    );
    builder.set_entity_alias(&target.entity_alias);
    (target.path).to_capnp(builder.reborrow().init_path())
}

fn read_value_target(
    reader: project_capnp::prefab_value_target::Reader<'_>,
) -> Result<vnext::PrefabValueTarget, Error> {
    Ok(vnext::PrefabValueTarget {
        instance_alias_chain: read_text_list(reader.get_instance_alias_chain()?)?,
        entity_alias: text(reader.get_entity_alias()?)?,
        path: vnext::ReflectedPath::from_capnp(reader.get_path()?)?,
    })
}

#[cfg(test)]
fn round_trip_prefab_source_snapshot(
    snapshot: &vnext::PrefabSourceSnapshot,
) -> Result<vnext::PrefabSourceSnapshot, Error> {
    let mut message = message::Builder::new_default();
    snapshot.to_capnp(message.init_root::<project_capnp::prefab_source_snapshot::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::PrefabSourceSnapshot::from_capnp(
        reader.get_root::<project_capnp::prefab_source_snapshot::Reader<'_>>()?,
    )
}

fn write_prefab_source_snapshot(
    snapshot: &vnext::PrefabSourceSnapshot,
    mut builder: project_capnp::prefab_source_snapshot::Builder<'_>,
) -> Result<(), Error> {
    builder.set_document_version(snapshot.document_version);
    builder.set_revision(snapshot.revision);

    let mut versions = builder
        .reborrow()
        .init_type_versions(capnp_list_index(snapshot.type_versions.len())?);
    for (index, (type_path, version)) in snapshot.type_versions.iter().enumerate() {
        let mut entry = versions.reborrow().get(capnp_list_index(index)?);
        entry.set_type_path(type_path);
        entry.set_version(*version);
    }

    let mut entities = builder
        .reborrow()
        .init_entities(capnp_list_index(snapshot.entities.len())?);
    for (index, entity) in snapshot.entities.iter().enumerate() {
        entities
            .reborrow()
            .get(capnp_list_index(index)?)
            .set_alias(&entity.alias);
    }

    let mut hierarchy = builder
        .reborrow()
        .init_hierarchy(capnp_list_index(snapshot.hierarchy.len())?);
    for (index, edge) in snapshot.hierarchy.iter().enumerate() {
        let mut target = hierarchy.reborrow().get(capnp_list_index(index)?);
        target.set_child_alias(&edge.child_alias);
        write_optional_text(
            edge.parent_alias.as_deref(),
            target.reborrow().init_parent_alias(),
        );
    }

    let mut components = builder
        .reborrow()
        .init_components(capnp_list_index(snapshot.components.len())?);
    for (index, component) in snapshot.components.iter().enumerate() {
        let mut target = components.reborrow().get(capnp_list_index(index)?);
        target.set_entity_alias(&component.entity_alias);
        target.set_type_path(&component.type_path);
        (component.sparse_value).to_capnp(target.reborrow().init_sparse_value())?;
    }

    let mut instances = builder
        .reborrow()
        .init_instances(capnp_list_index(snapshot.instances.len())?);
    for (index, instance) in snapshot.instances.iter().enumerate() {
        let mut target = instances.reborrow().get(capnp_list_index(index)?);
        target.set_alias(&instance.alias);
        target.set_source_asset(&instance.source_asset);
        write_optional_text(
            instance.parent_entity_alias.as_deref(),
            target.reborrow().init_parent_entity_alias(),
        );
    }

    let mut overrides = builder
        .reborrow()
        .init_overrides(capnp_list_index(snapshot.overrides.len())?);
    for (index, snapshot) in snapshot.overrides.iter().enumerate() {
        let mut builder = overrides.reborrow().get(capnp_list_index(index)?);
        (snapshot.operation).to_capnp(builder.reborrow().init_operation())?;
        (snapshot.operation.target()).to_capnp(builder.reborrow().init_target())?;
        if let vnext::PrefabOverrideOperation::Set { value, .. } = &snapshot.operation {
            (value).to_capnp(builder.reborrow().init_value())?;
        }
    }
    Ok(())
}

fn read_prefab_source_snapshot(
    reader: project_capnp::prefab_source_snapshot::Reader<'_>,
) -> Result<vnext::PrefabSourceSnapshot, Error> {
    let type_versions = reader
        .get_type_versions()?
        .iter()
        .map(|entry| Ok((text(entry.get_type_path()?)?, entry.get_version())))
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let entities = reader
        .get_entities()?
        .iter()
        .map(|entity| {
            Ok(vnext::PrefabEntitySnapshot {
                alias: text(entity.get_alias()?)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    let hierarchy = reader
        .get_hierarchy()?
        .iter()
        .map(|edge| {
            Ok(vnext::PrefabHierarchyEdge {
                child_alias: text(edge.get_child_alias()?)?,
                parent_alias: read_parent(edge.get_parent_alias()?)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    let components = reader
        .get_components()?
        .iter()
        .map(|component| {
            Ok(vnext::PrefabComponentSnapshot {
                entity_alias: text(component.get_entity_alias()?)?,
                type_path: text(component.get_type_path()?)?,
                sparse_value: vnext::ReflectedValueEnvelope::from_capnp(
                    component.get_sparse_value()?,
                )?,
            })
        })
        .collect::<Result<_, Error>>()?;
    let instances = reader
        .get_instances()?
        .iter()
        .map(|instance| {
            Ok(vnext::PrefabInstanceSnapshot {
                alias: text(instance.get_alias()?)?,
                source_asset: text(instance.get_source_asset()?)?,
                parent_entity_alias: read_parent(instance.get_parent_entity_alias()?)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    let overrides = reader
        .get_overrides()?
        .iter()
        .map(|snapshot| {
            let operation = if snapshot.has_operation() {
                vnext::PrefabOverrideOperation::from_capnp(snapshot.get_operation()?)?
            } else {
                vnext::PrefabOverrideOperation::Set {
                    target: vnext::PrefabValueTarget::from_capnp(snapshot.get_target()?)?,
                    value: vnext::ReflectedValueEnvelope::from_capnp(snapshot.get_value()?)?,
                }
            };
            Ok(vnext::PrefabOverrideSnapshot { operation })
        })
        .collect::<Result<_, Error>>()?;
    Ok(vnext::PrefabSourceSnapshot {
        document_version: reader.get_document_version(),
        type_versions,
        entities,
        hierarchy,
        components,
        instances,
        overrides,
        revision: reader.get_revision(),
    })
}

fn write_prefab_override_operation(
    operation: &vnext::PrefabOverrideOperation,
    mut builder: project_capnp::prefab_override_operation::Builder<'_>,
) -> Result<(), Error> {
    match operation {
        vnext::PrefabOverrideOperation::Set { target, value } => {
            let mut operation = builder.reborrow().init_set();
            (target).to_capnp(operation.reborrow().init_target())?;
            (value).to_capnp(operation.reborrow().init_value())?;
        }
        vnext::PrefabOverrideOperation::Clear { target } => {
            (target).to_capnp(builder.reborrow().init_clear().init_target())?;
        }
        vnext::PrefabOverrideOperation::Insert {
            target,
            index,
            value,
        } => {
            let mut operation = builder.reborrow().init_insert();
            (target).to_capnp(operation.reborrow().init_target())?;
            operation.set_index(*index);
            (value).to_capnp(operation.reborrow().init_value())?;
        }
        vnext::PrefabOverrideOperation::Remove { target, index } => {
            let mut operation = builder.reborrow().init_remove();
            (target).to_capnp(operation.reborrow().init_target())?;
            operation.set_index(*index);
        }
        vnext::PrefabOverrideOperation::Move { target, from, to } => {
            let mut operation = builder.reborrow().init_move();
            (target).to_capnp(operation.reborrow().init_target())?;
            operation.set_from(*from);
            operation.set_to(*to);
        }
    }
    Ok(())
}

fn read_prefab_override_operation(
    reader: project_capnp::prefab_override_operation::Reader<'_>,
) -> Result<vnext::PrefabOverrideOperation, Error> {
    use project_capnp::prefab_override_operation::Which;
    Ok(match reader.which()? {
        Which::Set(operation) => {
            let operation = operation?;
            vnext::PrefabOverrideOperation::Set {
                target: vnext::PrefabValueTarget::from_capnp(operation.get_target()?)?,
                value: vnext::ReflectedValueEnvelope::from_capnp(operation.get_value()?)?,
            }
        }
        Which::Clear(operation) => vnext::PrefabOverrideOperation::Clear {
            target: vnext::PrefabValueTarget::from_capnp(operation?.get_target()?)?,
        },
        Which::Insert(operation) => {
            let operation = operation?;
            vnext::PrefabOverrideOperation::Insert {
                target: vnext::PrefabValueTarget::from_capnp(operation.get_target()?)?,
                index: operation.get_index(),
                value: vnext::ReflectedValueEnvelope::from_capnp(operation.get_value()?)?,
            }
        }
        Which::Remove(operation) => {
            let operation = operation?;
            vnext::PrefabOverrideOperation::Remove {
                target: vnext::PrefabValueTarget::from_capnp(operation.get_target()?)?,
                index: operation.get_index(),
            }
        }
        Which::Move(operation) => {
            let operation = operation?;
            vnext::PrefabOverrideOperation::Move {
                target: vnext::PrefabValueTarget::from_capnp(operation.get_target()?)?,
                from: operation.get_from(),
                to: operation.get_to(),
            }
        }
    })
}

#[cfg(test)]
fn round_trip_prefab_edit_command(
    command: &vnext::PrefabEditCommand,
) -> Result<vnext::PrefabEditCommand, Error> {
    let mut message = message::Builder::new_default();
    command.to_capnp(message.init_root::<project_capnp::prefab_edit_command::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::PrefabEditCommand::from_capnp(
        reader.get_root::<project_capnp::prefab_edit_command::Reader<'_>>()?,
    )
}

fn write_prefab_edit_command(
    command: &vnext::PrefabEditCommand,
    mut builder: project_capnp::prefab_edit_command::Builder<'_>,
) -> Result<(), Error> {
    match command {
        vnext::PrefabEditCommand::SetValue { target, value } => {
            let mut command = builder.reborrow().init_set_value();
            (target).to_capnp(command.reborrow().init_target())?;
            (value).to_capnp(command.reborrow().init_value())?;
        }
        vnext::PrefabEditCommand::ListInsert {
            target,
            index,
            value,
        } => {
            let mut command = builder.reborrow().init_list_insert();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_index(*index);
            (value).to_capnp(command.reborrow().init_value())?;
        }
        vnext::PrefabEditCommand::ListRemove { target, index } => {
            let mut command = builder.reborrow().init_list_remove();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_index(*index);
        }
        vnext::PrefabEditCommand::ListMove { target, from, to } => {
            let mut command = builder.reborrow().init_list_move();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_from(*from);
            command.set_to(*to);
        }
        vnext::PrefabEditCommand::MapInsert { target, key, value } => {
            let mut command = builder.reborrow().init_map_insert();
            (target).to_capnp(command.reborrow().init_target())?;
            (key).to_capnp(command.reborrow().init_key())?;
            (value).to_capnp(command.reborrow().init_value())?;
        }
        vnext::PrefabEditCommand::MapRemove { target, key } => {
            let mut command = builder.reborrow().init_map_remove();
            (target).to_capnp(command.reborrow().init_target())?;
            (key).to_capnp(command.reborrow().init_key())?;
        }
        vnext::PrefabEditCommand::SetVariant {
            target,
            variant_name,
            value,
        } => {
            let mut command = builder.reborrow().init_set_variant();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_variant_name(variant_name);
            if let Some(value) = value {
                (value).to_capnp(command.reborrow().init_value())?;
            }
        }
        vnext::PrefabEditCommand::AddComponent {
            entity_alias,
            component_type_path,
            initial_value,
        } => {
            let mut command = builder.reborrow().init_add_component();
            command.set_entity_alias(entity_alias);
            command.set_component_type_path(component_type_path);
            if let Some(value) = initial_value {
                (value).to_capnp(command.reborrow().init_initial_value())?;
            }
        }
        vnext::PrefabEditCommand::RemoveComponent {
            entity_alias,
            component_type_path,
        } => {
            let mut command = builder.reborrow().init_remove_component();
            command.set_entity_alias(entity_alias);
            command.set_component_type_path(component_type_path);
        }
        vnext::PrefabEditCommand::AddEntity { .. }
        | vnext::PrefabEditCommand::RemoveEntity { .. }
        | vnext::PrefabEditCommand::ReparentEntity { .. }
        | vnext::PrefabEditCommand::AddInstance { .. }
        | vnext::PrefabEditCommand::RemoveInstance { .. }
        | vnext::PrefabEditCommand::ReparentInstance { .. } => {
            write_prefab_hierarchy_edit_command(command, builder);
        }
        vnext::PrefabEditCommand::SetOverride { .. }
        | vnext::PrefabEditCommand::ClearOverride { .. }
        | vnext::PrefabEditCommand::InsertOverride { .. }
        | vnext::PrefabEditCommand::RemoveOverrideItem { .. }
        | vnext::PrefabEditCommand::MoveOverride { .. }
        | vnext::PrefabEditCommand::RemoveOverride { .. } => {
            write_prefab_override_edit_command(command, builder)?;
        }
    }
    Ok(())
}

/// Writes the commands that restructure the entity/instance hierarchy.
///
/// Only reachable through [`write_prefab_edit_command`], which routes exactly
/// the six hierarchy variants here.
fn write_prefab_hierarchy_edit_command(
    command: &vnext::PrefabEditCommand,
    mut builder: project_capnp::prefab_edit_command::Builder<'_>,
) {
    match command {
        vnext::PrefabEditCommand::AddEntity {
            alias,
            parent_alias,
        } => {
            let mut command = builder.reborrow().init_add_entity();
            command.set_alias(alias);
            write_optional_text(
                parent_alias.as_deref(),
                command.reborrow().init_parent_alias(),
            );
        }
        vnext::PrefabEditCommand::RemoveEntity { alias } => {
            builder.reborrow().init_remove_entity().set_alias(alias);
        }
        vnext::PrefabEditCommand::ReparentEntity {
            alias,
            parent_alias,
        } => {
            let mut command = builder.reborrow().init_reparent_entity();
            command.set_alias(alias);
            write_optional_text(
                parent_alias.as_deref(),
                command.reborrow().init_parent_alias(),
            );
        }
        vnext::PrefabEditCommand::AddInstance {
            alias,
            source_asset,
            parent_entity_alias,
        } => {
            let mut command = builder.reborrow().init_add_instance();
            command.set_alias(alias);
            command.set_source_asset(source_asset);
            write_optional_text(
                parent_entity_alias.as_deref(),
                command.reborrow().init_parent_entity_alias(),
            );
        }
        vnext::PrefabEditCommand::RemoveInstance { alias } => {
            builder.reborrow().init_remove_instance().set_alias(alias);
        }
        vnext::PrefabEditCommand::ReparentInstance {
            alias,
            parent_entity_alias,
        } => {
            let mut command = builder.reborrow().init_reparent_instance();
            command.set_alias(alias);
            write_optional_text(
                parent_entity_alias.as_deref(),
                command.reborrow().init_parent_entity_alias(),
            );
        }
        _ => unreachable!("write_prefab_edit_command routes only hierarchy commands here"),
    }
}

/// Writes the commands that edit an instance's authored overrides.
///
/// Only reachable through [`write_prefab_edit_command`], which routes exactly
/// the six override variants here.
fn write_prefab_override_edit_command(
    command: &vnext::PrefabEditCommand,
    mut builder: project_capnp::prefab_edit_command::Builder<'_>,
) -> Result<(), Error> {
    match command {
        vnext::PrefabEditCommand::SetOverride { target, value } => {
            let mut command = builder.reborrow().init_set_override();
            (target).to_capnp(command.reborrow().init_target())?;
            (value).to_capnp(command.reborrow().init_value())?;
        }
        vnext::PrefabEditCommand::ClearOverride { target } => {
            let mut command = builder.reborrow().init_clear_override();
            (target).to_capnp(command.reborrow().init_target())?;
        }
        vnext::PrefabEditCommand::InsertOverride {
            target,
            index,
            value,
        } => {
            let mut command = builder.reborrow().init_insert_override();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_index(*index);
            (value).to_capnp(command.reborrow().init_value())?;
        }
        vnext::PrefabEditCommand::RemoveOverrideItem { target, index } => {
            let mut command = builder.reborrow().init_remove_override_item();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_index(*index);
        }
        vnext::PrefabEditCommand::MoveOverride { target, from, to } => {
            let mut command = builder.reborrow().init_move_override();
            (target).to_capnp(command.reborrow().init_target())?;
            command.set_from(*from);
            command.set_to(*to);
        }
        vnext::PrefabEditCommand::RemoveOverride { target } => {
            let mut command = builder.reborrow().init_remove_override();
            (target).to_capnp(command.reborrow().init_target())?;
        }
        _ => unreachable!("write_prefab_edit_command routes only override commands here"),
    }
    Ok(())
}

fn read_prefab_edit_command(
    reader: project_capnp::prefab_edit_command::Reader<'_>,
) -> Result<vnext::PrefabEditCommand, Error> {
    use project_capnp::prefab_edit_command::Which;
    Ok(match reader.which()? {
        Which::SetValue(command) => {
            let command = command?;
            vnext::PrefabEditCommand::SetValue {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                value: vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?)?,
            }
        }
        Which::ListInsert(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ListInsert {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                index: command.get_index(),
                value: vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?)?,
            }
        }
        Which::ListRemove(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ListRemove {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                index: command.get_index(),
            }
        }
        Which::ListMove(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ListMove {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                from: command.get_from(),
                to: command.get_to(),
            }
        }
        Which::MapInsert(command) => {
            let command = command?;
            vnext::PrefabEditCommand::MapInsert {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                key: vnext::ReflectedValueEnvelope::from_capnp(command.get_key()?)?,
                value: vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?)?,
            }
        }
        Which::MapRemove(command) => {
            let command = command?;
            vnext::PrefabEditCommand::MapRemove {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                key: vnext::ReflectedValueEnvelope::from_capnp(command.get_key()?)?,
            }
        }
        Which::SetVariant(command) => {
            let command = command?;
            vnext::PrefabEditCommand::SetVariant {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                variant_name: text(command.get_variant_name()?)?,
                value: command
                    .has_value()
                    .then(|| vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?))
                    .transpose()?,
            }
        }
        Which::AddComponent(command) => {
            let command = command?;
            vnext::PrefabEditCommand::AddComponent {
                entity_alias: text(command.get_entity_alias()?)?,
                component_type_path: text(command.get_component_type_path()?)?,
                initial_value: command
                    .has_initial_value()
                    .then(|| {
                        vnext::ReflectedValueEnvelope::from_capnp(command.get_initial_value()?)
                    })
                    .transpose()?,
            }
        }
        Which::RemoveComponent(command) => {
            let command = command?;
            vnext::PrefabEditCommand::RemoveComponent {
                entity_alias: text(command.get_entity_alias()?)?,
                component_type_path: text(command.get_component_type_path()?)?,
            }
        }
        Which::AddEntity(_)
        | Which::RemoveEntity(_)
        | Which::ReparentEntity(_)
        | Which::AddInstance(_)
        | Which::RemoveInstance(_)
        | Which::ReparentInstance(_) => read_prefab_hierarchy_edit_command(reader)?,
        Which::SetOverride(_)
        | Which::ClearOverride(_)
        | Which::InsertOverride(_)
        | Which::RemoveOverrideItem(_)
        | Which::MoveOverride(_)
        | Which::RemoveOverride(_) => read_prefab_override_edit_command(reader)?,
    })
}

/// Reads the commands that restructure the entity/instance hierarchy.
///
/// Only reachable through [`read_prefab_edit_command`], which routes exactly
/// the six hierarchy union tags here.
fn read_prefab_hierarchy_edit_command(
    reader: project_capnp::prefab_edit_command::Reader<'_>,
) -> Result<vnext::PrefabEditCommand, Error> {
    use project_capnp::prefab_edit_command::Which;
    Ok(match reader.which()? {
        Which::AddEntity(command) => {
            let command = command?;
            vnext::PrefabEditCommand::AddEntity {
                alias: text(command.get_alias()?)?,
                parent_alias: read_parent(command.get_parent_alias()?)?,
            }
        }
        Which::RemoveEntity(command) => vnext::PrefabEditCommand::RemoveEntity {
            alias: text(command?.get_alias()?)?,
        },
        Which::ReparentEntity(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ReparentEntity {
                alias: text(command.get_alias()?)?,
                parent_alias: read_parent(command.get_parent_alias()?)?,
            }
        }
        Which::AddInstance(command) => {
            let command = command?;
            vnext::PrefabEditCommand::AddInstance {
                alias: text(command.get_alias()?)?,
                source_asset: text(command.get_source_asset()?)?,
                parent_entity_alias: read_parent(command.get_parent_entity_alias()?)?,
            }
        }
        Which::RemoveInstance(command) => vnext::PrefabEditCommand::RemoveInstance {
            alias: text(command?.get_alias()?)?,
        },
        Which::ReparentInstance(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ReparentInstance {
                alias: text(command.get_alias()?)?,
                parent_entity_alias: read_parent(command.get_parent_entity_alias()?)?,
            }
        }
        _ => unreachable!("read_prefab_edit_command routes only hierarchy commands here"),
    })
}

/// Reads the commands that edit an instance's authored overrides.
///
/// Only reachable through [`read_prefab_edit_command`], which routes exactly
/// the six override union tags here.
fn read_prefab_override_edit_command(
    reader: project_capnp::prefab_edit_command::Reader<'_>,
) -> Result<vnext::PrefabEditCommand, Error> {
    use project_capnp::prefab_edit_command::Which;
    Ok(match reader.which()? {
        Which::SetOverride(command) => {
            let command = command?;
            vnext::PrefabEditCommand::SetOverride {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                value: vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?)?,
            }
        }
        Which::ClearOverride(command) => {
            let command = command?;
            vnext::PrefabEditCommand::ClearOverride {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
            }
        }
        Which::InsertOverride(command) => {
            let command = command?;
            vnext::PrefabEditCommand::InsertOverride {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                index: command.get_index(),
                value: vnext::ReflectedValueEnvelope::from_capnp(command.get_value()?)?,
            }
        }
        Which::RemoveOverrideItem(command) => {
            let command = command?;
            vnext::PrefabEditCommand::RemoveOverrideItem {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                index: command.get_index(),
            }
        }
        Which::MoveOverride(command) => {
            let command = command?;
            vnext::PrefabEditCommand::MoveOverride {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
                from: command.get_from(),
                to: command.get_to(),
            }
        }
        Which::RemoveOverride(command) => {
            let command = command?;
            vnext::PrefabEditCommand::RemoveOverride {
                target: vnext::PrefabValueTarget::from_capnp(command.get_target()?)?,
            }
        }
        _ => unreachable!("read_prefab_edit_command routes only override commands here"),
    })
}

fn write_prefab_diagnostic(
    diagnostic: &vnext::PrefabDiagnostic,
    mut builder: project_capnp::prefab_diagnostic::Builder<'_>,
) -> Result<(), Error> {
    builder.set_severity(match diagnostic.severity {
        vnext::DiagnosticSeverity::Info => project_capnp::VNextDiagnosticSeverity::Info,
        vnext::DiagnosticSeverity::Warning => project_capnp::VNextDiagnosticSeverity::Warning,
        vnext::DiagnosticSeverity::Error => project_capnp::VNextDiagnosticSeverity::Error,
    });
    builder.set_code(&diagnostic.code);
    builder.set_message(&diagnostic.message);
    if let Some(target) = &diagnostic.target {
        (target).to_capnp(builder.reborrow().init_target())?;
    }
    Ok(())
}

fn read_prefab_diagnostic(
    reader: project_capnp::prefab_diagnostic::Reader<'_>,
) -> Result<vnext::PrefabDiagnostic, Error> {
    let severity = match reader.get_severity()? {
        project_capnp::VNextDiagnosticSeverity::Info => vnext::DiagnosticSeverity::Info,
        project_capnp::VNextDiagnosticSeverity::Warning => vnext::DiagnosticSeverity::Warning,
        project_capnp::VNextDiagnosticSeverity::Error => vnext::DiagnosticSeverity::Error,
    };
    Ok(vnext::PrefabDiagnostic {
        severity,
        code: text(reader.get_code()?)?,
        message: text(reader.get_message()?)?,
        target: reader
            .has_target()
            .then(|| vnext::PrefabValueTarget::from_capnp(reader.get_target()?))
            .transpose()?,
    })
}

#[cfg(test)]
fn round_trip_prefab_rpc_result(
    result: &vnext::PrefabRpcResult,
) -> Result<vnext::PrefabRpcResult, Error> {
    let mut message = message::Builder::new_default();
    result.to_capnp(message.init_root::<project_capnp::prefab_rpc_result::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::PrefabRpcResult::from_capnp(
        reader.get_root::<project_capnp::prefab_rpc_result::Reader<'_>>()?,
    )
}

fn write_prefab_rpc_result(
    result: &vnext::PrefabRpcResult,
    mut builder: project_capnp::prefab_rpc_result::Builder<'_>,
) -> Result<(), Error> {
    if let Some(snapshot) = &result.snapshot {
        (snapshot).to_capnp(builder.reborrow().init_snapshot())?;
    }
    write_diagnostics(
        &result.diagnostics,
        builder
            .reborrow()
            .init_diagnostics(capnp_list_index(result.diagnostics.len())?),
    )
}

fn read_prefab_rpc_result(
    reader: project_capnp::prefab_rpc_result::Reader<'_>,
) -> Result<vnext::PrefabRpcResult, Error> {
    Ok(vnext::PrefabRpcResult {
        snapshot: reader
            .has_snapshot()
            .then(|| vnext::PrefabSourceSnapshot::from_capnp(reader.get_snapshot()?))
            .transpose()?,
        diagnostics: read_diagnostics(reader.get_diagnostics()?)?,
    })
}

#[cfg(test)]
fn round_trip_typed_action_result(
    result: &vnext::TypedActionResult,
) -> Result<vnext::TypedActionResult, Error> {
    let mut message = message::Builder::new_default();
    result.to_capnp(message.init_root::<project_capnp::typed_action_result::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::TypedActionResult::from_capnp(
        reader.get_root::<project_capnp::typed_action_result::Reader<'_>>()?,
    )
}

fn write_typed_action_result(
    result: &vnext::TypedActionResult,
    mut builder: project_capnp::typed_action_result::Builder<'_>,
) -> Result<(), Error> {
    if let Some(snapshot) = &result.snapshot {
        (snapshot).to_capnp(builder.reborrow().init_snapshot())?;
    }
    let mut paths = builder
        .reborrow()
        .init_changed_paths(capnp_list_index(result.changed_paths.len())?);
    for (index, path) in result.changed_paths.iter().enumerate() {
        (path).to_capnp(paths.reborrow().get(capnp_list_index(index)?))?;
    }
    write_diagnostics(
        &result.diagnostics,
        builder
            .reborrow()
            .init_diagnostics(capnp_list_index(result.diagnostics.len())?),
    )
}

fn read_typed_action_result(
    reader: project_capnp::typed_action_result::Reader<'_>,
) -> Result<vnext::TypedActionResult, Error> {
    Ok(vnext::TypedActionResult {
        snapshot: reader
            .has_snapshot()
            .then(|| vnext::PrefabSourceSnapshot::from_capnp(reader.get_snapshot()?))
            .transpose()?,
        changed_paths: reader
            .get_changed_paths()?
            .iter()
            .map(read_reflected_path)
            .collect::<Result<_, _>>()?,
        diagnostics: read_diagnostics(reader.get_diagnostics()?)?,
    })
}

fn write_source_session_status(
    status: &vnext::SourceSessionStatus,
    mut builder: project_capnp::source_session_status::Builder<'_>,
) {
    builder.set_open(status.open);
    builder.set_revision(status.revision);
    builder.set_dirty(status.dirty);
    builder.set_undo_depth(status.undo_depth);
    builder.set_redo_depth(status.redo_depth);
}

fn read_source_session_status(
    reader: project_capnp::source_session_status::Reader<'_>,
) -> vnext::SourceSessionStatus {
    vnext::SourceSessionStatus {
        open: reader.get_open(),
        revision: reader.get_revision(),
        dirty: reader.get_dirty(),
        undo_depth: reader.get_undo_depth(),
        redo_depth: reader.get_redo_depth(),
    }
}

impl vnext::SourceSessionCommand {
    #[must_use]
    pub const fn to_capnp(self) -> project_capnp::SourceSessionCommand {
        match self {
            Self::Open => project_capnp::SourceSessionCommand::Open,
            Self::Save => project_capnp::SourceSessionCommand::Save,
            Self::SaveRecovery => project_capnp::SourceSessionCommand::SaveRecovery,
            Self::Undo => project_capnp::SourceSessionCommand::Undo,
            Self::Redo => project_capnp::SourceSessionCommand::Redo,
            Self::Close => project_capnp::SourceSessionCommand::Close,
            Self::Status => project_capnp::SourceSessionCommand::Status,
        }
    }

    #[must_use]
    pub const fn from_capnp(command: project_capnp::SourceSessionCommand) -> Self {
        match command {
            project_capnp::SourceSessionCommand::Open => Self::Open,
            project_capnp::SourceSessionCommand::Save => Self::Save,
            project_capnp::SourceSessionCommand::SaveRecovery => Self::SaveRecovery,
            project_capnp::SourceSessionCommand::Undo => Self::Undo,
            project_capnp::SourceSessionCommand::Redo => Self::Redo,
            project_capnp::SourceSessionCommand::Close => Self::Close,
            project_capnp::SourceSessionCommand::Status => Self::Status,
        }
    }
}

#[cfg(test)]
fn round_trip_source_session_result(
    result: &vnext::SourceSessionResult,
) -> Result<vnext::SourceSessionResult, Error> {
    let mut message = message::Builder::new_default();
    result.to_capnp(message.init_root::<project_capnp::source_session_result::Builder<'_>>())?;
    let bytes = packed_message(&message)?;
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    vnext::SourceSessionResult::from_capnp(
        reader.get_root::<project_capnp::source_session_result::Reader<'_>>()?,
    )
}

fn write_source_session_result(
    result: &vnext::SourceSessionResult,
    mut builder: project_capnp::source_session_result::Builder<'_>,
) -> Result<(), Error> {
    (result.status).to_capnp(builder.reborrow().init_status());
    if let Some(snapshot) = &result.snapshot {
        (snapshot).to_capnp(builder.reborrow().init_snapshot())?;
    }
    write_diagnostics(
        &result.diagnostics,
        builder
            .reborrow()
            .init_diagnostics(capnp_list_index(result.diagnostics.len())?),
    )
}

fn read_source_session_result(
    reader: project_capnp::source_session_result::Reader<'_>,
) -> Result<vnext::SourceSessionResult, Error> {
    Ok(vnext::SourceSessionResult {
        status: vnext::SourceSessionStatus::from_capnp(reader.get_status()?),
        snapshot: reader
            .has_snapshot()
            .then(|| vnext::PrefabSourceSnapshot::from_capnp(reader.get_snapshot()?))
            .transpose()?,
        diagnostics: read_diagnostics(reader.get_diagnostics()?)?,
    })
}

fn write_diagnostics(
    diagnostics: &[vnext::PrefabDiagnostic],
    mut builder: capnp::struct_list::Builder<'_, project_capnp::prefab_diagnostic::Owned>,
) -> Result<(), Error> {
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        (diagnostic).to_capnp(builder.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_diagnostics(
    reader: capnp::struct_list::Reader<'_, project_capnp::prefab_diagnostic::Owned>,
) -> Result<Vec<vnext::PrefabDiagnostic>, Error> {
    reader.iter().map(read_prefab_diagnostic).collect()
}

fn read_parent(
    reader: crate::core::core_capnp::optional_text::Reader<'_>,
) -> Result<Option<String>, Error> {
    read_optional_text(reader)
}

#[cfg(test)]
fn packed_message(message: &message::Builder<message::HeapAllocator>) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, message)?;
    Ok(bytes)
}

fn text(value: capnp::text::Reader<'_>) -> Result<String, Error> {
    Ok(value.to_string()?)
}

fn write_optional_text(
    value: Option<&str>,
    mut builder: crate::core::core_capnp::optional_text::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_text(
    reader: crate::core::core_capnp::optional_text::Reader<'_>,
) -> Result<Option<String>, Error> {
    match reader.which()? {
        crate::core::core_capnp::optional_text::Which::None(()) => Ok(None),
        crate::core::core_capnp::optional_text::Which::Value(value) => {
            value.and_then(text).map(Some)
        }
    }
}

fn write_optional_u32(
    value: Option<u32>,
    mut builder: crate::core::core_capnp::optional_u32::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_u32(
    reader: crate::core::core_capnp::optional_u32::Reader<'_>,
) -> Result<Option<u32>, Error> {
    match reader.which()? {
        crate::core::core_capnp::optional_u32::Which::None(()) => Ok(None),
        crate::core::core_capnp::optional_u32::Which::Value(value) => Ok(Some(value)),
    }
}

fn write_text_list(values: &[String], mut builder: capnp::text_list::Builder<'_>) {
    for (index, value) in values.iter().enumerate() {
        builder.set(capnp_bounded_index(index), value);
    }
}

fn read_text_list(reader: capnp::text_list::Reader<'_>) -> Result<Vec<String>, Error> {
    reader.iter().map(|value| value.and_then(text)).collect()
}

impl vnext::TypeRegistrySnapshot {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::type_registry_snapshot::Builder<'_>,
    ) -> Result<(), Error> {
        write_type_registry_snapshot(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::type_registry_snapshot::Reader<'_>,
    ) -> Result<Self, Error> {
        read_type_registry_snapshot(reader)
    }
}

impl vnext::ReflectedTypeDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::reflected_type_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_type_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::reflected_type_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_type_descriptor(reader)
    }
}

impl vnext::ApplicabilityDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::applicability_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_applicability_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::applicability_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_applicability_descriptor(reader)
    }
}

impl vnext::ReflectedTypeKind {
    pub fn to_capnp(&self, builder: project_capnp::reflected_type_kind::Builder<'_>) {
        write_type_kind(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::reflected_type_kind::Reader<'_>,
    ) -> Result<Self, Error> {
        read_type_kind(reader)
    }
}

impl vnext::ReflectedFieldDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::reflected_field_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_field_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::reflected_field_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_field_descriptor(reader)
    }
}

impl vnext::ReflectedVariantDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::reflected_variant_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_variant_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::reflected_variant_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_variant_descriptor(reader)
    }
}

impl vnext::EditorAttributes {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::editor_attributes::Builder<'_>,
    ) -> Result<(), Error> {
        write_editor_attributes(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::editor_attributes::Reader<'_>) -> Result<Self, Error> {
        read_editor_attributes(reader)
    }
}

impl vnext::FieldConstraints {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::field_constraints::Builder<'_>,
    ) -> Result<(), Error> {
        write_field_constraints(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::field_constraints::Reader<'_>) -> Result<Self, Error> {
        read_field_constraints(reader)
    }
}

impl vnext::NumericRange {
    pub fn to_capnp(&self, builder: project_capnp::numeric_range::Builder<'_>) {
        write_numeric_range(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::numeric_range::Reader<'_>) -> Result<Self, Error> {
        read_numeric_range(reader)
    }
}

pub trait ReflectedValueEnvelopeCapnp: Sized {
    fn to_capnp(
        &self,
        builder: authoring_capnp::reflected_value_envelope::Builder<'_>,
    ) -> Result<(), Error>;

    fn from_capnp(
        reader: authoring_capnp::reflected_value_envelope::Reader<'_>,
    ) -> Result<Self, Error>;
}

impl ReflectedValueEnvelopeCapnp for vnext::ReflectedValueEnvelope {
    fn to_capnp(
        &self,
        builder: authoring_capnp::reflected_value_envelope::Builder<'_>,
    ) -> Result<(), Error> {
        write_value_envelope(self, builder)
    }

    fn from_capnp(
        reader: authoring_capnp::reflected_value_envelope::Reader<'_>,
    ) -> Result<Self, Error> {
        read_value_envelope(reader)
    }
}

impl vnext::ReflectedPath {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::reflected_path::Builder<'_>,
    ) -> Result<(), Error> {
        write_reflected_path(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::reflected_path::Reader<'_>) -> Result<Self, Error> {
        read_reflected_path(reader)
    }
}

impl vnext::PrefabValueTarget {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_value_target::Builder<'_>,
    ) -> Result<(), Error> {
        write_value_target(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::prefab_value_target::Reader<'_>,
    ) -> Result<Self, Error> {
        read_value_target(reader)
    }
}

impl vnext::PrefabSourceSnapshot {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_source_snapshot::Builder<'_>,
    ) -> Result<(), Error> {
        write_prefab_source_snapshot(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::prefab_source_snapshot::Reader<'_>,
    ) -> Result<Self, Error> {
        read_prefab_source_snapshot(reader)
    }
}

impl vnext::PrefabOverrideOperation {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_override_operation::Builder<'_>,
    ) -> Result<(), Error> {
        write_prefab_override_operation(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::prefab_override_operation::Reader<'_>,
    ) -> Result<Self, Error> {
        read_prefab_override_operation(reader)
    }
}

impl vnext::PrefabEditCommand {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_edit_command::Builder<'_>,
    ) -> Result<(), Error> {
        write_prefab_edit_command(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::prefab_edit_command::Reader<'_>,
    ) -> Result<Self, Error> {
        read_prefab_edit_command(reader)
    }
}

impl vnext::PrefabDiagnostic {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_diagnostic::Builder<'_>,
    ) -> Result<(), Error> {
        write_prefab_diagnostic(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::prefab_diagnostic::Reader<'_>) -> Result<Self, Error> {
        read_prefab_diagnostic(reader)
    }
}

impl vnext::PrefabRpcResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::prefab_rpc_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_prefab_rpc_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(reader: project_capnp::prefab_rpc_result::Reader<'_>) -> Result<Self, Error> {
        read_prefab_rpc_result(reader)
    }
}

impl vnext::TypedActionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::typed_action_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_typed_action_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::typed_action_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_typed_action_result(reader)
    }
}

impl vnext::SourceSessionStatus {
    pub fn to_capnp(&self, builder: project_capnp::source_session_status::Builder<'_>) {
        write_source_session_status(self, builder);
    }
    #[must_use]
    pub fn from_capnp(reader: project_capnp::source_session_status::Reader<'_>) -> Self {
        read_source_session_status(reader)
    }
}

impl vnext::SourceSessionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        builder: project_capnp::source_session_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_session_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::source_session_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_session_result(reader)
    }
}

impl vnext::SourceAuthoringSessionCommand {
    fn to_capnp(
        &self,
        mut builder: project_capnp::source_authoring_session_command::Builder<'_>,
    ) -> Result<(), Error> {
        match self {
            Self::Open => builder.set_open(()),
            Self::Apply(operation) => {
                operation.to_capnp(builder.reborrow().init_apply().init_operation())?;
            }
            Self::Undo => builder.set_undo(()),
            Self::Redo => builder.set_redo(()),
            Self::Close => builder.set_close(()),
            Self::Status => builder.set_status(()),
        }
        Ok(())
    }

    fn from_capnp(
        reader: project_capnp::source_authoring_session_command::Reader<'_>,
    ) -> Result<Self, Error> {
        Ok(match reader.which()? {
            project_capnp::source_authoring_session_command::Open(()) => Self::Open,
            project_capnp::source_authoring_session_command::Apply(operation) => Self::Apply(
                az_proto_asset::SourceFileEditOperation::from_capnp(operation?.get_operation()?)?,
            ),
            project_capnp::source_authoring_session_command::Undo(()) => Self::Undo,
            project_capnp::source_authoring_session_command::Redo(()) => Self::Redo,
            project_capnp::source_authoring_session_command::Close(()) => Self::Close,
            project_capnp::source_authoring_session_command::Status(()) => Self::Status,
        })
    }
}

impl vnext::SourceAuthoringSessionRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        mut builder: project_capnp::source_authoring_session_request::Builder<'_>,
    ) -> Result<(), Error> {
        Capability::to_capnp(&self.capability, builder.reborrow().init_capability())?;
        builder.set_session_id(&self.session_id);
        self.source.to_capnp(builder.reborrow().init_source())?;
        builder.set_expected_revision(self.expected_revision);
        self.command.to_capnp(builder.init_command())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::source_authoring_session_request::Reader<'_>,
    ) -> Result<Self, Error> {
        Ok(Self {
            capability: Capability::from_capnp(reader.get_capability()?)?,
            session_id: reader.get_session_id()?.to_string()?,
            source: az_proto_asset::WorkspaceSourceFileRef::from_capnp(reader.get_source()?)?,
            expected_revision: reader.get_expected_revision(),
            command: vnext::SourceAuthoringSessionCommand::from_capnp(reader.get_command()?)?,
        })
    }
}

impl vnext::SourceAuthoringSessionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the vnext.
    pub fn to_capnp(
        &self,
        mut builder: project_capnp::source_authoring_session_result::Builder<'_>,
    ) -> Result<(), Error> {
        let mut status = builder.reborrow().init_status();
        status.set_open(self.status.open);
        status.set_revision(self.status.revision);
        status.set_undo_depth(self.status.undo_depth);
        status.set_redo_depth(self.status.redo_depth);
        match &self.outcome {
            vnext::SourceAuthoringSessionOutcome::Snapshot(snapshot) => {
                snapshot.to_capnp(builder.init_snapshot())?;
            }
            vnext::SourceAuthoringSessionOutcome::Closed => builder.set_closed(()),
            vnext::SourceAuthoringSessionOutcome::Failure(failure) => {
                let mut target = builder.init_failure();
                target.set_code(match failure.code {
                    vnext::SourceAuthoringFailureCode::Unavailable => {
                        project_capnp::SourceAuthoringFailureCode::Unavailable
                    }
                    vnext::SourceAuthoringFailureCode::NotOpen => {
                        project_capnp::SourceAuthoringFailureCode::NotOpen
                    }
                    vnext::SourceAuthoringFailureCode::RevisionConflict => {
                        project_capnp::SourceAuthoringFailureCode::RevisionConflict
                    }
                    vnext::SourceAuthoringFailureCode::HistoryEmpty => {
                        project_capnp::SourceAuthoringFailureCode::HistoryEmpty
                    }
                    vnext::SourceAuthoringFailureCode::Transaction => {
                        project_capnp::SourceAuthoringFailureCode::Transaction
                    }
                    vnext::SourceAuthoringFailureCode::SourceMismatch => {
                        project_capnp::SourceAuthoringFailureCode::SourceMismatch
                    }
                });
                target.set_detail(&failure.detail);
                target.set_expected_revision(failure.expected_revision);
                target.set_current_revision(failure.current_revision);
            }
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the vnext is absent from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: project_capnp::source_authoring_session_result::Reader<'_>,
    ) -> Result<Self, Error> {
        let status = reader.get_status()?;
        let outcome = match reader.which()? {
            project_capnp::source_authoring_session_result::Snapshot(snapshot) => {
                vnext::SourceAuthoringSessionOutcome::Snapshot(
                    az_proto_asset::SourceFileEditSnapshot::from_capnp(snapshot?)?,
                )
            }
            project_capnp::source_authoring_session_result::Failure(failure) => {
                let failure = failure?;
                let code = match failure.get_code()? {
                    project_capnp::SourceAuthoringFailureCode::Unavailable => {
                        vnext::SourceAuthoringFailureCode::Unavailable
                    }
                    project_capnp::SourceAuthoringFailureCode::NotOpen => {
                        vnext::SourceAuthoringFailureCode::NotOpen
                    }
                    project_capnp::SourceAuthoringFailureCode::RevisionConflict => {
                        vnext::SourceAuthoringFailureCode::RevisionConflict
                    }
                    project_capnp::SourceAuthoringFailureCode::HistoryEmpty => {
                        vnext::SourceAuthoringFailureCode::HistoryEmpty
                    }
                    project_capnp::SourceAuthoringFailureCode::Transaction => {
                        vnext::SourceAuthoringFailureCode::Transaction
                    }
                    project_capnp::SourceAuthoringFailureCode::SourceMismatch => {
                        vnext::SourceAuthoringFailureCode::SourceMismatch
                    }
                };
                vnext::SourceAuthoringSessionOutcome::Failure(vnext::SourceAuthoringFailure {
                    code,
                    detail: failure.get_detail()?.to_string()?,
                    expected_revision: failure.get_expected_revision(),
                    current_revision: failure.get_current_revision(),
                })
            }
            project_capnp::source_authoring_session_result::Closed(()) => {
                vnext::SourceAuthoringSessionOutcome::Closed
            }
        };
        Ok(Self {
            status: vnext::SourceAuthoringSessionStatus {
                open: status.get_open(),
                revision: status.get_revision(),
                undo_depth: status.get_undo_depth(),
                redo_depth: status.get_redo_depth(),
            },
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes() -> vnext::EditorAttributes {
        vnext::EditorAttributes {
            label: Some(String::new()),
            description: Some("description".to_owned()),
            category: Some("category".to_owned()),
            icon: Some("icon".to_owned()),
            widget: Some("slider".to_owned()),
            range: Some(vnext::NumericRange {
                minimum: Some("0".to_owned()),
                maximum: Some("10".to_owned()),
                step: Some("0.5".to_owned()),
                suffix: Some("m".to_owned()),
            }),
            read_only: true,
            hidden: true,
            action_ids: vec!["reset".to_owned()],
            constraints: vnext::FieldConstraints {
                minimum_length: Some(1),
                maximum_length: Some(32),
                allowed_strings: vec!["Alpha".to_owned(), "Beta".to_owned()],
                allowed_variants: vec!["Alpha".to_owned()],
            },
        }
    }

    fn envelope(type_path: &str, payload: &[u8]) -> vnext::ReflectedValueEnvelope {
        vnext::ReflectedValueEnvelope {
            type_path: type_path.to_owned(),
            encoding: vnext::ReflectedValueEncoding::TypedRon,
            payload: payload.to_vec(),
        }
    }

    fn target() -> vnext::PrefabValueTarget {
        vnext::PrefabValueTarget {
            instance_alias_chain: vec!["outer".to_owned(), "inner".to_owned()],
            entity_alias: "entity".to_owned(),
            path: vnext::ReflectedPath {
                component_type_path: "fixture::Component".to_owned(),
                segments: vec![
                    vnext::ReflectedPathSegment::Field("field".to_owned()),
                    vnext::ReflectedPathSegment::Variant("Variant".to_owned()),
                    vnext::ReflectedPathSegment::TupleIndex(2),
                    vnext::ReflectedPathSegment::ListIndex(3),
                ],
            },
        }
    }

    fn snapshot() -> vnext::PrefabSourceSnapshot {
        vnext::PrefabSourceSnapshot {
            document_version: 1,
            type_versions: BTreeMap::from([("fixture::Component".to_owned(), 2)]),
            entities: vec![vnext::PrefabEntitySnapshot {
                alias: "entity".to_owned(),
            }],
            hierarchy: vec![vnext::PrefabHierarchyEdge {
                child_alias: "entity".to_owned(),
                parent_alias: Some(String::new()),
            }],
            components: vec![vnext::PrefabComponentSnapshot {
                entity_alias: "entity".to_owned(),
                type_path: "fixture::Component".to_owned(),
                sparse_value: envelope("fixture::Component", b"(value: 1)"),
            }],
            instances: vec![vnext::PrefabInstanceSnapshot {
                alias: "instance".to_owned(),
                source_asset: "prefabs/base.prefab.ron".to_owned(),
                parent_entity_alias: None,
            }],
            overrides: vec![
                vnext::PrefabOverrideSnapshot {
                    operation: vnext::PrefabOverrideOperation::Set {
                        target: target(),
                        value: envelope("f32", b"2.0"),
                    },
                },
                vnext::PrefabOverrideSnapshot {
                    operation: vnext::PrefabOverrideOperation::Clear { target: target() },
                },
                vnext::PrefabOverrideSnapshot {
                    operation: vnext::PrefabOverrideOperation::Insert {
                        target: target(),
                        index: 1,
                        value: envelope("f32", b"3.0"),
                    },
                },
                vnext::PrefabOverrideSnapshot {
                    operation: vnext::PrefabOverrideOperation::Remove {
                        target: target(),
                        index: 2,
                    },
                },
                vnext::PrefabOverrideSnapshot {
                    operation: vnext::PrefabOverrideOperation::Move {
                        target: target(),
                        from: 3,
                        to: 4,
                    },
                },
            ],
            revision: 9,
        }
    }

    #[test]
    fn vnext_type_registry_capnp_round_trip_covers_every_reflected_kind() {
        let kinds = vec![
            vnext::ReflectedTypeKind::Struct,
            vnext::ReflectedTypeKind::TupleStruct,
            vnext::ReflectedTypeKind::Tuple,
            vnext::ReflectedTypeKind::List,
            vnext::ReflectedTypeKind::Array { capacity: 17 },
            vnext::ReflectedTypeKind::Map,
            vnext::ReflectedTypeKind::Set,
            vnext::ReflectedTypeKind::Enum,
            vnext::ReflectedTypeKind::Optional,
            vnext::ReflectedTypeKind::Bool,
            vnext::ReflectedTypeKind::SignedInteger { bits: 64 },
            vnext::ReflectedTypeKind::UnsignedInteger { bits: 128 },
            vnext::ReflectedTypeKind::Float { bits: 32 },
            vnext::ReflectedTypeKind::String,
            vnext::ReflectedTypeKind::Opaque,
        ];
        let snapshot = vnext::TypeRegistrySnapshot {
            schema_catalog_hash: vec![0x42; 32],
            types: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| vnext::ReflectedTypeDescriptor {
                    type_path: format!("fixture::Type{index}"),
                    short_path: format!("Type{index}"),
                    kind,
                    fields: vec![vnext::ReflectedFieldDescriptor {
                        name: "value".to_owned(),
                        type_path: "f32".to_owned(),
                        editor_attributes: attributes(),
                    }],
                    variants: vec![vnext::ReflectedVariantDescriptor {
                        name: "Variant".to_owned(),
                        fields: Vec::new(),
                        editor_attributes: attributes(),
                    }],
                    editor_attributes: attributes(),
                    type_data_flags: vec!["Prefab".to_owned(), "Validation".to_owned()],
                    applicability: vnext::ApplicabilityDescriptor {
                        provides: vec!["fixture.value".to_owned()],
                        requires: vec!["fixture.parent".to_owned()],
                        incompatible: vec!["fixture.value".to_owned()],
                        default_available: true,
                    },
                    reflected_default: Some(envelope(
                        &format!("fixture::Type{index}"),
                        b"(value: 1)",
                    )),
                })
                .collect(),
        };

        assert_eq!(
            round_trip_type_registry_snapshot(&snapshot).expect("round-trip registry"),
            snapshot
        );
    }

    #[test]
    fn vnext_prefab_snapshot_capnp_round_trip_preserves_sparse_authoring() {
        let snapshot = snapshot();
        assert_eq!(
            round_trip_prefab_source_snapshot(&snapshot).expect("round-trip Prefab snapshot"),
            snapshot
        );
    }

    #[test]
    fn vnext_prefab_edit_command_capnp_round_trip_covers_every_variant() {
        let value = envelope("f32", b"1.5");
        let commands = vec![
            vnext::PrefabEditCommand::SetValue {
                target: target(),
                value: value.clone(),
            },
            vnext::PrefabEditCommand::ListInsert {
                target: target(),
                index: 1,
                value: value.clone(),
            },
            vnext::PrefabEditCommand::ListRemove {
                target: target(),
                index: 2,
            },
            vnext::PrefabEditCommand::ListMove {
                target: target(),
                from: 1,
                to: 3,
            },
            vnext::PrefabEditCommand::MapInsert {
                target: target(),
                key: envelope("String", b"\"key\""),
                value: value.clone(),
            },
            vnext::PrefabEditCommand::MapRemove {
                target: target(),
                key: envelope("String", b"\"key\""),
            },
            vnext::PrefabEditCommand::SetVariant {
                target: target(),
                variant_name: "Some".to_owned(),
                value: Some(value.clone()),
            },
            vnext::PrefabEditCommand::SetVariant {
                target: target(),
                variant_name: "None".to_owned(),
                value: None,
            },
            vnext::PrefabEditCommand::AddComponent {
                entity_alias: "entity".to_owned(),
                component_type_path: "fixture::Component".to_owned(),
                initial_value: Some(value.clone()),
            },
            vnext::PrefabEditCommand::RemoveComponent {
                entity_alias: "entity".to_owned(),
                component_type_path: "fixture::Component".to_owned(),
            },
            vnext::PrefabEditCommand::AddEntity {
                alias: "child".to_owned(),
                parent_alias: Some("entity".to_owned()),
            },
            vnext::PrefabEditCommand::RemoveEntity {
                alias: "child".to_owned(),
            },
            vnext::PrefabEditCommand::ReparentEntity {
                alias: "child".to_owned(),
                parent_alias: None,
            },
            vnext::PrefabEditCommand::AddInstance {
                alias: "instance".to_owned(),
                source_asset: "prefabs/base.prefab.ron".to_owned(),
                parent_entity_alias: Some("entity".to_owned()),
            },
            vnext::PrefabEditCommand::RemoveInstance {
                alias: "instance".to_owned(),
            },
            vnext::PrefabEditCommand::ReparentInstance {
                alias: "instance".to_owned(),
                parent_entity_alias: None,
            },
            vnext::PrefabEditCommand::SetOverride {
                target: target(),
                value: value.clone(),
            },
            vnext::PrefabEditCommand::ClearOverride { target: target() },
            vnext::PrefabEditCommand::InsertOverride {
                target: target(),
                index: 1,
                value,
            },
            vnext::PrefabEditCommand::RemoveOverrideItem {
                target: target(),
                index: 2,
            },
            vnext::PrefabEditCommand::MoveOverride {
                target: target(),
                from: 1,
                to: 3,
            },
            vnext::PrefabEditCommand::RemoveOverride { target: target() },
        ];

        for command in commands {
            assert_eq!(
                round_trip_prefab_edit_command(&command).expect("round-trip command"),
                command
            );
        }
    }

    #[test]
    fn vnext_rpc_support_types_capnp_round_trip() {
        let diagnostics = [
            vnext::DiagnosticSeverity::Info,
            vnext::DiagnosticSeverity::Warning,
            vnext::DiagnosticSeverity::Error,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, severity)| vnext::PrefabDiagnostic {
            severity,
            code: format!("fixture.{index}"),
            message: format!("diagnostic {index}"),
            target: (index != 0).then(target),
        })
        .collect::<Vec<_>>();
        let prefab_result = vnext::PrefabRpcResult {
            snapshot: Some(snapshot()),
            diagnostics: diagnostics.clone(),
        };
        assert_eq!(
            round_trip_prefab_rpc_result(&prefab_result).expect("round-trip Prefab result"),
            prefab_result,
        );
        let empty_prefab_result = vnext::PrefabRpcResult {
            snapshot: None,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            round_trip_prefab_rpc_result(&empty_prefab_result)
                .expect("round-trip empty Prefab result"),
            empty_prefab_result,
        );

        let action = vnext::TypedActionResult {
            snapshot: Some(snapshot()),
            changed_paths: vec![target().path],
            diagnostics: diagnostics.clone(),
        };
        assert_eq!(
            round_trip_typed_action_result(&action).expect("round-trip action result"),
            action,
        );

        let session = vnext::SourceSessionResult {
            status: vnext::SourceSessionStatus {
                open: true,
                revision: 7,
                dirty: true,
                undo_depth: 2,
                redo_depth: 1,
            },
            snapshot: Some(snapshot()),
            diagnostics,
        };
        assert_eq!(
            round_trip_source_session_result(&session).expect("round-trip session result"),
            session,
        );

        for command in [
            vnext::SourceSessionCommand::Open,
            vnext::SourceSessionCommand::Save,
            vnext::SourceSessionCommand::SaveRecovery,
            vnext::SourceSessionCommand::Undo,
            vnext::SourceSessionCommand::Redo,
            vnext::SourceSessionCommand::Close,
            vnext::SourceSessionCommand::Status,
        ] {
            assert_eq!(
                vnext::SourceSessionCommand::from_capnp(command.to_capnp()),
                command
            );
        }
    }
}
