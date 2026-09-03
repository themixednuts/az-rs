use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Data, DeriveInput, LitStr, Result};

use crate::attrs::{AzAttrs, az_core_path, parse_az_attrs};

pub fn expand_az_type_info(input: &DeriveInput) -> Result<TokenStream> {
    let attrs = parse_attrs(
        input,
        &["az_type_info", "type_info"],
        "#[derive(AzTypeInfo)]",
        false,
    )?;
    let trait_path = attrs.trait_path_or_default(az_core_path("type_info", "AzTypeInfo"));

    Ok(type_info_impl(input, &attrs, &trait_path))
}

pub fn expand_az_rtti(input: &DeriveInput) -> Result<TokenStream> {
    let attrs = parse_attrs(input, &["az_rtti", "rtti"], "#[derive(AzRtti)]", true)?;
    let rtti_path = attrs.trait_path_or_default(az_core_path("rtti", "AzRtti"));
    let rtti_stack = rtti_stack_impl(input, &attrs, &rtti_path, &[]);
    let registration = rtti_registration_impl(input, attrs.register);

    Ok(quote! {
        #rtti_stack
        #registration
    })
}

pub fn expand_az_component(input: &DeriveInput) -> Result<TokenStream> {
    let attrs = parse_attrs(
        input,
        &["az_component", "component"],
        "#[derive(AzComponent)]",
        true,
    )?;
    let component_path = attrs.trait_path_or_default(az_core_path("component", "AzComponent"));
    let az_rtti = az_core_path("rtti", "AzRtti");
    let registration = component_type_registration_impl(input);
    let component_type_id = az_core_path("component", "COMPONENT_TYPE_ID");

    let rtti_stack = rtti_stack_impl(input, &attrs, &az_rtti, &[component_type_id]);
    let component_impl = blank_trait_impl(input, &component_path);

    Ok(quote! {
        #rtti_stack

        #component_impl

        #registration
    })
}

fn parse_attrs(
    input: &DeriveInput,
    attr_names: &[&str],
    derive_label: &str,
    allow_bases: bool,
) -> Result<AzAttrs> {
    reject_unions(input, derive_label)?;
    parse_az_attrs(input, attr_names, allow_bases)?.into_validated(input)
}

fn rtti_stack_impl(
    input: &DeriveInput,
    attrs: &AzAttrs,
    rtti_path: &TokenStream,
    extra_base_type_ids: &[TokenStream],
) -> TokenStream {
    let type_info_path = az_core_path("type_info", "AzTypeInfo");
    let type_info = type_info_impl(input, attrs, &type_info_path);
    let rtti = rtti_impl(
        input,
        attrs,
        rtti_path,
        &type_info_path,
        extra_base_type_ids,
    );

    quote! {
        #type_info
        #rtti
    }
}

fn rtti_impl(
    input: &DeriveInput,
    attrs: &AzAttrs,
    rtti_path: &TokenStream,
    type_info_path: &TokenStream,
    extra_base_type_ids: &[TokenStream],
) -> TokenStream {
    let ident = &input.ident;
    let bases = &attrs.bases;
    let base_type_ids = bases
        .iter()
        .map(|base| quote!(<#base as #type_info_path>::TYPE_ID))
        .chain(extra_base_type_ids.iter().cloned())
        .collect::<Vec<_>>();
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    if base_type_ids.is_empty() {
        quote! {
            impl #impl_generics #rtti_path for #ident #ty_generics #where_clause {}
        }
    } else {
        quote! {
            impl #impl_generics #rtti_path for #ident #ty_generics #where_clause {
                const BASE_TYPE_IDS: &'static [::uuid::Uuid] = &[
                    #(#base_type_ids),*
                ];
            }
        }
    }
}

fn type_info_impl(input: &DeriveInput, attrs: &AzAttrs, trait_path: &TokenStream) -> TokenStream {
    let ident = &input.ident;
    let name = LitStr::new(&attrs.name, ident.span());
    let type_id = &attrs.type_id;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics #trait_path for #ident #ty_generics #where_clause {
            const NAME: &'static str = #name;
            const TYPE_ID: ::uuid::Uuid = #type_id;
        }
    }
}

fn rtti_registration_impl(input: &DeriveInput, register: bool) -> TokenStream {
    if !register {
        return TokenStream::new();
    }

    registration_item(
        input,
        &az_core_path("rtti", "AzTypeRegistration"),
        &format_ident!("rtti"),
    )
}

fn component_type_registration_impl(input: &DeriveInput) -> TokenStream {
    registration_item(
        input,
        &az_core_path("component", "ComponentLoweringRegistration"),
        &format_ident!("derived_component"),
    )
}

/// The inherent registration item this type carries for its crate's
/// enumeration to hand a composing host.
///
/// Nothing walks a link section any more, so the value has to be named by the
/// crate that owns the type. `pub(crate)` plus `deny(dead_code)` is what makes
/// that list omission-proof: a type that declares a registration and is then
/// left out of its crate's enumeration is a compile error naming that type,
/// where the submission it replaces would have produced an entry no host ever
/// saw. The visibility is the other half — the const is crate-private on
/// purpose, so the only place that can enumerate a type is the crate that
/// defines it.
fn registration_item(
    input: &DeriveInput,
    entry: &TokenStream,
    constructor: &syn::Ident,
) -> TokenStream {
    if !input.generics.params.is_empty() {
        return TokenStream::new();
    }

    let ident = &input.ident;

    // Spanned at the type's own name, not at `call_site`. `dead_code` is
    // suppressed for spans rustc attributes to an external macro expansion, so
    // a `call_site` item would carry the deny and never fire it. Borrowing the
    // ident's span also puts the error where the reader needs it: on the type
    // whose registration went missing.
    quote_spanned! {ident.span()=>
        impl #ident {
            /// This type's registry entry, for this crate's registration
            /// enumeration. Crate-private: the owning crate enumerates.
            #[deny(dead_code)]
            pub(crate) const REGISTRATION: #entry = #entry::#constructor::<#ident>();
        }
    }
}

fn blank_trait_impl(input: &DeriveInput, trait_path: &TokenStream) -> TokenStream {
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics #trait_path for #ident #ty_generics #where_clause {}
    }
}

fn reject_unions(input: &DeriveInput, derive_label: &str) -> Result<()> {
    if let Data::Union(u) = &input.data {
        return Err(syn::Error::new_spanned(
            u.union_token,
            format!("{derive_label} does not support unions"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn az_component_tokens() -> String {
        let input: DeriveInput = parse_quote! {
            #[az_component("11111111-1111-1111-1111-111111111111")]
            struct DemoComponent;
        };

        expand_az_component(&input)
            .expect("AzComponent expansion should succeed")
            .to_string()
    }

    fn az_rtti_register_tokens() -> String {
        let input: DeriveInput = parse_quote! {
            #[az_rtti("44444444-4444-4444-4444-444444444444", register)]
            struct DemoRtti;
        };

        expand_az_rtti(&input)
            .expect("AzRtti expansion should register concrete types when requested")
            .to_string()
    }

    #[test]
    fn az_component_registration_uses_unified_type_registry() {
        let tokens = az_component_tokens();

        assert!(
            tokens.contains(
                ":: az_core :: component :: ComponentLoweringRegistration :: derived_component"
            ),
            "{tokens}"
        );
        assert!(!tokens.contains("NativeComponentRegistration"), "{tokens}");
    }

    #[test]
    fn az_component_registration_does_not_test_consumer_features() {
        let tokens = az_component_tokens();

        assert!(!tokens.contains("cfg"), "{tokens}");
        assert!(tokens.contains("const REGISTRATION"), "{tokens}");
    }

    /// The whole point of the item: an omitted type is a compile error, not an
    /// entry nobody sees. `pub(crate)` keeps the enumeration in the owning
    /// crate and `deny(dead_code)` is what fires when it is missing from it.
    #[test]
    fn a_registration_item_is_crate_private_and_denies_being_unused() {
        for tokens in [az_component_tokens(), az_rtti_register_tokens()] {
            assert!(
                tokens.contains("# [deny (dead_code)] pub (crate) const REGISTRATION"),
                "{tokens}"
            );
            assert!(!tokens.contains("inventory"), "{tokens}");
        }
    }

    #[test]
    fn az_component_rtti_includes_native_component_base() {
        let tokens = az_component_tokens();

        assert!(
            tokens.contains(":: az_core :: component :: COMPONENT_TYPE_ID"),
            "{tokens}"
        );
    }

    #[test]
    fn az_type_info_accepts_braced_source_uuid_strings() {
        let input: DeriveInput = parse_quote! {
            #[az_type_info("{22222222-2222-2222-2222-222222222222}")]
            struct DemoType;
        };

        let tokens = expand_az_type_info(&input)
            .expect("AzTypeInfo expansion should accept braced source UUIDs")
            .to_string();

        assert!(
            tokens.contains(":: uuid :: uuid ! (\"22222222-2222-2222-2222-222222222222\")"),
            "{tokens}"
        );
    }

    #[test]
    fn az_type_info_accepts_type_id_expressions() {
        let input: DeriveInput = parse_quote! {
            #[az_type_info(type_id_for_demo())]
            struct DemoType;
        };

        let tokens = expand_az_type_info(&input)
            .expect("AzTypeInfo expansion should accept type id expressions")
            .to_string();

        assert!(tokens.contains("type_id_for_demo ()"), "{tokens}");
    }

    #[test]
    fn az_type_info_accepts_name_override_before_positional_type_id() {
        let input: DeriveInput = parse_quote! {
            #[az_type_info(name = "NativeDemo", native_demo_type_id())]
            struct DemoType;
        };

        let tokens = expand_az_type_info(&input)
            .expect("AzTypeInfo expansion should accept name plus positional UUID")
            .to_string();

        assert!(
            tokens.contains("const NAME : & 'static str = \"NativeDemo\""),
            "{tokens}"
        );
        assert!(tokens.contains("native_demo_type_id ()"), "{tokens}");
    }

    #[test]
    fn az_rtti_positional_bases_emit_base_type_ids() {
        let input: DeriveInput = parse_quote! {
            #[az_rtti("33333333-3333-3333-3333-333333333333", BaseOne, namespace::BaseTwo)]
            struct DemoRtti;
        };

        let tokens = expand_az_rtti(&input)
            .expect("AzRtti expansion should accept base type arguments")
            .to_string();

        assert!(tokens.contains("const BASE_TYPE_IDS"), "{tokens}");
        assert!(
            tokens.contains("< BaseOne as :: az_core :: type_info :: AzTypeInfo > :: TYPE_ID"),
            "{tokens}"
        );
        assert!(
            tokens.contains(
                "< namespace :: BaseTwo as :: az_core :: type_info :: AzTypeInfo > :: TYPE_ID"
            ),
            "{tokens}"
        );
    }

    #[test]
    fn az_rtti_accepts_name_before_positional_type_id_and_bases() {
        let input: DeriveInput = parse_quote! {
            #[az_rtti(
                name = "NativeDemo",
                native_demo_type_id(),
                BaseOne,
                namespace::BaseTwo,
            )]
            struct DemoRtti;
        };

        let tokens = expand_az_rtti(&input)
            .expect("AzRtti expansion should accept name plus positional UUID and bases")
            .to_string();

        assert!(
            tokens.contains("const NAME : & 'static str = \"NativeDemo\""),
            "{tokens}"
        );
        assert!(tokens.contains("native_demo_type_id ()"), "{tokens}");
        assert!(tokens.contains("const BASE_TYPE_IDS"), "{tokens}");
        assert!(
            tokens.contains("< BaseOne as :: az_core :: type_info :: AzTypeInfo > :: TYPE_ID"),
            "{tokens}"
        );
        assert!(
            tokens.contains(
                "< namespace :: BaseTwo as :: az_core :: type_info :: AzTypeInfo > :: TYPE_ID"
            ),
            "{tokens}"
        );
    }

    #[test]
    fn az_rtti_derive_does_not_register_by_default() {
        let input: DeriveInput = parse_quote! {
            #[az_rtti("44444444-4444-4444-4444-444444444444")]
            struct DemoRtti;
        };

        let tokens = expand_az_rtti(&input)
            .expect("AzRtti expansion should derive identity")
            .to_string();

        assert!(!tokens.contains("const REGISTRATION"), "{tokens}");
        assert!(
            !tokens.contains(":: az_core :: rtti :: AzTypeRegistration"),
            "{tokens}"
        );
    }

    #[test]
    fn az_rtti_derive_registers_when_requested() {
        let tokens = az_rtti_register_tokens();

        assert!(
            tokens.contains(
                "const REGISTRATION : :: az_core :: rtti :: AzTypeRegistration \
                 = :: az_core :: rtti :: AzTypeRegistration :: rtti :: < DemoRtti > ()"
            ),
            "{tokens}"
        );
    }

    #[test]
    fn az_rtti_derive_accepts_named_register_flag() {
        let input: DeriveInput = parse_quote! {
            #[az_rtti(register = true, "44444444-4444-4444-4444-444444444444")]
            struct DemoRtti;
        };

        let tokens = expand_az_rtti(&input)
            .expect("AzRtti expansion should register with named flag")
            .to_string();

        assert!(tokens.contains("const REGISTRATION"), "{tokens}");
    }

    /// A generic template has no single entry value, so it carries no
    /// registration item — and therefore nothing for a crate's enumeration to
    /// miss.
    #[test]
    fn az_rtti_derive_does_not_register_generic_templates() {
        let input: DeriveInput = parse_quote! {
            #[az_rtti("55555555-5555-5555-5555-555555555555", register)]
            struct DemoRtti<T>(T);
        };

        let tokens = expand_az_rtti(&input)
            .expect("AzRtti expansion should support generic templates")
            .to_string();

        assert!(!tokens.contains("const REGISTRATION"), "{tokens}");
    }
}
