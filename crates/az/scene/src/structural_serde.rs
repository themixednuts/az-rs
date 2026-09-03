//! Canonical structural wire ownership for named reflected records.
//!
//! AZSCENE is an engine product format, so named reflected structs and enums
//! use their Bevy type information on both sides of the wire. A registered
//! `ReflectSerialize`/`ReflectDeserialize` adapter may intentionally describe
//! another format (for example legacy `ObjectStream` integer enums or tagged
//! records); it must not silently replace the AZSCENE schema. Opaque leaves
//! and non-record containers continue through their registered adapters.

use std::fmt;

use bevy::reflect::{
    PartialReflect, ReflectRef, TypeInfo, TypeRegistration, TypeRegistry,
    enums::{
        DynamicEnum, DynamicVariant, Enum as ReflectEnum, EnumInfo, StructVariantInfo,
        TupleVariantInfo, VariantInfo, VariantType,
    },
    serde::{
        ReflectDeserializerProcessor, ReflectSerializerProcessor, SerializationData,
        TypedReflectDeserializer, TypedReflectSerializer,
    },
    structs::{DynamicStruct, Struct as ReflectStruct, StructInfo},
    tuple::DynamicTuple,
};
use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, EnumAccess, Error as _, MapAccess, SeqAccess, VariantAccess, Visitor},
    ser::{Error as _, SerializeStruct, SerializeStructVariant, SerializeTupleVariant},
};

pub fn try_serialize<S, P>(
    value: &dyn PartialReflect,
    registry: &TypeRegistry,
    processor: &P,
    serializer: S,
) -> Result<Result<S::Ok, S>, S::Error>
where
    S: serde::Serializer,
    P: ReflectSerializerProcessor,
{
    match value.reflect_ref() {
        ReflectRef::Struct(value) => StructuralStructSerializer {
            value,
            registry,
            processor,
        }
        .serialize(serializer)
        .map(Ok),
        ReflectRef::Enum(value) => StructuralEnumSerializer {
            value,
            registry,
            processor,
        }
        .serialize(serializer)
        .map(Ok),
        _ => Ok(Err(serializer)),
    }
}

struct StructuralStructSerializer<'a, P> {
    value: &'a dyn ReflectStruct,
    registry: &'a TypeRegistry,
    processor: &'a P,
}

impl<P: ReflectSerializerProcessor> Serialize for StructuralStructSerializer<'_, P> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let info = self
            .value
            .get_represented_type_info()
            .and_then(|info| info.as_struct().ok())
            .ok_or_else(|| S::Error::custom("AZSCENE struct has no represented StructInfo"))?;
        let serialization = self
            .registry
            .get(info.type_id())
            .and_then(|registration| registration.data::<SerializationData>());
        let skipped = serialization
            .map(SerializationData::len)
            .unwrap_or_default();
        let mut state = serializer.serialize_struct(
            info.type_path_table().ident().unwrap_or("AzSceneStruct"),
            self.value.field_len().saturating_sub(skipped),
        )?;
        for (index, field) in info.iter().enumerate() {
            if serialization.is_some_and(|data| data.is_field_skipped(index)) {
                continue;
            }
            let field_value = self.value.field(field.name()).ok_or_else(|| {
                S::Error::custom(format!(
                    "AZSCENE struct is missing field `{}`",
                    field.name()
                ))
            })?;
            state.serialize_field(
                field.name(),
                &TypedReflectSerializer::with_processor(field_value, self.registry, self.processor),
            )?;
        }
        state.end()
    }
}

struct StructuralEnumSerializer<'a, P> {
    value: &'a dyn ReflectEnum,
    registry: &'a TypeRegistry,
    processor: &'a P,
}

impl<P: ReflectSerializerProcessor> Serialize for StructuralEnumSerializer<'_, P> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let info = self
            .value
            .get_represented_type_info()
            .and_then(|info| info.as_enum().ok())
            .ok_or_else(|| S::Error::custom("AZSCENE enum has no represented EnumInfo"))?;
        let variant = info.variant(self.value.variant_name()).ok_or_else(|| {
            S::Error::custom(format!(
                "AZSCENE enum has unknown variant `{}`",
                self.value.variant_name()
            ))
        })?;
        let index = u32::try_from(self.value.variant_index())
            .map_err(|_| S::Error::custom("AZSCENE enum variant index exceeds u32"))?;
        let enum_name = info.type_path_table().ident().unwrap_or("AzSceneEnum");
        match self.value.variant_type() {
            VariantType::Unit if is_option(info) => serializer.serialize_none(),
            VariantType::Unit => {
                serializer.serialize_unit_variant(enum_name, index, variant.name())
            }
            VariantType::Tuple if self.value.field_len() == 1 && is_option(info) => serializer
                .serialize_some(&TypedReflectSerializer::with_processor(
                    self.value.field_at(0).expect("one Option::Some field"),
                    self.registry,
                    self.processor,
                )),
            VariantType::Tuple if self.value.field_len() == 1 => serializer
                .serialize_newtype_variant(
                    enum_name,
                    index,
                    variant.name(),
                    &TypedReflectSerializer::with_processor(
                        self.value.field_at(0).expect("one tuple field"),
                        self.registry,
                        self.processor,
                    ),
                ),
            VariantType::Tuple => {
                let mut state = serializer.serialize_tuple_variant(
                    enum_name,
                    index,
                    variant.name(),
                    self.value.field_len(),
                )?;
                for field in self.value.iter_fields() {
                    state.serialize_field(&TypedReflectSerializer::with_processor(
                        field.value(),
                        self.registry,
                        self.processor,
                    ))?;
                }
                state.end()
            }
            VariantType::Struct => {
                let VariantInfo::Struct(variant) = variant else {
                    return Err(S::Error::custom("AZSCENE enum variant shape mismatch"));
                };
                let mut state = serializer.serialize_struct_variant(
                    enum_name,
                    index,
                    variant.name(),
                    self.value.field_len(),
                )?;
                for field in variant.iter() {
                    let field_value = self.value.field(field.name()).ok_or_else(|| {
                        S::Error::custom(format!(
                            "AZSCENE struct variant is missing field `{}`",
                            field.name()
                        ))
                    })?;
                    state.serialize_field(
                        field.name(),
                        &TypedReflectSerializer::with_processor(
                            field_value,
                            self.registry,
                            self.processor,
                        ),
                    )?;
                }
                state.end()
            }
        }
    }
}

pub fn try_deserialize<'de, D, P>(
    registration: &TypeRegistration,
    registry: &TypeRegistry,
    processor: &mut P,
    deserializer: D,
) -> Result<Result<Box<dyn PartialReflect>, D>, D::Error>
where
    D: serde::Deserializer<'de>,
    P: ReflectDeserializerProcessor,
{
    let value: Box<dyn PartialReflect> = match registration.type_info() {
        TypeInfo::Struct(info) => {
            let mut value = deserializer.deserialize_struct(
                info.type_path_table().ident().unwrap_or("AzSceneStruct"),
                info.field_names(),
                StructVisitor {
                    info,
                    registration,
                    registry,
                    processor,
                },
            )?;
            value.set_represented_type(Some(registration.type_info()));
            Box::new(value)
        }
        TypeInfo::Enum(info) => {
            let mut value = if is_option(info) {
                deserializer.deserialize_option(OptionVisitor {
                    info,
                    registry,
                    processor,
                })?
            } else {
                deserializer.deserialize_enum(
                    info.type_path_table().ident().unwrap_or("AzSceneEnum"),
                    info.variant_names(),
                    EnumVisitor {
                        info,
                        registration,
                        registry,
                        processor,
                    },
                )?
            };
            value.set_represented_type(Some(registration.type_info()));
            Box::new(value)
        }
        _ => return Ok(Err(deserializer)),
    };
    Ok(Ok(value))
}

fn is_option(info: &EnumInfo) -> bool {
    info.type_path_table().module_path() == Some("core::option")
        && info.type_path_table().ident() == Some("Option")
}

struct StructVisitor<'a, P> {
    info: &'static StructInfo,
    registration: &'a TypeRegistration,
    registry: &'a TypeRegistry,
    processor: &'a mut P,
}

impl<'de, P: ReflectDeserializerProcessor> Visitor<'de> for StructVisitor<'_, P> {
    type Value = DynamicStruct;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an AZSCENE reflected struct")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        visit_struct_sequence(
            &mut sequence,
            self.info,
            self.registration,
            self.registry,
            self.processor,
        )
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut value = DynamicStruct::default();
        while let Some(FieldName(name)) = map.next_key::<FieldName>()? {
            if value.field(&name).is_some() {
                return Err(A::Error::custom(format!("duplicate field `{name}`")));
            }
            let field = self.info.field(&name).ok_or_else(|| {
                A::Error::custom(format!(
                    "unknown field `{name}` on `{}`",
                    self.info.type_path()
                ))
            })?;
            let field_value = map.next_value_seed(TypedReflectDeserializer::with_processor(
                require_registration::<A::Error>(field.type_id(), self.registry)?,
                self.registry,
                self.processor,
            ))?;
            value.insert_boxed(name, field_value);
        }
        insert_skipped_fields(&mut value, self.info, self.registration);
        Ok(value)
    }
}

fn visit_struct_sequence<'de, A>(
    sequence: &mut A,
    info: &'static StructInfo,
    registration: &TypeRegistration,
    registry: &TypeRegistry,
    processor: &mut impl ReflectDeserializerProcessor,
) -> Result<DynamicStruct, A::Error>
where
    A: SeqAccess<'de>,
{
    let mut value = DynamicStruct::default();
    let serialization = registration.data::<SerializationData>();
    for (index, field) in info.iter().enumerate() {
        if serialization.is_some_and(|data| data.is_field_skipped(index)) {
            if let Some(default) = serialization.and_then(|data| data.generate_default(index)) {
                value.insert_boxed(field.name(), default.into_partial_reflect());
            }
            continue;
        }
        let field_value = sequence
            .next_element_seed(TypedReflectDeserializer::with_processor(
                require_registration::<A::Error>(field.type_id(), registry)?,
                registry,
                processor,
            ))?
            .ok_or_else(|| A::Error::invalid_length(index, &"all reflected struct fields"))?;
        value.insert_boxed(field.name(), field_value);
    }
    Ok(value)
}

fn insert_skipped_fields(
    value: &mut DynamicStruct,
    info: &'static StructInfo,
    registration: &TypeRegistration,
) {
    let Some(serialization) = registration.data::<SerializationData>() else {
        return;
    };
    for (index, skipped) in serialization.iter_skipped() {
        if let (Some(field), default) = (info.field_at(*index), skipped.generate_default()) {
            value.insert_boxed(field.name(), default.into_partial_reflect());
        }
    }
}

struct EnumVisitor<'a, P> {
    info: &'static EnumInfo,
    registration: &'a TypeRegistration,
    registry: &'a TypeRegistry,
    processor: &'a mut P,
}

impl<'de, P: ReflectDeserializerProcessor> Visitor<'de> for EnumVisitor<'_, P> {
    type Value = DynamicEnum;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an AZSCENE reflected enum")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (variant, access) = data.variant_seed(VariantSeed { info: self.info })?;
        let dynamic = match variant {
            VariantInfo::Unit(_) => {
                access.unit_variant()?;
                DynamicVariant::Unit
            }
            VariantInfo::Struct(info) => DynamicVariant::Struct(access.struct_variant(
                info.field_names(),
                StructVariantVisitor {
                    info,
                    registration: self.registration,
                    registry: self.registry,
                    processor: self.processor,
                },
            )?),
            VariantInfo::Tuple(info) if info.field_len() == 1 => {
                let field = info.field_at(0).expect("single tuple variant field");
                let field_value =
                    access.newtype_variant_seed(TypedReflectDeserializer::with_processor(
                        require_registration::<A::Error>(field.type_id(), self.registry)?,
                        self.registry,
                        self.processor,
                    ))?;
                let mut tuple = DynamicTuple::default();
                tuple.insert_boxed(field_value);
                DynamicVariant::Tuple(tuple)
            }
            VariantInfo::Tuple(info) => DynamicVariant::Tuple(access.tuple_variant(
                info.field_len(),
                TupleVariantVisitor {
                    info,
                    registration: self.registration,
                    registry: self.registry,
                    processor: self.processor,
                },
            )?),
        };
        let index = self
            .info
            .index_of(variant.name())
            .expect("resolved variant belongs to its EnumInfo");
        Ok(DynamicEnum::new_with_index(index, variant.name(), dynamic))
    }
}

struct VariantSeed {
    info: &'static EnumInfo,
}

impl<'de> DeserializeSeed<'de> for VariantSeed {
    type Value = &'static VariantInfo;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct VariantVisitor(&'static EnumInfo);

        impl Visitor<'_> for VariantVisitor {
            type Value = &'static VariantInfo;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a registered enum variant")
            }

            fn visit_u32<E>(self, index: u32) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.0
                    .variant_at(index as usize)
                    .ok_or_else(|| E::custom(format!("unknown variant index {index}")))
            }

            fn visit_str<E>(self, name: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.0
                    .variant(name)
                    .ok_or_else(|| E::custom(format!("unknown variant `{name}`")))
            }
        }

        deserializer.deserialize_identifier(VariantVisitor(self.info))
    }
}

struct StructVariantVisitor<'a, P> {
    info: &'static StructVariantInfo,
    registration: &'a TypeRegistration,
    registry: &'a TypeRegistry,
    processor: &'a mut P,
}

impl<'de, P: ReflectDeserializerProcessor> Visitor<'de> for StructVariantVisitor<'_, P> {
    type Value = DynamicStruct;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an AZSCENE reflected struct variant")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut value = DynamicStruct::default();
        let serialization = self.registration.data::<SerializationData>();
        for (index, field) in self.info.iter().enumerate() {
            if serialization.is_some_and(|data| data.is_field_skipped(index)) {
                if let Some(default) = serialization.and_then(|data| data.generate_default(index)) {
                    value.insert_boxed(field.name(), default.into_partial_reflect());
                }
                continue;
            }

            let field_value = sequence
                .next_element_seed(TypedReflectDeserializer::with_processor(
                    require_registration::<A::Error>(field.type_id(), self.registry)?,
                    self.registry,
                    self.processor,
                ))?
                .ok_or_else(|| A::Error::invalid_length(index, &"all struct-variant fields"))?;
            value.insert_boxed(field.name(), field_value);
        }
        Ok(value)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut value = DynamicStruct::default();
        while let Some(FieldName(name)) = map.next_key::<FieldName>()? {
            if value.field(&name).is_some() {
                return Err(A::Error::custom(format!("duplicate field `{name}`")));
            }
            let field = self.info.field(&name).ok_or_else(|| {
                A::Error::custom(format!(
                    "unknown field `{name}` on struct variant `{}`",
                    self.info.name()
                ))
            })?;
            let field_value = map.next_value_seed(TypedReflectDeserializer::with_processor(
                require_registration::<A::Error>(field.type_id(), self.registry)?,
                self.registry,
                self.processor,
            ))?;
            value.insert_boxed(name, field_value);
        }

        if let Some(serialization) = self.registration.data::<SerializationData>() {
            for (index, skipped) in serialization.iter_skipped() {
                if let (Some(field), default) =
                    (self.info.field_at(*index), skipped.generate_default())
                {
                    value.insert_boxed(field.name(), default.into_partial_reflect());
                }
            }
        }
        Ok(value)
    }
}

struct TupleVariantVisitor<'a, P> {
    info: &'static TupleVariantInfo,
    registration: &'a TypeRegistration,
    registry: &'a TypeRegistry,
    processor: &'a mut P,
}

impl<'de, P: ReflectDeserializerProcessor> Visitor<'de> for TupleVariantVisitor<'_, P> {
    type Value = DynamicTuple;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an AZSCENE reflected tuple variant")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut value = DynamicTuple::default();
        let serialization = self.registration.data::<SerializationData>();
        for (index, field) in self.info.iter().enumerate() {
            if let Some(default) = serialization.and_then(|data| data.generate_default(index)) {
                value.insert_boxed(default.into_partial_reflect());
                continue;
            }
            let field_value = sequence
                .next_element_seed(TypedReflectDeserializer::with_processor(
                    require_registration::<A::Error>(field.type_id(), self.registry)?,
                    self.registry,
                    self.processor,
                ))?
                .ok_or_else(|| A::Error::invalid_length(index, &"all tuple-variant fields"))?;
            value.insert_boxed(field_value);
        }
        Ok(value)
    }
}

struct OptionVisitor<'a, P> {
    info: &'static EnumInfo,
    registry: &'a TypeRegistry,
    processor: &'a mut P,
}

impl<'de, P: ReflectDeserializerProcessor> Visitor<'de> for OptionVisitor<'_, P> {
    type Value = DynamicEnum;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an AZSCENE option of type {}",
            self.info.type_path()
        )
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let mut value = DynamicEnum::default();
        value.set_variant("None", ());
        Ok(value)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let VariantInfo::Tuple(info) = self.info.variant("Some").expect("Option::Some exists")
        else {
            return Err(D::Error::custom("Option::Some is not a tuple variant"));
        };
        let field = info.field_at(0).expect("Option::Some has one field");
        let inner = TypedReflectDeserializer::with_processor(
            require_registration::<D::Error>(field.type_id(), self.registry)?,
            self.registry,
            self.processor,
        )
        .deserialize(deserializer)?;
        let mut tuple = DynamicTuple::default();
        tuple.insert_boxed(inner);
        let mut value = DynamicEnum::default();
        value.set_variant("Some", tuple);
        Ok(value)
    }
}

struct FieldName(String);

impl<'de> Deserialize<'de> for FieldName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldNameVisitor;

        impl Visitor<'_> for FieldNameVisitor {
            type Value = FieldName;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a reflected field name")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(FieldName(value.to_owned()))
            }
        }

        deserializer.deserialize_identifier(FieldNameVisitor)
    }
}

fn require_registration<E: serde::de::Error>(
    type_id: std::any::TypeId,
    registry: &TypeRegistry,
) -> Result<&TypeRegistration, E> {
    registry
        .get(type_id)
        .ok_or_else(|| E::custom(format!("unregistered reflected field TypeId {type_id:?}")))
}
