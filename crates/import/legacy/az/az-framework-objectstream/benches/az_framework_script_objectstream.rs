use az_framework::{
    SCRIPT_COMPONENT_TYPE_ID, SCRIPT_PROPERTY_BOOLEAN_TYPE_ID, SCRIPT_PROPERTY_GROUP_TYPE_ID,
    SCRIPT_PROPERTY_TYPE_ID,
};
use az_framework_objectstream::script::{
    read_script_bool_scalar, read_script_bool_vector, read_script_component,
    read_script_context_id, read_script_entity_ref, read_script_number_scalar,
    read_script_number_vector, read_script_property_group, read_script_property_key,
    read_script_string_scalar, read_script_string_vector,
};
use az_objectstream::{Element, types};
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use uuid::Uuid;

fn bench_script_property_key(c: &mut Criterion) {
    let key = Element::new(types::AZSTD_VECTOR).with_children([
        Element::new(types::AZ_U64)
            .with_field("Id")
            .with_data(0xCAFE_u64.to_be_bytes()),
        Element::new(types::AZSTD_STRING)
            .with_field("Name")
            .with_data(b"Enabled".as_slice()),
    ]);
    let entity_ref = Element::new(types::ENTITY_ID).with_children([Element::new(types::AZ_U64)
        .with_field("ID")
        .with_data(0xBEEF_u64.to_be_bytes())]);
    let context_id = Element::new(types::UNSIGNED_INT).with_data(0xCAFE_u32.to_be_bytes());
    let number_scalar = Element::new(types::DOUBLE).with_data(12.5_f64.to_be_bytes());
    let bool_scalar = Element::new(types::BOOL).with_data([1]);
    let string_scalar = Element::new(types::AZSTD_STRING).with_data(b"Enabled".as_slice());
    let bool_vector = Element::new(types::AZSTD_VECTOR).with_children([
        Element::new(types::BOOL).with_data([1]),
        Element::new(types::BOOL).with_data([0]),
    ]);
    let number_vector = Element::new(types::AZSTD_VECTOR).with_children([
        Element::new(types::DOUBLE).with_data(1.5_f64.to_be_bytes()),
        Element::new(types::FLOAT).with_data(2.25_f32.to_be_bytes()),
    ]);
    let string_vector = Element::new(types::AZSTD_VECTOR).with_children([
        Element::new(types::AZSTD_STRING).with_data(b"Alpha".as_slice()),
        Element::new(types::AZSTD_BASIC_STRING).with_data(b"Beta".as_slice()),
    ]);
    let property_group = property_group_element(None);
    let script_component = Element::new(SCRIPT_COMPONENT_TYPE_ID).with_children([
        Element::new(Uuid::nil())
            .with_field("BaseClass2")
            .with_children([Element::new(types::BOOL)
                .with_field("m_isNetSyncEnabled")
                .with_data([1])]),
        Element::new(types::UNSIGNED_INT)
            .with_field("ContextID")
            .with_data(0xCAFE_u32.to_be_bytes()),
        property_group_element(Some("properties")),
        Element::new(types::ASSET)
            .with_field("Script")
            .with_data(b"hint={scripts/props/lever.lua}".as_slice()),
        Element::new(types::BOOL)
            .with_field("IsRunOnServer")
            .with_data([0]),
        Element::new(types::BOOL)
            .with_field("IsRunOnClient")
            .with_data([1]),
    ]);

    c.bench_function("script_objectstream/read_script_property_key", |b| {
        b.iter(|| read_script_property_key(black_box(&key)).expect("key"));
    });
    c.bench_function("script_objectstream/read_script_context_id", |b| {
        b.iter(|| read_script_context_id(black_box(&context_id)).expect("context id"));
    });
    c.bench_function("script_objectstream/read_script_number_scalar", |b| {
        b.iter(|| read_script_number_scalar(black_box(&number_scalar)).expect("number"));
    });
    c.bench_function("script_objectstream/read_script_bool_scalar", |b| {
        b.iter(|| read_script_bool_scalar(black_box(&bool_scalar)).expect("bool"));
    });
    c.bench_function("script_objectstream/read_script_string_scalar", |b| {
        b.iter(|| read_script_string_scalar(black_box(&string_scalar)).expect("string"));
    });
    c.bench_function("script_objectstream/read_script_bool_vector", |b| {
        b.iter(|| read_script_bool_vector(black_box(&bool_vector)).expect("bool values"));
    });
    c.bench_function("script_objectstream/read_script_number_vector", |b| {
        b.iter(|| read_script_number_vector(black_box(&number_vector)).expect("number values"));
    });
    c.bench_function("script_objectstream/read_script_string_vector", |b| {
        b.iter(|| read_script_string_vector(black_box(&string_vector)).expect("string values"));
    });
    c.bench_function("script_objectstream/read_script_entity_ref", |b| {
        b.iter(|| read_script_entity_ref(black_box(&entity_ref)).expect("entity ref"));
    });
    c.bench_function("script_objectstream/read_script_property_group", |b| {
        b.iter(|| read_script_property_group(black_box(&property_group)).expect("group"));
    });
    c.bench_function("script_objectstream/read_script_component", |b| {
        b.iter(|| read_script_component(black_box(&script_component)).expect("component"));
    });
}

criterion_group!(benches, bench_script_property_key);
criterion_main!(benches);

fn script_property(id: Uuid, name: &str, children: Vec<Element>) -> Element {
    let mut property_children = vec![key_base(name)];
    property_children.extend(children);
    Element::new(id).with_children(property_children)
}

fn property_group_element(field: Option<&'static str>) -> Element {
    let element = Element::new(SCRIPT_PROPERTY_GROUP_TYPE_ID).with_children([
        Element::new(types::AZSTD_STRING)
            .with_field("Name")
            .with_data(b"Root".as_slice()),
        Element::new(types::AZSTD_VECTOR)
            .with_field("Properties")
            .with_children([script_property(
                SCRIPT_PROPERTY_BOOLEAN_TYPE_ID,
                "Enabled",
                vec![Element::new(types::BOOL).with_field("value").with_data([1])],
            )]),
    ]);

    if let Some(field) = field {
        element.with_field(field)
    } else {
        element
    }
}

fn key_base(name: &str) -> Element {
    Element::new(SCRIPT_PROPERTY_TYPE_ID)
        .with_field("BaseClass1")
        .with_children([
            Element::new(types::AZ_U64)
                .with_field("Id")
                .with_data(0xCAFE_u64.to_be_bytes()),
            Element::new(types::AZSTD_STRING)
                .with_field("Name")
                .with_data(name.as_bytes()),
        ])
}
