use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    DeriveInput, Error, LitStr, Path, Result, Token, parse_macro_input, punctuated::Punctuated,
};

#[proc_macro_derive(SourceFormat, attributes(source))]
pub fn derive_source_format(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_source_format(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(ProductFormat, attributes(product_format))]
pub fn derive_product_format(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_product_format(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand_source_format(input: &DeriveInput) -> Result<TokenStream2> {
    let ident = &input.ident;
    let attrs = parse_source_attrs(input)?;
    let schema_tokens = attrs.schema.as_ref().map_or_else(
        || quote! { ::std::option::Option::None },
        |schema| {
            quote! {
                ::std::option::Option::Some(
                    ::az_asset_builder::SourceSchemaType::__from_static(#schema)
                )
            }
        },
    );
    let schema_types_tokens = attrs.schema.as_ref().map_or_else(
        || quote! { &[] },
        |schema| quote! { &[::az_asset_builder::SourceSchemaType::__from_static(#schema)] },
    );
    let pattern_literals = attrs.patterns.iter().map(
        |pattern| quote! { ::az_asset_builder::AssetBuilderPattern::static_wildcard(#pattern) },
    );
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::az_asset_builder::SourceFormat for #ident #ty_generics #where_clause {
            const SCHEMA: ::std::option::Option<::az_asset_builder::SourceSchemaType> = #schema_tokens;
            const SCHEMA_TYPES: &'static [::az_asset_builder::SourceSchemaType] = #schema_types_tokens;
            const PATTERNS: &'static [::az_asset_builder::AssetBuilderPattern] = &[
                #(#pattern_literals,)*
            ];
        }
    })
}

fn expand_product_format(input: &DeriveInput) -> Result<TokenStream2> {
    let ident = &input.ident;
    let attrs = parse_product_attrs(input)?;
    let id = attrs
        .id
        .ok_or_else(|| Error::new_spanned(input, "#[product_format(id = \"...\")] is required"))?;
    let version = attrs
        .version
        .ok_or_else(|| Error::new_spanned(input, "#[product_format(version = N)] is required"))?;
    if version == 0 {
        return Err(Error::new_spanned(
            input,
            "#[product_format(version = N)] must be non-zero",
        ));
    }
    let asset = attrs
        .asset
        .ok_or_else(|| Error::new_spanned(input, "#[product_format(asset = Type)] is required"))?;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::az_asset_builder::ProductFormat for #ident #ty_generics #where_clause {
            const ID: ::az_asset_builder::ProductFormatId =
                ::az_asset_builder::ProductFormatId::__from_static(#id);
            const VERSION: u32 = #version;
            type Asset = #asset;
        }
    })
}

#[derive(Default)]
struct SourceAttrs {
    schema: Option<String>,
    patterns: Vec<String>,
}

#[derive(Default)]
struct ProductAttrs {
    id: Option<String>,
    version: Option<u32>,
    asset: Option<Path>,
}

fn parse_source_attrs(input: &DeriveInput) -> Result<SourceAttrs> {
    let mut out = SourceAttrs::default();

    for attr in &input.attrs {
        if !attr.path().is_ident("source") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("schema") {
                set_once(
                    &mut out.schema,
                    parse_non_empty_string(&meta, "source schema")?,
                    &meta,
                    "source schema",
                )?;
            } else if meta.path.is_ident("ext") {
                let extension = parse_non_empty_string(&meta, "source extension")?;
                out.patterns.push(extension_pattern(&extension, &meta)?);
            } else if meta.path.is_ident("pattern") || meta.path.is_ident("wildcard") {
                out.patterns
                    .push(parse_non_empty_string(&meta, "source wildcard pattern")?);
            } else {
                return Err(meta.error("unsupported source attribute"));
            }
            Ok(())
        })?;
    }

    Ok(out)
}

fn parse_product_attrs(input: &DeriveInput) -> Result<ProductAttrs> {
    let mut out = ProductAttrs::default();

    for attr in &input.attrs {
        if !attr.path().is_ident("product_format") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                set_once(
                    &mut out.id,
                    parse_non_empty_string(&meta, "product format id")?,
                    &meta,
                    "product format id",
                )?;
            } else if meta.path.is_ident("version") {
                set_once(
                    &mut out.version,
                    parse_u32(&meta)?,
                    &meta,
                    "product format version",
                )?;
            } else if meta.path.is_ident("asset") {
                let value = meta.value()?;
                let asset = if value.peek(LitStr) {
                    let text: LitStr = value.parse()?;
                    syn::parse_str::<Path>(&text.value()).map_err(|error| {
                        Error::new(text.span(), format!("invalid asset type path: {error}"))
                    })?
                } else {
                    value.parse::<Path>()?
                };
                set_once(&mut out.asset, asset, &meta, "product format asset")?;
            } else {
                return Err(meta.error("unsupported product_format attribute"));
            }
            Ok(())
        })?;
    }

    Ok(out)
}

fn parse_non_empty_string(
    meta: &syn::meta::ParseNestedMeta<'_>,
    what: &'static str,
) -> Result<String> {
    let value = meta.value()?;
    let literal: LitStr = value.parse()?;
    let text = literal.value();
    if text.is_empty() {
        return Err(Error::new(
            literal.span(),
            format!("{what} must not be empty"),
        ));
    }
    Ok(text)
}

fn parse_u32(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<u32> {
    let value = meta.value()?;
    let literal: syn::LitInt = value.parse()?;
    literal.base10_parse()
}

fn extension_pattern(extension: &str, meta: &syn::meta::ParseNestedMeta<'_>) -> Result<String> {
    if extension.starts_with('.') || extension.contains('/') || extension.contains('\\') {
        return Err(meta.error("source extension must not include a leading dot or path separator"));
    }
    Ok(format!("*.{}", extension.to_ascii_lowercase()))
}

fn set_once<T>(
    target: &mut Option<T>,
    value: T,
    meta: &syn::meta::ParseNestedMeta<'_>,
    what: &'static str,
) -> Result<()> {
    if target.is_some() {
        return Err(meta.error(format!("duplicate {what}")));
    }
    *target = Some(value);
    Ok(())
}

#[allow(dead_code)]
fn parse_comma_separated_strings(input: syn::parse::ParseStream<'_>) -> Result<Vec<String>> {
    let values = Punctuated::<LitStr, Token![,]>::parse_terminated(input)?;
    Ok(values.into_iter().map(|value| value.value()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens as _;
    use syn::parse_quote;

    #[test]
    fn source_format_derives_schema_and_patterns() {
        let input: DeriveInput = parse_quote! {
            #[source(schema = "az.test.Source", ext = "foo.ron", pattern = "special/*")]
            struct TestSource;
        };

        let output = expand_source_format(&input)
            .unwrap()
            .to_token_stream()
            .to_string();

        assert!(output.contains("az.test.Source"), "{output}");
        assert!(output.contains("*.foo.ron"), "{output}");
        assert!(output.contains("special/*"), "{output}");
        assert!(output.contains("SourceFormat"), "{output}");
    }

    #[test]
    fn product_format_derives_id_version_and_asset() {
        let input: DeriveInput = parse_quote! {
            #[product_format(id = "az.test.product", version = 1, asset = crate::TestAsset)]
            struct TestProduct;
        };

        let output = expand_product_format(&input)
            .unwrap()
            .to_token_stream()
            .to_string();

        assert!(output.contains("az.test.product"), "{output}");
        assert!(output.contains("const VERSION : u32 = 1"), "{output}");
        assert!(
            output.contains("type Asset = crate :: TestAsset"),
            "{output}"
        );
    }

    #[test]
    fn product_format_rejects_zero_version() {
        let input: DeriveInput = parse_quote! {
            #[product_format(id = "az.test.product", version = 0, asset = crate::TestAsset)]
            struct TestProduct;
        };

        let error = expand_product_format(&input).unwrap_err();

        assert!(error.to_string().contains("must be non-zero"), "{error}");
    }
}
