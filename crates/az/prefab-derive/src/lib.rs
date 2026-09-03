//! Derive support for Azoth's narrow Bevy-native Prefab registration.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Attribute, DeriveInput, Ident, LitInt, LitStr, Path, parse_macro_input, spanned::Spanned,
};

#[proc_macro_derive(Prefab, attributes(prefab))]
pub fn derive_prefab(input: TokenStream) -> TokenStream {
    expand_prefab(&parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Clone)]
struct Alias {
    tag: LitStr,
    version: u32,
    migrate: Option<Path>,
}

#[derive(Clone)]
struct Migration {
    from: u32,
    to: u32,
    migrate: Path,
}

#[derive(Clone, Copy, Default)]
enum ProductPolicy {
    #[default]
    Runtime,
    SourceOnly,
}

#[derive(Default)]
struct PrefabAttributes {
    tag: Option<LitStr>,
    version: Option<u32>,
    aliases: Vec<Alias>,
    migrations: Vec<Migration>,
    template: Option<Path>,
    construct: Option<Path>,
    product: Option<ProductPolicy>,
}

fn expand_prefab(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let attributes = parse_prefab_attributes(&input.attrs)?;
    validate_reflect_registration(&input.attrs, attributes.template.is_some())?;
    validate_source_metadata(&attributes)?;

    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let target = quote!(#ident #ty_generics);
    let version = attributes
        .version
        .ok_or_else(|| syn::Error::new(input.span(), "Prefab requires #[prefab(version = ...)]"))?;
    let tag = attributes.tag.as_ref().map_or_else(
        || quote!(<#target as ::az_prefab::__private::bevy_reflect::TypePath>::type_path()),
        |tag| quote!(#tag),
    );
    let aliases = attributes.aliases.iter().map(|alias| {
        let tag = &alias.tag;
        let version = alias.version;
        quote!(::az_prefab::PrefabTagAlias { tag: #tag, source_version: #version })
    });
    let migrations = normalized_migrations(&attributes)
        .into_iter()
        .map(|migration| {
            let from = migration.from;
            let to = migration.to;
            let migrate = migration.migrate;
            quote!(::az_prefab::PrefabMigrationStep {
                from_version: #from,
                to_version: #to,
                migrate: #migrate,
            })
        });
    let product_policy = match attributes.product.unwrap_or_default() {
        ProductPolicy::Runtime => quote!(::az_prefab::PrefabProductPolicy::Runtime),
        ProductPolicy::SourceOnly => quote!(::az_prefab::PrefabProductPolicy::SourceOnly),
    };

    let (construction, construct) = match (&attributes.template, &attributes.construct) {
        (Some(template), Some(adapter)) => (
            quote!(::az_prefab::PrefabConstruction::Template {
                template_type_info: || <#template as ::az_prefab::__private::bevy_reflect::Typed>::type_info(),
            }),
            quote!(|registration, sparse, entity, references| {
                ::az_prefab::type_data::construct_from_template::<#template, #target>(
                    registration,
                    sparse,
                    entity,
                    references,
                    #adapter,
                )
            }),
        ),
        (None, None) => (
            quote!(::az_prefab::PrefabConstruction::ReflectDefaultOrFromWorld),
            quote!(::az_prefab::type_data::construct_reflected),
        ),
        _ => unreachable!("template/construct pairing is validated"),
    };

    Ok(quote! {
        impl #impl_generics ::az_prefab::__private::bevy_reflect::FromType<#target>
            for ::az_prefab::PrefabTypeData #where_clause
        {
            fn from_type() -> Self {
                ::az_prefab::PrefabTypeData {
                    tag: #tag,
                    source_version: #version,
                    aliases: &[#(#aliases),*],
                    migrations: &[#(#migrations),*],
                    construction: #construction,
                    product_policy: #product_policy,
                    construct: #construct,
                    insert: ::az_prefab::type_data::insert_reflected_component,
                }
            }
        }
    })
}

fn parse_prefab_attributes(attrs: &[Attribute]) -> syn::Result<PrefabAttributes> {
    let mut result = PrefabAttributes::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("prefab")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tag") {
                replace_once(
                    &mut result.tag,
                    meta.value()?.parse()?,
                    meta.path.span(),
                    "tag",
                )?;
            } else if meta.path.is_ident("version") {
                let value: LitInt = meta.value()?.parse()?;
                replace_once(
                    &mut result.version,
                    value.base10_parse()?,
                    meta.path.span(),
                    "version",
                )?;
            } else if meta.path.is_ident("template") {
                replace_once(
                    &mut result.template,
                    meta.value()?.parse()?,
                    meta.path.span(),
                    "template",
                )?;
            } else if meta.path.is_ident("construct") {
                replace_once(
                    &mut result.construct,
                    meta.value()?.parse()?,
                    meta.path.span(),
                    "construct",
                )?;
            } else if meta.path.is_ident("product") {
                let value: Ident = meta.value()?.parse()?;
                let product = match value.to_string().as_str() {
                    "Runtime" => ProductPolicy::Runtime,
                    "SourceOnly" => ProductPolicy::SourceOnly,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "Prefab product must be Runtime or SourceOnly",
                        ));
                    }
                };
                replace_once(&mut result.product, product, meta.path.span(), "product")?;
            } else if meta.path.is_ident("alias") {
                result.aliases.push(parse_alias(&meta)?);
            } else if meta.path.is_ident("migration") {
                result.migrations.push(parse_migration(&meta)?);
            } else {
                return Err(meta.error("unsupported Prefab option"));
            }
            Ok(())
        })?;
    }
    Ok(result)
}

fn parse_alias(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Alias> {
    let mut tag = None;
    let mut version = None;
    let mut migrate = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("tag") {
            replace_once(
                &mut tag,
                nested.value()?.parse()?,
                nested.path.span(),
                "alias tag",
            )
        } else if nested.path.is_ident("version") {
            let value: LitInt = nested.value()?.parse()?;
            replace_once(
                &mut version,
                value.base10_parse()?,
                nested.path.span(),
                "alias version",
            )
        } else if nested.path.is_ident("migrate") {
            replace_once(
                &mut migrate,
                nested.value()?.parse()?,
                nested.path.span(),
                "alias migration",
            )
        } else {
            Err(nested.error("unsupported Prefab alias option"))
        }
    })?;
    Ok(Alias {
        tag: tag.ok_or_else(|| meta.error("Prefab alias requires tag"))?,
        version: version.ok_or_else(|| meta.error("Prefab alias requires version"))?,
        migrate,
    })
}

fn parse_migration(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Migration> {
    let mut from = None;
    let mut to = None;
    let mut migrate = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("from") {
            let value: LitInt = nested.value()?.parse()?;
            replace_once(
                &mut from,
                value.base10_parse()?,
                nested.path.span(),
                "migration from",
            )
        } else if nested.path.is_ident("to") {
            let value: LitInt = nested.value()?.parse()?;
            replace_once(
                &mut to,
                value.base10_parse()?,
                nested.path.span(),
                "migration to",
            )
        } else if nested.path.is_ident("migrate") {
            replace_once(
                &mut migrate,
                nested.value()?.parse()?,
                nested.path.span(),
                "migration function",
            )
        } else {
            Err(nested.error("unsupported Prefab migration option"))
        }
    })?;
    Ok(Migration {
        from: from.ok_or_else(|| meta.error("Prefab migration requires from"))?,
        to: to.ok_or_else(|| meta.error("Prefab migration requires to"))?,
        migrate: migrate.ok_or_else(|| meta.error("Prefab migration requires migrate"))?,
    })
}

fn replace_once<T>(
    destination: &mut Option<T>,
    value: T,
    span: proc_macro2::Span,
    name: &str,
) -> syn::Result<()> {
    if destination.replace(value).is_some() {
        return Err(syn::Error::new(span, format!("duplicate Prefab {name}")));
    }
    Ok(())
}

fn validate_reflect_registration(attrs: &[Attribute], template_backed: bool) -> syn::Result<()> {
    let reflect_tokens = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("reflect"))
        .map(ToTokens::to_token_stream)
        .map(|tokens| tokens.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let has = |name: &str| {
        reflect_tokens
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(|token| token == name)
    };

    if !has("Component") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "Prefab requires #[reflect(Component)] so ReflectComponent is registered",
        ));
    }
    if !has("Prefab") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "Prefab requires #[reflect(Prefab)] and `use az_prefab::ReflectPrefab` so PrefabTypeData is inserted into the normal Bevy TypeRegistration",
        ));
    }
    if !template_backed && !has("Default") && !has("FromWorld") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "Prefab requires #[reflect(Default)] or #[reflect(FromWorld)], or a template construction adapter",
        ));
    }
    Ok(())
}

fn validate_source_metadata(attributes: &PrefabAttributes) -> syn::Result<()> {
    let version = attributes
        .version
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing Prefab version"))?;
    if let Some(tag) = &attributes.tag {
        validate_tag(tag)?;
    }
    match (&attributes.template, &attributes.construct) {
        (Some(_), None) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "template-backed Prefab requires construct = <adapter>",
            ));
        }
        (None, Some(_)) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Prefab construct adapter requires template = <type>",
            ));
        }
        _ => {}
    }

    let canonical = attributes.tag.as_ref().map(LitStr::value);
    let mut tags = BTreeSet::new();
    if let Some(tag) = canonical {
        tags.insert(tag);
    }
    for alias in &attributes.aliases {
        validate_tag(&alias.tag)?;
        if alias.version > version {
            return Err(syn::Error::new(
                alias.tag.span(),
                "Prefab alias version must be in 0..=current version",
            ));
        }
        if !tags.insert(alias.tag.value()) {
            return Err(syn::Error::new(
                alias.tag.span(),
                "Prefab canonical/alias tag collision",
            ));
        }
    }

    let migrations = normalized_migrations(attributes);
    let mut edges = BTreeMap::new();
    for migration in &migrations {
        if migration.to > version {
            return Err(syn::Error::new(
                migration.migrate.span(),
                "Prefab migration versions must be in 0..=current version",
            ));
        }
        if migration.to <= migration.from {
            return Err(syn::Error::new(
                migration.migrate.span(),
                "Prefab migration chain is cyclic or non-monotonic",
            ));
        }
        if migration.to != migration.from + 1 {
            return Err(syn::Error::new(
                migration.migrate.span(),
                "Prefab migration chain has a version gap",
            ));
        }
        if edges.insert(migration.from, migration.to).is_some() {
            return Err(syn::Error::new(
                migration.migrate.span(),
                "Prefab migration chain has an ambiguous outgoing edge",
            ));
        }
    }

    for alias in &attributes.aliases {
        if alias.version < version {
            let mut cursor = alias.version;
            while cursor < version {
                cursor = *edges.get(&cursor).ok_or_else(|| {
                    syn::Error::new(alias.tag.span(), "Prefab alias migration chain is gapped")
                })?;
            }
        }
    }
    if let Some((&start, _)) = edges.first_key_value() {
        let mut cursor = start;
        while let Some(next) = edges.get(&cursor) {
            cursor = *next;
        }
        if cursor != version {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Prefab migration chain does not end at the registered current version",
            ));
        }
    }
    Ok(())
}

fn normalized_migrations(attributes: &PrefabAttributes) -> Vec<Migration> {
    let mut migrations = attributes.migrations.clone();
    migrations.extend(attributes.aliases.iter().filter_map(|alias| {
        alias.migrate.clone().map(|migrate| Migration {
            from: alias.version,
            to: alias.version + 1,
            migrate,
        })
    }));
    migrations
}

fn validate_tag(tag: &LitStr) -> syn::Result<()> {
    let value = tag.value();
    if value.trim().is_empty() || value != value.trim() || value.starts_with("__") {
        return Err(syn::Error::new(
            tag.span(),
            "Prefab tag must be non-empty, trimmed, and outside the reserved `__` namespace",
        ));
    }
    Ok(())
}
