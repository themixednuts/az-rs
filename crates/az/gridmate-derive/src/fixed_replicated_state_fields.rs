//! `#[derive(FixedReplicatedStateFields)]` — emits the Rust adapter for
//! source-shaped `MB::FixedReplicatedState<...>` fragments.

use std::collections::BTreeMap;

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    AngleBracketedGenericArguments, Data, DeriveInput, Field, Fields, GenericArgument, Ident,
    LitInt, PathArguments, Type,
};

struct ConstArgs {
    n_groups: TokenStream,
    n_fields_per_group: TokenStream,
    client_whitelist_size: TokenStream,
    n_user_attributes: TokenStream,
}

struct BaseField {
    ident: Ident,
    const_args: ConstArgs,
}

struct FieldInfo {
    ident: Ident,
    ty: Type,
}

struct FieldAttrs {
    skip: bool,
    group: usize,
}

pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "#[derive(FixedReplicatedStateFields)] only supports structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "#[derive(FixedReplicatedStateFields)] requires named fields",
        ));
    };

    let (base_field, groups) = partition(input, fields)?;

    let ident = &input.ident;
    let base_ident = &base_field.ident;
    let const_args = &base_field.const_args;
    let n_groups = &const_args.n_groups;
    let n_fields_per_group = &const_args.n_fields_per_group;
    let client_whitelist_size = &const_args.client_whitelist_size;
    let n_user_attributes = &const_args.n_user_attributes;

    let arms = arms(&groups, n_fields_per_group);
    let group_count_arms = &arms.count;
    let visit_arms = &arms.visit;
    let visit_mut_arms = &arms.visit_mut;
    let visit_merge_arms = &arms.merge;
    let accessors = accessors(base_ident, const_args);

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::gridmate::hub::FixedReplicatedStateFields<
            #n_groups,
            #n_fields_per_group,
            #client_whitelist_size,
            #n_user_attributes,
        > for #ident #ty_generics #where_clause
        {
            #accessors

            fn fixed_group_field_count(&self, group_idx: usize) -> ::core::option::Option<usize> {
                match group_idx {
                    #(#group_count_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn visit_fixed_fields<'a>(
                &'a self,
                group_idx: usize,
                mut visit: impl FnMut(usize, &'a dyn ::gridmate::ReplicatedFieldHandlerBase),
            ) {
                match group_idx {
                    #(#visit_arms,)*
                    _ => {}
                }
            }

            fn try_visit_fixed_fields_mut(
                &mut self,
                group_idx: usize,
                mut visit: impl FnMut(
                    usize,
                    &mut dyn ::gridmate::ReplicatedFieldHandlerBase,
                ) -> ::core::result::Result<(), ::gridmate::serialize::MarshalerError>,
            ) -> ::core::result::Result<(), ::gridmate::serialize::MarshalerError> {
                match group_idx {
                    #(#visit_mut_arms,)*
                    _ => ::core::result::Result::Ok(()),
                }
            }

            fn try_visit_fixed_fields_for_merge(
                &mut self,
                old_state: &Self,
                new_state: &mut Self,
                group_idx: usize,
                mut visit: impl FnMut(
                    usize,
                    &mut dyn ::gridmate::ReplicatedFieldHandlerBase,
                    &dyn ::gridmate::ReplicatedFieldHandlerBase,
                    &mut dyn ::gridmate::ReplicatedFieldHandlerBase,
                ) -> ::core::result::Result<(), ::gridmate::serialize::MarshalerError>,
            ) -> ::core::result::Result<(), ::gridmate::serialize::MarshalerError>
            where
                Self: Sized,
            {
                match group_idx {
                    #(#visit_merge_arms,)*
                    _ => ::core::result::Result::Ok(()),
                }
            }
        }
    })
}

/// Split the struct's named fields into the embedded `FixedReplicatedState`
/// base and the per-group typed field lists.
fn partition(
    input: &DeriveInput,
    fields: &syn::FieldsNamed,
) -> syn::Result<(BaseField, BTreeMap<usize, Vec<FieldInfo>>)> {
    let mut base_field = None;
    let mut groups: BTreeMap<usize, Vec<FieldInfo>> = BTreeMap::new();
    for field in &fields.named {
        let Some(ident) = &field.ident else {
            continue;
        };
        if let Some(const_args) = fixed_replicated_state_const_args(&field.ty)? {
            if base_field.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "only one gridmate::hub::FixedReplicatedState<...> base field is allowed",
                ));
            }
            base_field = Some(BaseField {
                ident: ident.clone(),
                const_args,
            });
            continue;
        }
        let attrs = parse_field_attrs(field)?;
        if attrs.skip {
            continue;
        }
        groups.entry(attrs.group).or_default().push(FieldInfo {
            ident: ident.clone(),
            ty: field.ty.clone(),
        });
    }

    let Some(base_field) = base_field else {
        return Err(syn::Error::new_spanned(
            input,
            "#[derive(FixedReplicatedStateFields)] requires an embedded \
             `gridmate::hub::FixedReplicatedState<...>` base field",
        ));
    };

    Ok((base_field, groups))
}

/// The four `group_idx` match-arm sets the generated impl dispatches on.
struct Arms {
    count: Vec<TokenStream>,
    visit: Vec<TokenStream>,
    visit_mut: Vec<TokenStream>,
    merge: Vec<TokenStream>,
}

fn arms(groups: &BTreeMap<usize, Vec<FieldInfo>>, limit: &TokenStream) -> Arms {
    let count = groups
        .iter()
        .map(|(group_idx, fields)| {
            let field_count = field_count_expr(fields);
            quote! {
                #group_idx => {
                    debug_assert!(#field_count <= #limit);
                    ::core::option::Option::Some(#field_count)
                }
            }
        })
        .collect();
    let visit = groups
        .iter()
        .map(|(group_idx, fields)| {
            let body = expand_visit_fields(fields);
            quote! {
                #group_idx => {
                    #body
                }
            }
        })
        .collect();
    let visit_mut = groups
        .iter()
        .map(|(group_idx, fields)| {
            let body = expand_visit_fields_mut(fields);
            quote! {
                #group_idx => {
                    #body
                    ::core::result::Result::Ok(())
                }
            }
        })
        .collect();
    let merge = groups
        .iter()
        .map(|(group_idx, fields)| {
            let body = expand_visit_fields_for_merge(fields);
            quote! {
                #group_idx => {
                    #body
                    ::core::result::Result::Ok(())
                }
            }
        })
        .collect();

    Arms {
        count,
        visit,
        visit_mut,
        merge,
    }
}

/// The two base accessors, whose signatures repeat the base field's const args.
fn accessors(base: &Ident, args: &ConstArgs) -> TokenStream {
    let ConstArgs {
        n_groups,
        n_fields_per_group,
        client_whitelist_size,
        n_user_attributes,
    } = args;
    quote! {
        fn fixed_replicated_state(
            &self,
        ) -> &::gridmate::hub::FixedReplicatedState<
            #n_groups,
            #n_fields_per_group,
            #client_whitelist_size,
            #n_user_attributes,
        > {
            &self.#base
        }

        fn fixed_replicated_state_mut(
            &mut self,
        ) -> &mut ::gridmate::hub::FixedReplicatedState<
            #n_groups,
            #n_fields_per_group,
            #client_whitelist_size,
            #n_user_attributes,
        > {
            &mut self.#base
        }
    }
}

fn parse_field_attrs(field: &Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs {
        skip: false,
        group: 0,
    };

    for attr in &field.attrs {
        if !attr.path().is_ident("fixed_state") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                attrs.skip = true;
                Ok(())
            } else if meta.path.is_ident("group") {
                let value = meta.value()?;
                let lit: LitInt = value.parse()?;
                attrs.group = lit.base10_parse()?;
                Ok(())
            } else {
                Err(meta.error("unsupported fixed_state field attribute"))
            }
        })?;
    }

    Ok(attrs)
}

fn fixed_replicated_state_const_args(base_ty: &Type) -> syn::Result<Option<ConstArgs>> {
    let Type::Path(type_path) = base_ty else {
        return Ok(None);
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Ok(None);
    };
    if segment.ident != "FixedReplicatedState" {
        return Ok(None);
    }
    let PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) =
        &segment.arguments
    else {
        return Err(syn::Error::new_spanned(
            base_ty,
            "FixedReplicatedState base field must include const args",
        ));
    };

    let mut const_args = Vec::new();
    for arg in args {
        match arg {
            GenericArgument::Const(_) => const_args.push(arg.to_token_stream()),
            _ => {
                return Err(syn::Error::new_spanned(
                    arg,
                    "FixedReplicatedState generic arguments must be const arguments",
                ));
            }
        }
    }

    if const_args.len() < 2 || const_args.len() > 4 {
        return Err(syn::Error::new_spanned(
            base_ty,
            "FixedReplicatedState requires 2 to 4 const arguments",
        ));
    }

    Ok(Some(ConstArgs {
        n_groups: const_args[0].clone(),
        n_fields_per_group: const_args[1].clone(),
        client_whitelist_size: const_args.get(2).cloned().unwrap_or_else(|| quote!(0usize)),
        n_user_attributes: const_args.get(3).cloned().unwrap_or_else(|| quote!(0usize)),
    }))
}

fn field_count_expr(fields: &[FieldInfo]) -> TokenStream {
    let counts = fields.iter().map(|field| {
        let ty = &field.ty;
        quote!(<#ty as ::gridmate::hub::FixedStateRegister>::FIELD_COUNT)
    });
    quote!(0usize #(+ #counts)*)
}

fn expand_visit_fields(fields: &[FieldInfo]) -> TokenStream {
    let visits = fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        quote! {
            <#ty as ::gridmate::hub::FixedStateRegister>::visit_registered_fields(
                &self.#ident,
                first_index,
                &mut visit,
            );
            first_index += <#ty as ::gridmate::hub::FixedStateRegister>::FIELD_COUNT;
        }
    });
    quote! {
        let mut first_index = 0usize;
        #(#visits)*
        let _ = first_index;
    }
}

fn expand_visit_fields_mut(fields: &[FieldInfo]) -> TokenStream {
    let visits = fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        quote! {
            <#ty as ::gridmate::hub::FixedStateRegister>::try_visit_registered_fields_mut(
                &mut self.#ident,
                first_index,
                &mut visit,
            )?;
            first_index += <#ty as ::gridmate::hub::FixedStateRegister>::FIELD_COUNT;
        }
    });
    quote! {
        let mut first_index = 0usize;
        #(#visits)*
        let _ = first_index;
    }
}

fn expand_visit_fields_for_merge(fields: &[FieldInfo]) -> TokenStream {
    let visits = fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        quote! {
            <#ty as ::gridmate::hub::FixedStateRegister>::try_visit_registered_fields_for_merge(
                &mut self.#ident,
                &old_state.#ident,
                &mut new_state.#ident,
                first_index,
                &mut visit,
            )?;
            first_index += <#ty as ::gridmate::hub::FixedStateRegister>::FIELD_COUNT;
        }
    });
    quote! {
        let mut first_index = 0usize;
        #(#visits)*
        let _ = first_index;
    }
}
