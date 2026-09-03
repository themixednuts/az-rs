//! `#[derive(NetworkBase)]` — closed polymorphic-value family enum.
//!
//! Generates `impl NetworkBase` for an enum where each variant either:
//!
//! 1. Wraps a single `T: Class` (no struct fields). The variant's UUID
//!    comes from `AzTypeInfo::TYPE_ID`; its body codec from
//!    `T::marshal` / `T::unmarshal`.
//!
//! 2. Carries `#[network_base(unknown)]` and has the shape
//!    `Variant { uuid: Uuid, body: Vec<u8> }`. This variant captures wire
//!    UUIDs that aren't in [`Self::VARIANTS`] — useful while the codebase
//!    is still being mapped to Rust types.
//!
//! At most one `unknown` variant is allowed; if absent, unknown UUIDs cause
//! a [`MarshalerError::UnknownClassUuid`].
//!
//! # Generated impl shape
//!
//! ```ignore
//! impl NetworkBase for StorageItem {
//!     const VARIANTS: &'static [VariantEntry<Self>] = &[
//!         VariantEntry { uuid: <ContractStorageItem as AzTypeInfo>::TYPE_ID,
//!                        unmarshal: |rb| ContractStorageItem::unmarshal(rb).map(Self::Contract) },
//!         VariantEntry { uuid: <CurrencyStorageItem as AzTypeInfo>::TYPE_ID,
//!                        unmarshal: |rb| CurrencyStorageItem::unmarshal(rb).map(Self::Currency) },
//!     ];
//!
//!     fn uuid(&self) -> Uuid { match self { ... } }
//!     fn marshal_body(&self, wb: &mut WriteBuffer) { match self { ... } }
//!     fn unmarshal_unknown(uuid: Uuid, rb: &mut ReadBuffer) -> Result<Self, _> {
//!         // captures into Unknown { uuid, body }
//!     }
//! }
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericArgument, Ident, PathArguments, Type};

pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Enum(d) => d,
        Data::Struct(s) => {
            return Err(syn::Error::new_spanned(
                s.struct_token,
                "#[derive(NetworkBase)] only supports enums",
            ));
        }
        Data::Union(u) => {
            return Err(syn::Error::new_spanned(
                u.union_token,
                "#[derive(NetworkBase)] only supports enums",
            ));
        }
    };

    let mut known: Vec<KnownVariant> = Vec::new();
    let mut unknown: Option<UnknownVariant> = None;

    for v in &data.variants {
        let is_unknown = v.attrs.iter().any(is_network_base_unknown_attr);
        if is_unknown {
            if unknown.is_some() {
                return Err(syn::Error::new_spanned(
                    &v.ident,
                    "#[derive(NetworkBase)]: only one #[network_base(unknown)] variant allowed",
                ));
            }
            unknown = Some(parse_unknown_variant(&v.ident, &v.fields)?);
        } else {
            known.push(parse_known_variant(&v.ident, &v.fields)?);
        }
    }

    let arms = arms(&known);
    let variants_table = &arms.table;

    let (unknown_uuid_arm, unknown_marshal_arm, unmarshal_unknown_impl) =
        unknown.as_ref().map_or_else(
            || (quote! {}, quote! {}, quote! {}),
            |u| {
                let v = &u.ident;
                (
                    quote! {
                        Self::#v { uuid, .. } => *uuid,
                    },
                    quote! {
                        Self::#v { body, .. } => wb.write_bytes(body),
                    },
                    quote! {
                        fn unmarshal_unknown(
                            uuid: ::uuid::Uuid,
                            rb: &mut ::gridmate::serialize::ReadBuffer<'_>,
                        ) -> ::core::result::Result<Self, ::gridmate::serialize::MarshalerError> {
                            let remaining = rb.left();
                            let bytes = rb.read_bytes(remaining)?;
                            let body = bytes.to_vec();
                            ::core::result::Result::Ok(Self::#v { uuid, body })
                        }
                    },
                )
            },
        );

    // Special-case the zero-variant ("placeholder family") enum: an empty
    // `match self { }` on `&EmptyEnum` doesn't compile because `&T` is
    // treated as inhabited even when `T` isn't. Deref'ing the receiver
    // produces a value of the empty type, which the compiler accepts as
    // exhaustive without arms.
    let (uuid_match, marshal_match) = if known.is_empty() && unknown.is_none() {
        (quote! { match *self {} }, quote! { match *self {} })
    } else {
        let uuid_arms = &arms.uuid;
        let marshal_arms = &arms.marshal;
        (
            quote! {
                match self {
                    #( #uuid_arms )*
                    #unknown_uuid_arm
                }
            },
            quote! {
                match self {
                    #( #marshal_arms )*
                    #unknown_marshal_arm
                }
            },
        )
    };

    Ok(quote! {
        impl #impl_generics ::gridmate::az::NetworkBase for #ident #ty_generics #where_clause {
            const VARIANTS: &'static [::gridmate::az::VariantEntry<Self>] = &[
                #( #variants_table, )*
            ];

            fn uuid(&self) -> ::uuid::Uuid {
                #uuid_match
            }

            fn marshal_body(&self, wb: &mut ::gridmate::serialize::WriteBuffer) {
                let _ = wb;
                #marshal_match
            }

            #unmarshal_unknown_impl
        }
    })
}

/// The three per-variant token lists the generated impl interpolates: the
/// `VARIANTS` table rows, the `uuid()` match arms, and the `marshal_body()`
/// match arms.
struct Arms {
    table: Vec<TokenStream>,
    uuid: Vec<TokenStream>,
    marshal: Vec<TokenStream>,
}

fn arms(known: &[KnownVariant]) -> Arms {
    let table = known
        .iter()
        .map(|k| {
            let v = &k.ident;
            let inner = &k.inner_ty;
            let constructor = if k.boxed {
                // `Box<T>` variant: read T::unmarshal(rb), wrap in Box, then construct Self::Variant.
                quote! { <#inner as ::gridmate::serialize::Marshaler>::unmarshal(rb)
                .map(::std::boxed::Box::new)
                .map(Self::#v) }
            } else {
                quote! { <#inner as ::gridmate::serialize::Marshaler>::unmarshal(rb)
                .map(Self::#v) }
            };
            quote! {
                ::gridmate::az::VariantEntry::<Self> {
                    uuid: <#inner as ::az_core::type_info::AzTypeInfo>::TYPE_ID,
                    type_index: <#inner as ::gridmate::az::Class>::TYPE_INDEX,
                    unmarshal: |rb| { #constructor },
                }
            }
        })
        .collect();

    let uuid = known
        .iter()
        .map(|k| {
            let v = &k.ident;
            let inner = &k.inner_ty;
            quote! {
                Self::#v(_) => <#inner as ::az_core::type_info::AzTypeInfo>::TYPE_ID,
            }
        })
        .collect();

    let marshal = known
        .iter()
        .map(|k| {
            let v = &k.ident;
            // For boxed variants, the blanket `impl<T: Marshaler> Marshaler for
            // Box<T>` carries the call through to the inner `T::marshal_body`;
            // no special-casing needed in the codegen.
            quote! {
                Self::#v(inner) => ::gridmate::serialize::Marshaler::marshal(inner, wb),
            }
        })
        .collect();

    Arms {
        table,
        uuid,
        marshal,
    }
}

struct KnownVariant {
    ident: Ident,
    /// The `T` referenced for AZ type identity and `Class::TYPE_INDEX`.
    /// For a boxed variant `Variant(Box<T>)` this is the unwrapped `T`, not `Box<T>`,
    /// since `Box<T>` doesn't implement [`ClassDesc`].
    inner_ty: Type,
    /// `true` when the variant has the shape `Variant(Box<T>)`. Codegen wraps
    /// the unmarshaled `T` in `Box::new(...)` before constructing the variant;
    /// the marshal path goes through the `impl<T: Marshaler> Marshaler for
    /// Box<T>` blanket without any extra ceremony. Used for breaking type
    /// recursion (e.g. an `ItemVersion` variant inside a family enum that's
    /// also stored inside `ItemVersionData`'s own `data` field).
    boxed: bool,
}

struct UnknownVariant {
    ident: Ident,
}

fn parse_known_variant(ident: &Ident, fields: &Fields) -> syn::Result<KnownVariant> {
    let unnamed = match fields {
        Fields::Unnamed(f) => f,
        Fields::Named(_) => {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "#[derive(NetworkBase)]: variant `{ident}` must be a single-field tuple variant"
                ),
            ));
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                ident,
                format!("#[derive(NetworkBase)]: variant `{ident}` must wrap a Class type"),
            ));
        }
    };
    if unnamed.unnamed.len() != 1 {
        return Err(syn::Error::new_spanned(
            ident,
            format!(
                "#[derive(NetworkBase)]: variant `{ident}` must have exactly one unnamed field"
            ),
        ));
    }
    let raw_ty = unnamed.unnamed.first().unwrap().ty.clone();
    let (inner_ty, boxed) = unwrap_box(&raw_ty).map_or((raw_ty, false), |inner| (inner, true));
    Ok(KnownVariant {
        ident: ident.clone(),
        inner_ty,
        boxed,
    })
}

/// Recognize the syntactic shape `Box<T>` (with any path qualifier:
/// `Box<T>`, `std::boxed::Box<T>`, `::std::boxed::Box<T>`, `alloc::boxed::Box<T>`).
/// Returns the inner `T` if matched. The path-segment match is purely
/// syntactic — a user-defined `Box` in scope would be incorrectly treated
/// as the standard one, which is an accepted limitation of derive macros.
fn unwrap_box(ty: &Type) -> Option<Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    let last = type_path.path.segments.last()?;
    if last.ident != "Box" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner.clone()),
        _ => None,
    }
}

fn parse_unknown_variant(ident: &Ident, fields: &Fields) -> syn::Result<UnknownVariant> {
    let Fields::Named(named) = fields else {
        return Err(syn::Error::new_spanned(
            ident,
            format!(
                "#[network_base(unknown)] variant `{ident}` must be a struct variant with `uuid` and `body` fields"
            ),
        ));
    };
    let idents: Vec<String> = named
        .named
        .iter()
        .filter_map(|f| f.ident.as_ref().map(ToString::to_string))
        .collect();
    let has_uuid = idents.iter().any(|n| n == "uuid");
    let has_body = idents.iter().any(|n| n == "body");
    if !has_uuid || !has_body {
        return Err(syn::Error::new_spanned(
            ident,
            format!(
                "#[network_base(unknown)] variant `{ident}` must have fields `uuid: Uuid` and `body: Vec<u8>`"
            ),
        ));
    }
    Ok(UnknownVariant {
        ident: ident.clone(),
    })
}

fn is_network_base_unknown_attr(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("network_base") {
        return false;
    }
    let mut found = false;
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("unknown") {
            found = true;
        }
        Ok(())
    });
    found
}
