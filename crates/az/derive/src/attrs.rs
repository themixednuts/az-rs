use quote::quote;
use syn::parse::{ParseStream, Parser};
use syn::spanned::Spanned;
use syn::{Attribute, DeriveInput, Error, Expr, LitBool, LitStr, Meta, Path, Result, Token, Type};

pub struct ParsedAzAttrs {
    pub name: String,
    pub trait_path: Option<Path>,
    pub bases: Vec<Type>,
    pub register: bool,
    type_id_tokens: Option<proc_macro2::TokenStream>,
}

pub struct AzAttrs {
    pub name: String,
    pub trait_path: Option<Path>,
    pub bases: Vec<Type>,
    pub register: bool,
    pub type_id: proc_macro2::TokenStream,
}

impl ParsedAzAttrs {
    pub fn into_validated(self, input: &DeriveInput) -> Result<AzAttrs> {
        let type_id = self.type_id_tokens.ok_or_else(|| {
            Error::new_spanned(
                &input.ident,
                "missing required AZ type id; use a positional UUID like `#[az_type_info(\"...\")]`",
            )
        })?;

        Ok(AzAttrs {
            name: self.name,
            trait_path: self.trait_path,
            bases: self.bases,
            register: self.register,
            type_id,
        })
    }
}

impl AzAttrs {
    pub fn trait_path_or_default(
        &self,
        default: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        self.trait_path
            .as_ref()
            .map_or(default, |path| quote!(#path))
    }
}

pub fn parse_az_attrs(
    input: &DeriveInput,
    attr_names: &[&str],
    allow_bases: bool,
) -> Result<ParsedAzAttrs> {
    let ident = &input.ident;
    let mut parsed = ParsedAzAttrs {
        name: ident.to_string(),
        trait_path: None,
        bases: Vec::new(),
        register: false,
        type_id_tokens: None,
    };

    for attr in &input.attrs {
        if !attr_names.iter().any(|name| attr.path().is_ident(name)) {
            continue;
        }

        parse_attr(attr, &mut parsed, allow_bases)?;
    }

    Ok(parsed)
}

pub fn az_core_path(module: &str, item: &str) -> proc_macro2::TokenStream {
    let module = syn::Ident::new(module, proc_macro2::Span::call_site());
    let item = syn::Ident::new(item, proc_macro2::Span::call_site());
    if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("az-core") {
        quote!(crate::#module::#item)
    } else {
        quote!(::az_core::#module::#item)
    }
}

fn parse_attr(attr: &Attribute, parsed: &mut ParsedAzAttrs, allow_bases: bool) -> Result<()> {
    let Meta::List(list) = &attr.meta else {
        return Ok(());
    };

    let parser = |input: ParseStream<'_>| {
        while !input.is_empty() {
            if input.peek(syn::Ident) && input.peek2(Token![=]) {
                parse_named_arg(input, parsed)?;
            } else if input.peek(syn::Ident) && is_register_flag(input)? {
                parsed.register = true;
            } else if parsed.type_id_tokens.is_none() {
                let type_id = parse_type_id_expr(input)?;
                set_type_id(parsed, type_id, attr.path().span())?;
            } else {
                let base = input.parse::<Type>()?;
                if !allow_bases {
                    return Err(Error::new_spanned(
                        base,
                        "extra positional arguments are only supported by AzRtti and AzComponent",
                    ));
                }
                parsed.bases.push(base);
            }

            if input.peek(Token![,]) {
                let _comma: Token![,] = input.parse()?;
            } else if !input.is_empty() {
                return Err(input.error("expected comma"));
            }
        }
        Ok(())
    };

    parser.parse2(list.tokens.clone())
}

fn parse_named_arg(input: ParseStream<'_>, parsed: &mut ParsedAzAttrs) -> Result<()> {
    let key: syn::Ident = input.parse()?;
    let _eq: Token![=] = input.parse()?;
    if key == "name" {
        let lit: LitStr = input.parse()?;
        parsed.name = lit.value();
        Ok(())
    } else if key == "path" {
        let lit: LitStr = input.parse()?;
        parsed.trait_path = Some(lit.parse()?);
        Ok(())
    } else if key == "register" {
        let lit: LitBool = input.parse()?;
        parsed.register = lit.value;
        Ok(())
    } else {
        Err(Error::new(
            key.span(),
            "unsupported attribute; expected optional `name`, optional `path`, optional `register`, positional UUID, then bases",
        ))
    }
}

fn is_register_flag(input: ParseStream<'_>) -> Result<bool> {
    let fork = input.fork();
    let ident: syn::Ident = fork.parse()?;
    if ident != "register" {
        return Ok(false);
    }
    let _: syn::Ident = input.parse()?;
    Ok(true)
}

fn parse_type_id_expr(input: ParseStream<'_>) -> Result<proc_macro2::TokenStream> {
    if input.peek(LitStr) {
        let lit: LitStr = input.parse()?;
        let value = lit.value();
        let normalized = value
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(&value);
        let normalized = LitStr::new(normalized, lit.span());
        Ok(quote! { ::uuid::uuid!(#normalized) })
    } else {
        let expr: Expr = input.parse()?;
        Ok(quote! { #expr })
    }
}

fn set_type_id(
    parsed: &mut ParsedAzAttrs,
    type_id: proc_macro2::TokenStream,
    span: proc_macro2::Span,
) -> Result<()> {
    if parsed.type_id_tokens.is_some() {
        return Err(Error::new(span, "duplicate AZ type id"));
    }
    parsed.type_id_tokens = Some(type_id);
    Ok(())
}
