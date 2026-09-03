//! Derive macros for `GameData` authored row schemas.

#![forbid(unsafe_code)]

use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Lit, Result, parse_macro_input};

#[proc_macro_derive(GameDataRow, attributes(key, row))]
pub fn derive_game_data_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_game_data_row(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand_game_data_row(input: &DeriveInput) -> Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "#[derive(GameDataRow)] does not support generic row types",
        ));
    }

    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "#[derive(GameDataRow)] is only valid on structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            &data.fields,
            "#[derive(GameDataRow)] requires named fields",
        ));
    };

    let attrs = parse_type_attrs(&input.attrs)?;
    let row_name = attrs.row_name.unwrap_or_else(|| {
        input
            .ident
            .to_string()
            .trim_start_matches("r#")
            .to_upper_camel_case()
    });

    let mut key_fields = Vec::new();
    for field in &fields.named {
        let attrs = parse_field_attrs(&field.attrs)?;
        if attrs.primary_key {
            let ident = field.ident.as_ref().expect("named field").clone();
            let field_name = ident.to_string();
            key_fields.push(KeyField { name: field_name });
        }
    }

    let ident = &input.ident;
    let key_names = key_fields.iter().map(|key| &key.name);

    Ok(quote! {
        impl ::gamedata::Row for #ident {
            const NAME: &'static str = #row_name;
        }

        impl ::gamedata::GameDataRow for #ident {
            const KEY_FIELD_NAMES: &'static [&'static str] = &[#(#key_names),*];
        }
    })
}

#[derive(Default)]
struct TypeAttrs {
    row_name: Option<String>,
}

#[derive(Default)]
struct FieldAttrs {
    primary_key: bool,
}

struct KeyField {
    name: String,
}

fn parse_type_attrs(attrs: &[Attribute]) -> Result<TypeAttrs> {
    let mut parsed = TypeAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("row")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                if parsed.row_name.is_some() {
                    return Err(meta.error("duplicate #[row(name = \"...\")] attribute"));
                }
                parsed.row_name = Some(parse_string_value(&meta)?);
                return Ok(());
            }
            Err(meta.error("unsupported #[row(...)] attribute"))
        })?;
    }
    Ok(parsed)
}

fn parse_field_attrs(attrs: &[Attribute]) -> Result<FieldAttrs> {
    let mut parsed = FieldAttrs::default();
    for attr in attrs {
        if attr.path().is_ident("key") {
            if parsed.primary_key {
                return Err(Error::new_spanned(attr, "duplicate #[key] attribute"));
            }
            attr.meta.require_path_only()?;
            parsed.primary_key = true;
        }
    }
    Ok(parsed)
}

fn parse_string_value(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<String> {
    let value = meta.value()?;
    let lit: Lit = value.parse()?;
    match lit {
        Lit::Str(value) => Ok(value.value()),
        _ => Err(meta.error("expected string literal")),
    }
}
