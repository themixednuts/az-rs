//! Canonical Lumberyard `SerializeContext::IDataContainer` families.
//!
//! A serialized type's specialized UUID and its `uuidGenericMap` registration
//! aliases identify an instantiation, not the behavior of its data-container
//! adapter. Resolve either through `GenericClassInfo`, then classify the
//! resulting `GetGenericTypeId()` with [`container_shape_for_generic_uuid`].
//! Unknown generic IDs deliberately remain unclassified.

use uuid::Uuid;

use crate::context::{ContainerCardinality, ContainerShape};

/// `GenericClassInfoVector`.
pub const GENERIC_VECTOR_TYPE_ID: Uuid = Uuid::from_u128(0x2bade35a_6f1b_4698_b2bc_3373d010020c);
/// `GenericClassInfoFixedVector`.
pub const GENERIC_FIXED_VECTOR_TYPE_ID: Uuid =
    Uuid::from_u128(0x6c6751b0_392a_4e71_8bf8_179484d7d22f);
/// `GenericClassInfoList`.
pub const GENERIC_LIST_TYPE_ID: Uuid = Uuid::from_u128(0xb845ad64_b5a0_4ccd_a86b_3477a36779be);
/// `GenericClassInfoForwardList`.
pub const GENERIC_FORWARD_LIST_TYPE_ID: Uuid =
    Uuid::from_u128(0xd48e20e1_d3a3_46f7_a869_927f5ef50127);
/// `GenericClassInfoArray`.
pub const GENERIC_ARRAY_TYPE_ID: Uuid = Uuid::from_u128(0x286e1198_0867_4198_95d3_6cc569658e07);

/// `GenericClassSet`.
pub const GENERIC_SET_TYPE_ID: Uuid = Uuid::from_u128(0x4a64d2a5_7265_4e3d_805c_ba2d0626f542);
/// `GenericClassUnorderedSet`.
pub const GENERIC_UNORDERED_SET_TYPE_ID: Uuid =
    Uuid::from_u128(0xb04e902e_c6f7_4212_a166_1b52f7437d3c);
/// `GenericClassUnorderedMultiSet`.
pub const GENERIC_UNORDERED_MULTISET_TYPE_ID: Uuid =
    Uuid::from_u128(0xb619da0d_f050_41aa_b092_416cacb7c710);
/// `GenericClassMap`.
pub const GENERIC_MAP_TYPE_ID: Uuid = Uuid::from_u128(0xdb825311_453d_45c8_b07f_b9cd9a32acb4);
/// `GenericClassUnorderedMap`.
pub const GENERIC_UNORDERED_MAP_TYPE_ID: Uuid =
    Uuid::from_u128(0x18456a80_63cc_40c5_bf16_6af94f9a9ecc);
/// `GenericClassUnorderedMultiMap`.
pub const GENERIC_UNORDERED_MULTIMAP_TYPE_ID: Uuid =
    Uuid::from_u128(0x119669b8_92bf_468f_97d9_9d45f298bcd4);
/// `GenericClassUnorderedFlatMap` (`AZStd::unordered_flat_map`).
///
/// Lumberyard's associative-pair flat map. Its `GetGenericTypeId()` differs
/// from the node-based `unordered_map`, but its captured `ClassData` carries the
/// same generic-level `AZStd::pair` element (`GENERIC_PAIR_TYPE_ID`), so it
/// decodes through the identical [`ContainerShape::Map`] associative path.
pub const GENERIC_UNORDERED_FLAT_MAP_TYPE_ID: Uuid =
    Uuid::from_u128(0x5aa0cb76_f01c_4c17_9fb0_21acee30a5d5);

/// `GenericClassPair`.
pub const GENERIC_PAIR_TYPE_ID: Uuid = Uuid::from_u128(0x9f3f5302_3390_407a_a6f7_2e011e3bb686);
/// `GenericClassTuple`.
pub const GENERIC_TUPLE_TYPE_ID: Uuid = Uuid::from_u128(0xf98df943_f870_4fe2_b6a9_3e8bc5861782);

/// `GenericClassSharedPtr`.
pub const GENERIC_SHARED_PTR_TYPE_ID: Uuid =
    Uuid::from_u128(0xd5b5aca6_a81e_410e_8151_80c97b8cd2a0);
/// `GenericClassIntrusivePtr`.
pub const GENERIC_INTRUSIVE_PTR_TYPE_ID: Uuid =
    Uuid::from_u128(0xc4b5400b_5cdc_4b14_932e_bfa30bc1de35);
/// `GenericClassUniquePtr`.
pub const GENERIC_UNIQUE_PTR_TYPE_ID: Uuid =
    Uuid::from_u128(0xd37df835_5b18_4f9e_9c8f_03b967483080);
/// `GenericClassOptional`.
pub const GENERIC_OPTIONAL_TYPE_ID: Uuid = Uuid::from_u128(0x2c75b779_4661_4102_9d40_b2a680b6de36);
/// `GenericClassVariant`.
pub const GENERIC_VARIANT_TYPE_ID: Uuid = Uuid::from_u128(0x1e8bb1e5_410a_4367_8faa_d43a4de14d4b);

/// `ClassData` ID hardcoded by `GenericClassWrapper`.
///
/// This family has no canonical generic UUID: its inherited
/// `GetGenericTypeId()` returns the per-`T` specialized UUID while its
/// `ClassData` uses this fixed ID.
pub const RVALUE_WRAPPER_CLASS_DATA_TYPE_ID: Uuid =
    Uuid::from_u128(0x642aba5e_bb70_40ef_a986_933420d89f85);

/// Native fixed-size/fixed-capacity behavior of an `IDataContainer` family.
///
/// A [`FixedCapacity`](Self::FixedCapacity) or
/// [`FixedSize`](Self::FixedSize) family still needs its captured numeric
/// capacity. Lumberyard exposes that through the adapter; reflection capture
/// schemas can also retain the non-type template argument for `fixed_vector`
/// and `array`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerCapacityKind {
    Dynamic,
    FixedCapacity,
    FixedSize,
}

/// Classify one native `GenericClassInfo::GetGenericTypeId()` value.
///
/// These UUIDs come from the corresponding O3DE
/// `SerializeGenericTypeInfo` specializations in
/// `Code/Framework/AzCore/AzCore/Serialization/AZStdContainers.inl`.
/// Their `ClassData` is constructed with one of the native
/// `AZStdBasicContainer`, `AZStdAssociativeContainer`,
/// `AZStdPairContainer`, `AZStdTupleContainer`, `AZStdSmartPtrContainer`,
/// `AZStdOptionalContainer`, or `AZStdVariantContainer` adapter families.
/// Lumberyard's
/// `dev/Code/Framework/AzCore/AzCore/Serialization/Utils.cpp` uses this same
/// `FindGenericClassInfo` -> `GetGenericTypeId` policy for its built-in
/// vector, set, map, and pair predicates.
///
/// This does not accept specialized UUIDs or registration aliases directly.
/// Callers must resolve those to `GenericClassInfo` first. `None` means the
/// native adapter family was not captured by this canonical registry.
#[must_use]
pub fn container_shape_for_generic_uuid(generic_type_id: Uuid) -> Option<ContainerShape> {
    match generic_type_id {
        id if id == GENERIC_VECTOR_TYPE_ID
            || id == GENERIC_FIXED_VECTOR_TYPE_ID
            || id == GENERIC_LIST_TYPE_ID
            || id == GENERIC_FORWARD_LIST_TYPE_ID
            || id == GENERIC_ARRAY_TYPE_ID =>
        {
            Some(ContainerShape::Sequence)
        }
        id if id == GENERIC_SET_TYPE_ID
            || id == GENERIC_UNORDERED_SET_TYPE_ID
            || id == GENERIC_UNORDERED_MULTISET_TYPE_ID =>
        {
            Some(ContainerShape::Set)
        }
        id if id == GENERIC_MAP_TYPE_ID
            || id == GENERIC_UNORDERED_MAP_TYPE_ID
            || id == GENERIC_UNORDERED_MULTIMAP_TYPE_ID
            || id == GENERIC_UNORDERED_FLAT_MAP_TYPE_ID =>
        {
            Some(ContainerShape::Map)
        }
        id if id == GENERIC_PAIR_TYPE_ID => Some(ContainerShape::Pair),
        id if id == GENERIC_TUPLE_TYPE_ID => Some(ContainerShape::Tuple),
        id if id == GENERIC_SHARED_PTR_TYPE_ID
            || id == GENERIC_INTRUSIVE_PTR_TYPE_ID
            || id == GENERIC_UNIQUE_PTR_TYPE_ID =>
        {
            Some(ContainerShape::SmartPointer)
        }
        id if id == GENERIC_OPTIONAL_TYPE_ID => Some(ContainerShape::Optional),
        id if id == GENERIC_VARIANT_TYPE_ID => Some(ContainerShape::Variant),
        _ => None,
    }
}

/// Classify a native family whose canonical evidence is the `ClassData` UUID.
///
/// Standard `AZStd` containers use [`container_shape_for_generic_uuid`].
/// Lumberyard's internal rvalue wrapper is the exception because its
/// `GenericClassInfo` deliberately owns `ClassData` whose ID differs from the
/// specialized and registered IDs.
#[must_use]
pub fn container_shape_for_class_data_uuid(type_id: Uuid) -> Option<ContainerShape> {
    (type_id == RVALUE_WRAPPER_CLASS_DATA_TYPE_ID).then_some(ContainerShape::Wrapper)
}

/// Return the native capacity behavior for a canonical generic family UUID.
#[must_use]
pub fn container_capacity_kind_for_generic_uuid(
    generic_type_id: Uuid,
) -> Option<ContainerCapacityKind> {
    match generic_type_id {
        id if id == GENERIC_FIXED_VECTOR_TYPE_ID || id == GENERIC_OPTIONAL_TYPE_ID => {
            Some(ContainerCapacityKind::FixedCapacity)
        }
        id if id == GENERIC_ARRAY_TYPE_ID
            || id == GENERIC_PAIR_TYPE_ID
            || id == GENERIC_TUPLE_TYPE_ID
            || id == GENERIC_SHARED_PTR_TYPE_ID
            || id == GENERIC_INTRUSIVE_PTR_TYPE_ID
            || id == GENERIC_UNIQUE_PTR_TYPE_ID
            || id == GENERIC_VARIANT_TYPE_ID =>
        {
            Some(ContainerCapacityKind::FixedSize)
        }
        id if container_shape_for_generic_uuid(id).is_some() => {
            Some(ContainerCapacityKind::Dynamic)
        }
        _ => None,
    }
}

/// Return native capacity behavior keyed by canonical `ClassData` evidence.
#[must_use]
pub fn container_capacity_kind_for_class_data_uuid(type_id: Uuid) -> Option<ContainerCapacityKind> {
    (type_id == RVALUE_WRAPPER_CLASS_DATA_TYPE_ID).then_some(ContainerCapacityKind::FixedSize)
}

/// Reconstruct `ObjectStream` child cardinality for one canonical native family.
///
/// `non_type_capacity` is required for `fixed_vector` and `array`.
/// `templated_argument_count` is required for tuple size. Missing required
/// evidence returns `None` instead of inventing a bound. This differs from
/// [`ContainerCapacityKind`] for nullable smart pointers: their native adapter
/// reports fixed size one, while `ObjectStream` omits the null pointer child.
#[must_use]
pub fn container_cardinality_for_generic_uuid(
    generic_type_id: Uuid,
    non_type_capacity: Option<usize>,
    templated_argument_count: Option<usize>,
) -> Option<ContainerCardinality> {
    match generic_type_id {
        id if id == GENERIC_FIXED_VECTOR_TYPE_ID => {
            non_type_capacity.map(|capacity| ContainerCardinality::Bounded { capacity })
        }
        id if id == GENERIC_ARRAY_TYPE_ID => {
            non_type_capacity.map(|size| ContainerCardinality::Fixed { size })
        }
        id if id == GENERIC_PAIR_TYPE_ID => Some(ContainerCardinality::Fixed { size: 2 }),
        id if id == GENERIC_TUPLE_TYPE_ID => {
            templated_argument_count.map(|size| ContainerCardinality::Fixed { size })
        }
        id if id == GENERIC_SHARED_PTR_TYPE_ID
            || id == GENERIC_INTRUSIVE_PTR_TYPE_ID
            || id == GENERIC_UNIQUE_PTR_TYPE_ID =>
        {
            // The native adapter reports fixed size one, but ObjectStream
            // omits a null FLG_POINTER child during enumeration.
            Some(ContainerCardinality::Bounded { capacity: 1 })
        }
        id if id == GENERIC_VARIANT_TYPE_ID => Some(ContainerCardinality::Fixed { size: 1 }),
        id if id == GENERIC_OPTIONAL_TYPE_ID => Some(ContainerCardinality::Bounded { capacity: 1 }),
        id if container_shape_for_generic_uuid(id).is_some() => {
            Some(ContainerCardinality::Variable)
        }
        _ => None,
    }
}

/// Reconstruct native cardinality keyed by canonical `ClassData` evidence.
#[must_use]
pub fn container_cardinality_for_class_data_uuid(type_id: Uuid) -> Option<ContainerCardinality> {
    (type_id == RVALUE_WRAPPER_CLASS_DATA_TYPE_ID)
        .then_some(ContainerCardinality::Fixed { size: 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ObjectStreamReadContext, ReflectedClass};

    #[test]
    fn canonical_generic_ids_classify_by_native_adapter_family() {
        for (type_ids, expected) in [
            (
                &[
                    GENERIC_VECTOR_TYPE_ID,
                    GENERIC_FIXED_VECTOR_TYPE_ID,
                    GENERIC_LIST_TYPE_ID,
                    GENERIC_FORWARD_LIST_TYPE_ID,
                    GENERIC_ARRAY_TYPE_ID,
                ][..],
                ContainerShape::Sequence,
            ),
            (
                &[
                    GENERIC_SET_TYPE_ID,
                    GENERIC_UNORDERED_SET_TYPE_ID,
                    GENERIC_UNORDERED_MULTISET_TYPE_ID,
                ][..],
                ContainerShape::Set,
            ),
            (
                &[
                    GENERIC_MAP_TYPE_ID,
                    GENERIC_UNORDERED_MAP_TYPE_ID,
                    GENERIC_UNORDERED_MULTIMAP_TYPE_ID,
                    GENERIC_UNORDERED_FLAT_MAP_TYPE_ID,
                ][..],
                ContainerShape::Map,
            ),
            (&[GENERIC_PAIR_TYPE_ID][..], ContainerShape::Pair),
            (&[GENERIC_TUPLE_TYPE_ID][..], ContainerShape::Tuple),
            (
                &[
                    GENERIC_SHARED_PTR_TYPE_ID,
                    GENERIC_INTRUSIVE_PTR_TYPE_ID,
                    GENERIC_UNIQUE_PTR_TYPE_ID,
                ][..],
                ContainerShape::SmartPointer,
            ),
            (&[GENERIC_OPTIONAL_TYPE_ID][..], ContainerShape::Optional),
            (&[GENERIC_VARIANT_TYPE_ID][..], ContainerShape::Variant),
        ] {
            for type_id in type_ids {
                assert_eq!(container_shape_for_generic_uuid(*type_id), Some(expected));
            }
        }
    }

    #[test]
    fn wrapper_uses_its_intentionally_distinct_class_data_id() {
        assert_eq!(
            container_shape_for_class_data_uuid(RVALUE_WRAPPER_CLASS_DATA_TYPE_ID),
            Some(ContainerShape::Wrapper)
        );
        assert_eq!(
            container_shape_for_generic_uuid(RVALUE_WRAPPER_CLASS_DATA_TYPE_ID),
            None
        );
    }

    #[test]
    fn native_capacity_kind_preserves_fixed_vector_array_and_wrapper_distinctions() {
        assert_eq!(
            container_capacity_kind_for_generic_uuid(GENERIC_VECTOR_TYPE_ID),
            Some(ContainerCapacityKind::Dynamic)
        );
        assert_eq!(
            container_capacity_kind_for_generic_uuid(GENERIC_FIXED_VECTOR_TYPE_ID),
            Some(ContainerCapacityKind::FixedCapacity)
        );
        assert_eq!(
            container_capacity_kind_for_generic_uuid(GENERIC_ARRAY_TYPE_ID),
            Some(ContainerCapacityKind::FixedSize)
        );
        assert_eq!(
            container_capacity_kind_for_class_data_uuid(RVALUE_WRAPPER_CLASS_DATA_TYPE_ID),
            Some(ContainerCapacityKind::FixedSize)
        );
    }

    #[test]
    fn cardinality_requires_captured_non_type_bounds() {
        assert_eq!(
            container_cardinality_for_generic_uuid(GENERIC_FIXED_VECTOR_TYPE_ID, Some(30), None),
            Some(ContainerCardinality::Bounded { capacity: 30 })
        );
        assert_eq!(
            container_cardinality_for_generic_uuid(GENERIC_ARRAY_TYPE_ID, Some(4), None),
            Some(ContainerCardinality::Fixed { size: 4 })
        );
        assert_eq!(
            container_cardinality_for_generic_uuid(GENERIC_ARRAY_TYPE_ID, None, None),
            None
        );
        assert_eq!(
            container_cardinality_for_generic_uuid(GENERIC_TUPLE_TYPE_ID, None, Some(3)),
            Some(ContainerCardinality::Fixed { size: 3 })
        );
        assert_eq!(
            container_cardinality_for_generic_uuid(GENERIC_SHARED_PTR_TYPE_ID, None, None),
            Some(ContainerCardinality::Bounded { capacity: 1 })
        );
        assert_eq!(
            container_cardinality_for_class_data_uuid(RVALUE_WRAPPER_CLASS_DATA_TYPE_ID),
            Some(ContainerCardinality::Fixed { size: 1 })
        );
    }

    #[test]
    fn nullable_smart_pointer_and_optional_accept_zero_or_one_child() {
        for generic_type_id in [GENERIC_SHARED_PTR_TYPE_ID, GENERIC_OPTIONAL_TYPE_ID] {
            let cardinality =
                container_cardinality_for_generic_uuid(generic_type_id, None, None).unwrap();
            assert_cardinality(cardinality, &[0, 1], &[2]);
        }
    }

    #[test]
    fn fixed_and_bounded_families_enforce_captured_child_counts() {
        assert_cardinality(
            container_cardinality_for_generic_uuid(GENERIC_PAIR_TYPE_ID, None, None).unwrap(),
            &[2],
            &[0, 1, 3],
        );
        assert_cardinality(
            container_cardinality_for_generic_uuid(GENERIC_TUPLE_TYPE_ID, None, Some(3)).unwrap(),
            &[3],
            &[2, 4],
        );
        assert_cardinality(
            container_cardinality_for_generic_uuid(GENERIC_FIXED_VECTOR_TYPE_ID, Some(3), None)
                .unwrap(),
            &[0, 1, 3],
            &[4],
        );
        assert_cardinality(
            container_cardinality_for_generic_uuid(GENERIC_ARRAY_TYPE_ID, Some(3), None).unwrap(),
            &[3],
            &[2, 4],
        );
        assert_cardinality(
            container_cardinality_for_class_data_uuid(RVALUE_WRAPPER_CLASS_DATA_TYPE_ID).unwrap(),
            &[1],
            &[0, 2],
        );
    }

    #[test]
    fn specialized_registered_and_unknown_ids_are_not_guessed() {
        assert_eq!(
            container_shape_for_generic_uuid(Uuid::from_u128(
                0x2a3d1e1e_a69f_5860_8934_00c3bfe920cc
            )),
            None
        );
        assert_eq!(container_shape_for_generic_uuid(Uuid::nil()), None);
        assert_eq!(
            container_shape_for_generic_uuid(Uuid::from_u128(u128::MAX)),
            None
        );
    }

    fn assert_cardinality(
        cardinality: ContainerCardinality,
        accepted: &[usize],
        rejected: &[usize],
    ) {
        let type_id = Uuid::from_u128(1);
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                type_id,
                ReflectedClass::new(type_id).with_container_cardinality(cardinality),
            )
            .unwrap();

        for actual in accepted {
            assert!(
                context
                    .validate_container_cardinality(class, *actual)
                    .is_ok(),
                "{cardinality:?} rejected {actual}"
            );
        }
        for actual in rejected {
            assert!(
                matches!(
                    context.validate_container_cardinality(class, *actual),
                    Err(crate::ObjectStreamError::InvalidContainerCardinality {
                        expected: Some(expected),
                        actual: rejected_actual,
                        ..
                    }) if expected == cardinality && rejected_actual == *actual
                ),
                "{cardinality:?} accepted {actual}"
            );
        }
    }
}
