//! What gridmate itself contributes to a composed host's wire registries.
//!
//! Three registries carry a host's wire identity, and none of them is
//! populated by the linker any more:
//!
//! - [`ClassRegistration`] — every reflected class, keyed on its AZ UUID. One
//!   entry per type: [`ClassRegistration::of_message`] is the constructor for a
//!   type that is also a [`Message`](crate::message::Message), and it carries
//!   the envelope index and protocol metadata that a plain class has none of.
//! - [`FragmentRegistration`] — the concrete replication fragments this host
//!   can decode, keyed on UUID.
//! - [`DebugIntrospect`] — debug-only wire pretty-printers, keyed on envelope
//!   index. Registered from the same message list the class entries come from,
//!   so the two cannot drift.
//!
//! Every crate that declares gridmate types exposes this same shape: typed
//! arrays naming what it owns, and a `register` that hands them to the
//! registrar. The arrays are the enumeration — nothing scans for derives — so a
//! type that is declared and never listed is absent from the host, and
//! [`TypeRegistry::collect_diagnostics`](crate::az::TypeRegistry::collect_diagnostics)
//! is where that shows up: against a debug build's native snapshot, an omitted
//! class is a hole the walk can name.

use az_gem_contract::GemContext;

use crate::az::ClassRegistration;
#[cfg(debug_assertions)]
use crate::az::DebugIntrospect;
use crate::hub::{
    FragmentRegistration, GdeMetadataReplicatedState, ReplicatedStateBundle,
    ReplicationPerformanceData,
};

/// The reflected classes gridmate itself owns.
#[must_use]
pub const fn classes() -> [ClassRegistration; 3] {
    [
        ClassRegistration::of_message::<ReplicatedStateBundle>(),
        ClassRegistration::of_message::<ReplicationPerformanceData>(),
        ClassRegistration::of::<GdeMetadataReplicatedState>(),
    ]
}

/// The replication fragments gridmate itself owns.
#[must_use]
pub const fn fragments() -> [FragmentRegistration; 1] {
    [FragmentRegistration::of::<GdeMetadataReplicatedState>()]
}

/// Debug-build wire pretty-printers for gridmate's own message types.
#[cfg(debug_assertions)]
#[must_use]
pub const fn introspects() -> [DebugIntrospect; 2] {
    [
        DebugIntrospect::of::<ReplicatedStateBundle>(),
        DebugIntrospect::of::<ReplicationPerformanceData>(),
    ]
}

/// Register gridmate's own wire types into a composing host.
pub fn register<D>(ctx: &mut GemContext<'_, D>) {
    ctx.registrar::<ClassRegistration>()
        .register_many(classes());
    ctx.registrar::<FragmentRegistration>()
        .register_many(fragments());
    #[cfg(debug_assertions)]
    ctx.registrar::<DebugIntrospect>()
        .register_many(introspects());
}

#[cfg(test)]
mod tests {
    use az_core::type_info::AzTypeInfo;
    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    use super::*;
    use crate::az::TypeRegistry;
    use crate::hub::Fragments;
    use crate::serialize::buffer::{CARRIER_ENDIAN, Endian, ReadBuffer, WriteBuffer};
    use crate::serialize::{Marshaler, MarshalerError};

    declare_caps!(WireCaps:);

    const WIRE: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.gridmate"),
        contribution: ContributionId::new("wire"),
        roles: &[],
    };

    const RIVAL: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.gridmate-rival"),
        contribution: ContributionId::new("wire"),
        roles: &[],
    };

    struct Wire;

    impl Contribution for Wire {
        type Caps = WireCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            WIRE
        }

        fn register(&self, ctx: &mut GemContext<'_, WireCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::Unified);
        composer
            .add(Wire, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn a_type_deriving_both_class_and_message_registers_once() {
        let composer = composed();
        let types = TypeRegistry::of(composer.registries()).expect("classes composed");

        assert_eq!(types.len(), classes().len());

        // `ReplicatedStateBundle` carries `#[derive(ClassDesc, Message)]`. The
        // pair used to submit two entries, and lookup picked between them by
        // preferring a non-zero `type_index`. One entry now carries both
        // halves, so there is nothing left to prefer.
        let bundle = types
            .class_desc(&<ReplicatedStateBundle as AzTypeInfo>::TYPE_ID)
            .expect("the bundle class is registered");
        assert_eq!(bundle.type_index, 8);
        assert!(
            bundle.message.is_some(),
            "of_message carries the protocol metadata the class-only entry lacked"
        );

        let single = types
            .entries()
            .filter(|reg| reg.uuid == <ReplicatedStateBundle as AzTypeInfo>::TYPE_ID)
            .count();
        assert_eq!(single, 1, "two derives, one entry");
    }

    #[test]
    fn two_contributions_claiming_one_class_fail_composition() {
        struct Rival;

        impl Contribution for Rival {
            type Caps = WireCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                RIVAL
            }

            fn register(&self, ctx: &mut GemContext<'_, WireCaps>) {
                ctx.registrar::<ClassRegistration>()
                    .register(ClassRegistration::of_message::<ReplicatedStateBundle>());
            }
        }

        let mut composer = Composer::new(GemTargetRole::Unified);
        composer.add(Wire, ProductActivation::default()).unwrap();
        composer.add(Rival, ProductActivation::default()).unwrap();

        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer.finalize().unwrap_err()
        else {
            panic!("a repeated class UUID must fail composition");
        };
        assert_eq!(registry, "gridmate-class");
        assert_eq!(
            key,
            <ReplicatedStateBundle as AzTypeInfo>::TYPE_ID.to_string()
        );
        assert_eq!(first.gem.as_str(), "azoth.gridmate");
        assert_eq!(second.gem.as_str(), "azoth.gridmate-rival");
    }

    #[test]
    fn compact_wire_identity_resolves_through_the_composition() {
        let composer = composed();
        let types = TypeRegistry::of(composer.registries()).expect("classes composed");

        let uuid = types.uuid_for_type_index(8).expect("index 8 is the bundle");
        assert_eq!(uuid, <ReplicatedStateBundle as AzTypeInfo>::TYPE_ID);
        assert_eq!(types.type_index_of(&uuid), Some(8));
        assert_eq!(
            types.name_for_type_index(8),
            Some(<ReplicatedStateBundle as AzTypeInfo>::NAME)
        );
        assert_eq!(types.uuid_for_type_index(0), None);
    }

    #[test]
    fn a_fragment_resolves_through_the_pair_the_host_composed() {
        let composer = composed();
        let fragments = Fragments::of(composer.registries()).expect("fragments composed");

        let entry = fragments
            .by_type_index(10)
            .expect("the metadata fragment is registered");
        assert_eq!(entry.name, <GdeMetadataReplicatedState as AzTypeInfo>::NAME);
        assert_eq!(fragments.type_indices(), vec![10]);
        assert_eq!(fragments.name(10), Some(entry.name));
    }

    #[test]
    fn a_host_that_composed_nothing_has_no_wire_registries() {
        let registries = Registries::new();
        assert!(TypeRegistry::of(&registries).is_none());
        assert!(Fragments::of(&registries).is_none());
    }

    /// A class with no wire body, so the assertion is about type dispatch and
    /// nothing else.
    #[derive(
        az_derive::AzTypeInfo,
        gridmate_derive::Marshaler,
        gridmate_derive::ClassDesc,
        Debug,
        Default,
    )]
    #[az_type_info("33333333-3333-4333-8333-333333333333")]
    #[class_desc(type_index = 64_990)]
    struct Marker;

    struct Extra;

    impl Contribution for Extra {
        type Caps = WireCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            RIVAL
        }

        fn register(&self, ctx: &mut GemContext<'_, WireCaps>) {
            ctx.registrar::<ClassRegistration>()
                .register(ClassRegistration::of::<Marker>());
        }
    }

    #[test]
    fn an_unbound_read_buffer_names_the_omission_instead_of_reporting_unknown_types() {
        // A `ClassValue` is the erased carrier: it cannot know how many bytes
        // the body occupies without the registry. Before composition this read
        // reached a process-global registry; a buffer nobody bound must say so.
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        crate::az::ClassValue {
            uuid: <Marker as AzTypeInfo>::TYPE_ID,
            type_index: 64_990,
            body: Vec::new(),
        }
        .marshal(&mut wb);
        let bytes = wb.into_vec();

        let mut unbound = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        assert!(matches!(
            crate::az::ClassValue::unmarshal(&mut unbound),
            Err(MarshalerError::UnboundTypes)
        ));

        let mut composer = Composer::new(GemTargetRole::Unified);
        composer.add(Extra, ProductActivation::default()).unwrap();
        let types = TypeRegistry::of(composer.registries()).expect("classes composed");
        let mut bound = ReadBuffer::new(CARRIER_ENDIAN, &bytes).with_types(types);
        let value = crate::az::ClassValue::unmarshal(&mut bound).expect("bound decode resolves");
        assert_eq!(value.type_index, 64_990);
        assert_eq!(value.class_name(types), Some(<Marker as AzTypeInfo>::NAME));
    }

    #[test]
    fn diagnostics_name_two_classes_claiming_one_envelope_index() {
        // The composer's key is type identity, so it cannot see this: two
        // distinct UUIDs, one compact id. Wire dispatch would silently follow
        // whichever composed first.
        struct Clash;

        impl Contribution for Clash {
            type Caps = WireCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                RIVAL
            }

            fn register(&self, ctx: &mut GemContext<'_, WireCaps>) {
                let mut entry = ClassRegistration::of::<GdeMetadataReplicatedState>();
                entry.uuid = uuid::Uuid::from_u128(0x1234);
                entry.name = "azoth.test.Impostor";
                ctx.registrar::<ClassRegistration>().register(entry);
            }
        }

        let mut composer = Composer::new(GemTargetRole::Unified);
        composer.add(Wire, ProductActivation::default()).unwrap();
        composer.add(Clash, ProductActivation::default()).unwrap();
        composer
            .finalize()
            .expect("distinct UUIDs compose; the clash is on the compact id");

        let diag = TypeRegistry::of(composer.registries())
            .expect("classes composed")
            .collect_diagnostics();
        let clash = diag
            .duplicate_type_index
            .first()
            .expect("the clash is reported, not resolved by scan order");
        assert_eq!(clash.type_index, 10);
        assert_eq!(
            clash.first,
            <GdeMetadataReplicatedState as AzTypeInfo>::NAME
        );
        assert_eq!(clash.second, "azoth.test.Impostor");
        assert!(!diag.is_clean());
    }

    /// The drift check the two arrays need: an introspection entry exists for
    /// every message class, keyed on the same compact id. This is the
    /// omission detector for the debug family — a message added to `classes()`
    /// and forgotten in `introspects()` fails here rather than going quiet in
    /// a capture tool.
    #[cfg(debug_assertions)]
    #[test]
    fn every_composed_message_class_has_an_introspection_entry() {
        let composer = composed();
        let registries = composer.registries();
        let types = TypeRegistry::of(registries).expect("classes composed");
        let introspects = registries
            .get::<DebugIntrospect>()
            .expect("introspects composed");

        for reg in types.entries() {
            if reg.message.is_none() {
                continue;
            }
            let entry = crate::az::introspect(introspects, reg.type_index)
                .unwrap_or_else(|| panic!("no introspection entry for {}", reg.name));
            assert_eq!(entry.name, reg.name);
        }
    }

    #[test]
    fn endianness_and_types_are_independent_buffer_state() {
        let composer = composed();
        let types = TypeRegistry::of(composer.registries()).expect("classes composed");
        let bytes = [0u8; 4];
        let rb = ReadBuffer::new(Endian::LittleEndian, &bytes).with_types(types);
        assert!(rb.types().is_some());
        assert!(
            ReadBuffer::new(Endian::LittleEndian, &bytes)
                .types()
                .is_none()
        );
    }
}
