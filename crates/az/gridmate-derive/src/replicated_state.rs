//! `#[derive(ReplicatedState)]` — emits source `MB::ReplicatedState` glue.
//!
//! It emits the contents, `Fragment`, and `Marshaler` impls. It does not
//! register the fragment: `FragmentRegistration::of::<T>()` is published by the
//! crate that declares the type, through a contribution's registrar, so a host
//! decodes exactly the fragments it composed. That also retires the old
//! "registers only when `#[class_desc]` is present, and never for generics"
//! rule — a silent skip that a registration site cannot express by accident.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, Ident, LitInt, LitStr, Type};

struct Opts {
    category: Option<String>,
    category_field: Option<String>,
    metadata: bool,
    world_position: Option<String>,
    skip_groups: BTreeSet<usize>,
}

struct FieldInfo {
    ident: Ident,
    source_name: LitStr,
    ty: Type,
}

struct BaseField {
    ident: Ident,
}

pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let opts = parse_opts(input)?;

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "#[derive(ReplicatedState)] only supports structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "#[derive(ReplicatedState)] requires named fields",
        ));
    };

    let mut groups: BTreeMap<usize, Vec<FieldInfo>> = BTreeMap::new();
    let mut attributes = Vec::new();
    for field in &fields.named {
        if field_is_base(field) {
            continue;
        }
        if field_is_skipped(field) {
            continue;
        }
        let Some(ident) = &field.ident else {
            continue;
        };
        let attrs = parse_field_attrs(field)?;
        let info = FieldInfo {
            ident: ident.clone(),
            source_name: attrs.source_name,
            ty: field.ty.clone(),
        };
        if attrs.attribute {
            attributes.push(info);
        } else {
            groups.entry(attrs.group).or_default().push(info);
        }
    }

    if groups.is_empty() && attributes.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "#[derive(ReplicatedState)] requires at least one field",
        ));
    }

    let ident = &input.ident;
    let base_field = find_base_field(ident, fields.named.iter())?;
    for group in &opts.skip_groups {
        if groups.contains_key(group) {
            return Err(syn::Error::new_spanned(
                ident,
                format!("replicated_state skip_group {group} also has fields"),
            ));
        }
    }

    let contents_impl =
        expand_generated_contents(ident, &groups, &opts.skip_groups, &input.generics);
    let fragment_impl = expand_fragment(
        ident,
        &base_field,
        &groups,
        &attributes,
        &opts,
        &input.generics,
    )?;

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let marshaler_impl = quote! {
        impl #impl_generics ::gridmate::serialize::marshaler::Marshaler
        for #ident #ty_generics #where_clause
        {
            fn marshal(&self, wb: &mut ::gridmate::serialize::buffer::WriteBuffer) {
                ::gridmate::hub::Fragment::marshal_contents(self, wb);
            }

            fn unmarshal(
                rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
            ) -> ::core::result::Result<Self, ::gridmate::serialize::error::MarshalerError> {
                let mut value = Self::default();
                if !rb.is_empty() {
                    ::gridmate::hub::Fragment::unmarshal_contents(&mut value, rb)?;
                }
                Ok(value)
            }
        }
    };
    Ok(quote! {
        #contents_impl
        #fragment_impl
        #marshaler_impl
    })
}

fn parse_opts(input: &DeriveInput) -> syn::Result<Opts> {
    let mut opts = Opts {
        category: None,
        category_field: None,
        metadata: false,
        world_position: None,
        skip_groups: BTreeSet::new(),
    };

    for attr in &input.attrs {
        if !attr.path().is_ident("replicated_state") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("facet") {
                Err(meta.error(
                    "ReplicatedState no longer owns facet metadata; replicated state is an IFragment, not an IMessage",
                ))
            } else if meta.path.is_ident("message") {
                Err(meta.error(
                    "`message` is obsolete; ReplicatedState derives only gridmate Fragment/Marshaler glue",
                ))
            } else if meta.path.is_ident("category") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                opts.category = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("category_field") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                opts.category_field = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("metadata") {
                opts.metadata = true;
                Ok(())
            } else if meta.path.is_ident("world_position") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                opts.world_position = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("skip_group") {
                let value = meta.value()?;
                let lit: LitInt = value.parse()?;
                opts.skip_groups.insert(lit.base10_parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported replicated_state attribute"))
            }
        })?;
    }

    if opts.category.is_some() && opts.category_field.is_some() {
        return Err(syn::Error::new_spanned(
            input,
            "replicated_state category and category_field are mutually exclusive",
        ));
    }
    if opts.category_field.is_some() && !opts.metadata {
        return Err(syn::Error::new_spanned(
            input,
            "replicated_state category_field requires metadata",
        ));
    }

    Ok(opts)
}

fn field_is_skipped(field: &syn::Field) -> bool {
    field_has_attr(field, "skip")
}

fn field_is_base(field: &Field) -> bool {
    let Type::Path(type_path) = &field.ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "ReplicatedState")
}

fn field_has_attr(field: &Field, name: &str) -> bool {
    for attr in &field.attrs {
        if !attr.path().is_ident("replicated_state") {
            continue;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(name) {
                found = true;
                Ok(())
            } else if meta.path.is_ident("group") {
                let value = meta.value()?;
                let _: LitInt = value.parse()?;
                Ok(())
            } else if meta.path.is_ident("name") {
                let value = meta.value()?;
                let _: LitStr = value.parse()?;
                Ok(())
            } else if meta.path.is_ident("attribute") {
                Ok(())
            } else {
                Err(meta.error("unsupported replicated_state field attribute"))
            }
        });
        if found {
            return true;
        }
    }
    false
}

fn find_base_field<'a, I>(ident: &Ident, fields: I) -> syn::Result<BaseField>
where
    I: Iterator<Item = &'a Field>,
{
    let mut base = None;
    for field in fields {
        if !field_is_base(field) {
            continue;
        }
        let Some(field_ident) = &field.ident else {
            continue;
        };
        if base.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "only one gridmate::hub::ReplicatedState base field is allowed",
            ));
        }
        base = Some(BaseField {
            ident: field_ident.clone(),
        });
    }

    let Some(base_field) = base else {
        return Err(syn::Error::new_spanned(
            ident,
            "#[derive(ReplicatedState)] requires an embedded \
             `gridmate::hub::ReplicatedState` base field",
        ));
    };

    Ok(base_field)
}

struct FieldAttrs {
    group: usize,
    source_name: LitStr,
    attribute: bool,
}

fn parse_field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut group = 0usize;
    let mut group_is_explicit = false;
    let ident = field
        .ident
        .as_ref()
        .expect("ReplicatedState named field checked before attr parse");
    let mut source_name = LitStr::new(&ident.to_string(), ident.span());
    let mut attribute = false;

    for attr in &field.attrs {
        if !attr.path().is_ident("replicated_state") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("group") {
                let value = meta.value()?;
                let lit: LitInt = value.parse()?;
                group = lit.base10_parse()?;
                group_is_explicit = true;
                Ok(())
            } else if meta.path.is_ident("name") {
                let value = meta.value()?;
                source_name = value.parse()?;
                Ok(())
            } else if meta.path.is_ident("skip") {
                Ok(())
            } else if meta.path.is_ident("attribute") {
                attribute = true;
                Ok(())
            } else {
                Err(meta.error("unsupported replicated_state field attribute"))
            }
        })?;
    }

    if attribute && group_is_explicit {
        return Err(syn::Error::new_spanned(
            field,
            "replicated_state attribute fields do not belong to content groups",
        ));
    }

    Ok(FieldAttrs {
        group,
        source_name,
        attribute,
    })
}

fn expand_generated_contents(
    ident: &Ident,
    groups: &BTreeMap<usize, Vec<FieldInfo>>,
    skip_groups: &BTreeSet<usize>,
    generics: &syn::Generics,
) -> TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let max_descriptor = groups
        .keys()
        .chain(skip_groups.iter())
        .copied()
        .max()
        .unwrap_or(0);
    let descriptor_count = max_descriptor + 1;
    let unmarshal_chunks = unmarshal(groups, skip_groups, max_descriptor);
    let marshal_chunks = marshal(groups, max_descriptor);
    let handlers = handlers(groups);
    let dirty_groups = &handlers.dirty;
    let reset_fields = &handlers.reset;
    let merge_fields = &handlers.merge;
    let metadata = metadata(groups);
    let marshal_metadata_fields = &metadata.marshal;
    let unmarshal_metadata_fields = &metadata.unmarshal;

    quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            fn __replicated_state_unmarshal_fields(
                &mut self,
                rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
            ) -> ::core::result::Result<(), ::gridmate::serialize::error::MarshalerError> {
                #(#unmarshal_chunks)*
                Ok(())
            }

            fn __replicated_state_marshal_fields(
                &self,
                mc: &::gridmate::hub::MarshalContext<'_>,
                wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
            ) -> bool {
                use ::gridmate::serialize::marshaler::Marshaler as _;

                let mut descriptor_dirty = [false; #descriptor_count];
                #(#dirty_groups)*
                if !descriptor_dirty.iter().any(|dirty| *dirty) {
                    return false;
                }
                #(#marshal_chunks)*
                true
            }

            fn __replicated_state_reset_has_new_network_data(&mut self) {
                #(#reset_fields)*
            }

            fn __replicated_state_merge_fields(
                &self,
                new_state: &mut Self,
                merged_state: &mut Self,
                seq: ::gridmate::hub::SequenceNumber,
                inherit_previous_network_data_status: bool,
                outcome: &mut ::gridmate::hub::ReplicatedMergeOutcome,
            ) {
                #(#merge_fields)*
            }

            fn __replicated_state_marshal_field_metadata(
                &self,
                wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
            ) -> bool {
                use ::gridmate::serialize::marshaler::Marshaler as _;
                #(#marshal_metadata_fields)*
                true
            }

            fn __replicated_state_unmarshal_field_metadata(
                &mut self,
                rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
            ) -> ::core::result::Result<bool, ::gridmate::serialize::error::MarshalerError> {
                use ::gridmate::serialize::marshaler::Marshaler as _;
                #(#unmarshal_metadata_fields)*
                Ok(true)
            }
        }
    }
}

/// Per-descriptor-chunk unmarshal blocks: one `descriptor_mask` byte per eight
/// groups, then the bitmasked field reads for the groups in that chunk.
fn unmarshal(
    groups: &BTreeMap<usize, Vec<FieldInfo>>,
    skip_groups: &BTreeSet<usize>,
    max_descriptor: usize,
) -> Vec<TokenStream> {
    let unmarshal_group = |group_idx: &usize, fields: &Vec<FieldInfo>| {
        let field_count = fields.len();
        let field_unmarshal = fields.iter().map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;
            quote! {
                if !descriptor_done {
                    if field_index % 7 == 0 {
                        field_mask = rb.read_u8()?;
                    }
                    if (field_mask & (1 << (field_index % 7))) != 0 {
                        self.#ident =
                            <#ty as ::gridmate::serialize::marshaler::Marshaler>::unmarshal(rb)?;
                    }
                    if (field_index % 7 == 6 || field_index + 1 == field_count)
                        && (field_mask & 0x80) == 0
                    {
                        descriptor_done = true;
                    }
                }
                field_index += 1;
            }
        });
        let group_bit = 1u8 << (group_idx % 8);

        quote! {
            if (descriptor_mask & #group_bit) != 0 {
                let field_count = #field_count;
                let mut field_mask = 0u8;
                let mut field_index = 0usize;
                let mut descriptor_done = false;
                #(#field_unmarshal)*
                if field_count != 0 && field_count % 7 == 0 && !descriptor_done {
                    loop {
                        let mask = rb.read_u8()?;
                        if (mask & 0x80) == 0 {
                            break;
                        }
                    }
                }
                let _ = (field_index, descriptor_done);
            }
        }
    };
    (0..=(max_descriptor / 8))
        .map(|chunk_idx| {
            let chunk_start = chunk_idx * 8;
            let chunk_end = chunk_start + 8;
            let skipped_in_chunk = skip_groups
                .iter()
                .filter(|group_idx| **group_idx >= chunk_start && **group_idx < chunk_end)
                .map(|group_idx| {
                    let bit = 1u8 << (group_idx % 8);
                    quote! {
                        if (descriptor_mask & #bit) != 0 {
                            ::gridmate::serialize::MaskChain::skip(rb)?;
                        }
                    }
                });
            let groups_in_chunk = groups
                .iter()
                .filter(|(group_idx, _)| **group_idx >= chunk_start && **group_idx < chunk_end)
                .map(|(group_idx, fields)| unmarshal_group(group_idx, fields));
            quote! {
                let descriptor_mask = rb.read_u8()?;
                #(#skipped_in_chunk)*
                #(#groups_in_chunk)*
            }
        })
        .collect()
}

/// Per-descriptor-chunk marshal blocks: the dirty-group descriptor byte
/// followed by each dirty group's field masks and payloads.
fn marshal(groups: &BTreeMap<usize, Vec<FieldInfo>>, max_descriptor: usize) -> Vec<TokenStream> {
    let marshal_group = |group_idx: &usize, fields: &Vec<FieldInfo>| {
        let field_idents: Vec<_> = fields.iter().map(|field| &field.ident).collect();
        let field_chunks = fields
            .chunks(7)
            .enumerate()
            .map(|(chunk_idx, chunk_fields)| {
                let chunk_start = chunk_idx * 7;
                let mask_bits = chunk_fields.iter().enumerate().map(|(bit, _)| {
                    let field_idx = chunk_start + bit;
                    quote! {
                        if field_dirty[#field_idx] {
                            field_mask |= 1 << #bit;
                        }
                    }
                });
                let payloads = chunk_fields.iter().enumerate().map(|(bit, field)| {
                    let field_idx = chunk_start + bit;
                    let ident = &field.ident;
                    quote! {
                        if field_dirty[#field_idx] {
                            self.#ident.marshal(wb);
                        }
                    }
                });
                let later_indices = (chunk_start + chunk_fields.len())..fields.len();
                let is_first = chunk_idx == 0;
                quote! {
                    {
                        let mut field_mask = 0u8;
                        #(#mask_bits)*
                        let later_dirty = false #(|| field_dirty[#later_indices])*;
                        if later_dirty {
                            field_mask |= 0x80;
                        }
                        if #is_first || field_mask != 0 {
                            wb.write_u8(field_mask);
                            #(#payloads)*
                        }
                    }
                }
            });

        quote! {
            if descriptor_dirty[#group_idx] {
                let baseline = mc
                    .group_baselines
                    .and_then(|baselines| baselines.get(#group_idx))
                    .copied()
                    .unwrap_or(mc.baseline_seq);
                let field_dirty = [#(
                    ::gridmate::ReplicatedFieldHandlerBase::is_dirty(
                        &self.#field_idents,
                        baseline,
                    )
                ),*];
                #(#field_chunks)*
            }
        }
    };
    (0..=(max_descriptor / 8))
        .map(|chunk_idx| {
            let chunk_start = chunk_idx * 8;
            let chunk_end = chunk_start + 8;
            let descriptor_bits = groups
                .keys()
                .filter(|group_idx| **group_idx >= chunk_start && **group_idx < chunk_end)
                .map(|group_idx| {
                    let bit = 1u8 << (group_idx % 8);
                    quote! {
                        if descriptor_dirty[#group_idx] {
                            descriptor_mask |= #bit;
                        }
                    }
                });
            let groups_in_chunk = groups
                .iter()
                .filter(|(group_idx, _)| **group_idx >= chunk_start && **group_idx < chunk_end)
                .map(|(group_idx, fields)| marshal_group(group_idx, fields));
            quote! {
                let mut descriptor_mask = 0u8;
                #(#descriptor_bits)*
                wb.write_u8(descriptor_mask);
                #(#groups_in_chunk)*
            }
        })
        .collect()
}

/// The per-field handler statements shared by the dirty scan, the network-data
/// reset, and the merge pass.
struct Handlers {
    dirty: Vec<TokenStream>,
    reset: Vec<TokenStream>,
    merge: Vec<TokenStream>,
}

fn handlers(groups: &BTreeMap<usize, Vec<FieldInfo>>) -> Handlers {
    let dirty = groups.iter().map(|(group_idx, fields)| {
        let field_idents: Vec<_> = fields.iter().map(|field| &field.ident).collect();
        quote! {
            if mc
                .filter_target
                .map(|target| {
                    ::gridmate::hub::Fragment::should_send_to_client_group(self, target, #group_idx)
                })
                .unwrap_or(true)
            {
                let baseline = mc
                    .group_baselines
                    .and_then(|baselines| baselines.get(#group_idx))
                    .copied()
                    .unwrap_or(mc.baseline_seq);
                descriptor_dirty[#group_idx] = false #(
                    || ::gridmate::ReplicatedFieldHandlerBase::is_dirty(
                        &self.#field_idents,
                        baseline,
                    )
                )*;
            }
        }
    }).collect();

    let reset = groups
        .values()
        .flat_map(|fields| {
            fields.iter().map(|field| {
                let ident = &field.ident;
                quote! {
                    ::gridmate::ReplicatedFieldHandlerBase::reset_has_new_network_data(
                        &mut self.#ident,
                    );
                }
            })
        })
        .collect();

    let merge = groups
        .values()
        .flat_map(|fields| {
            fields.iter().map(|field| {
                let ident = &field.ident;
                quote! {
                    outcome.detected_new_data_in_last_merge |=
                        ::gridmate::ReplicatedFieldHandlerBase::merge_and_update_sequence(
                            &mut merged_state.#ident,
                            &self.#ident,
                            &mut new_state.#ident,
                            seq,
                            inherit_previous_network_data_status,
                        );
                    outcome.last_modified = outcome
                        .last_modified
                        .max(::gridmate::ReplicatedFieldHandlerBase::last_modified(
                            &merged_state.#ident,
                        ));
                    outcome.has_new_network_data |=
                        ::gridmate::ReplicatedFieldHandlerBase::has_new_network_data(
                            &merged_state.#ident,
                        );
                }
            })
        })
        .collect();

    Handlers {
        dirty,
        reset,
        merge,
    }
}

/// The per-field sequence-number statements for the field-metadata pass.
struct Metadata {
    marshal: Vec<TokenStream>,
    unmarshal: Vec<TokenStream>,
}

fn metadata(groups: &BTreeMap<usize, Vec<FieldInfo>>) -> Metadata {
    let marshal = groups
        .values()
        .flat_map(|fields| {
            fields.iter().map(|field| {
                let ident = &field.ident;
                quote! {
                    ::gridmate::ReplicatedFieldHandlerBase::last_modified(&self.#ident).marshal(wb);
                }
            })
        })
        .collect();

    let unmarshal = groups
        .values()
        .flat_map(|fields| {
            fields.iter().map(|field| {
                let ident = &field.ident;
                quote! {
                    let sequence = ::gridmate::hub::SequenceNumber::unmarshal(rb)?;
                    ::gridmate::ReplicatedFieldHandlerBase::set_last_modified(
                        &mut self.#ident,
                        sequence,
                    );
                }
            })
        })
        .collect();

    Metadata { marshal, unmarshal }
}

fn fragment_category_expr(
    category: Option<&str>,
    span: proc_macro2::Span,
) -> syn::Result<TokenStream> {
    let Some(name) = category else {
        return Ok(quote! { ::gridmate::hub::FragmentCategory::Uncategorized });
    };
    Ok(match name {
        "uncategorized" => quote! { ::gridmate::hub::FragmentCategory::Uncategorized },
        "player_character" => quote! { ::gridmate::hub::FragmentCategory::PlayerCharacter },
        "non_player_character" | "npc" => {
            quote! { ::gridmate::hub::FragmentCategory::NonPlayerCharacter }
        }
        "important_non_player_character" | "important_npc" => {
            quote! { ::gridmate::hub::FragmentCategory::ImportantNonPlayerCharacter }
        }
        "spell" => quote! { ::gridmate::hub::FragmentCategory::Spell },
        "projectile" => quote! { ::gridmate::hub::FragmentCategory::Projectile },
        "buildable" => quote! { ::gridmate::hub::FragmentCategory::Buildable },
        other => {
            return Err(syn::Error::new(
                span,
                format!("unsupported replicated_state category `{other}`"),
            ));
        }
    })
}

fn expand_fragment(
    ident: &Ident,
    base: &BaseField,
    groups: &BTreeMap<usize, Vec<FieldInfo>>,
    attributes: &[FieldInfo],
    opts: &Opts,
    generics: &syn::Generics,
) -> syn::Result<TokenStream> {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let base_ident = &base.ident;
    let n_groups = groups
        .keys()
        .chain(opts.skip_groups.iter())
        .copied()
        .max()
        .map_or(1, |max| max + 1);

    let category = category(ident, opts, attributes)?;
    let is_metadata = opts.metadata.then(|| {
        quote! {
            fn is_metadata(&self) -> bool {
                true
            }
        }
    });
    let position = position(ident, opts);

    let contents = contents(base_ident);
    let attrs = attrs(base_ident, n_groups, groups, attributes);
    let merge = merge(base_ident, n_groups, attributes);
    let tail = tail(
        base_ident,
        n_groups,
        attributes,
        &category,
        is_metadata.as_ref(),
        &position,
    );

    Ok(quote! {
        impl #impl_generics ::gridmate::hub::Fragment
        for #ident #ty_generics #where_clause
        {
            #contents
            #attrs
            #merge
            #tail
        }
    })
}

/// The `category()` body: either a fixed category or a read of the registered
/// attribute field named by `#[replicated_state(category_field = "…")]`.
fn category(ident: &Ident, opts: &Opts, attributes: &[FieldInfo]) -> syn::Result<TokenStream> {
    let Some(field_name) = &opts.category_field else {
        return fragment_category_expr(opts.category.as_deref(), ident.span());
    };

    let category_ident = syn::parse_str::<Ident>(field_name).map_err(|_| {
        syn::Error::new_spanned(
            ident,
            format!("replicated_state category_field `{field_name}` is not an identifier"),
        )
    })?;
    if !attributes
        .iter()
        .any(|attribute| attribute.ident == category_ident)
    {
        return Err(syn::Error::new_spanned(
            ident,
            format!("replicated_state category_field `{field_name}` must name an attribute field"),
        ));
    }

    Ok(quote! {
        self.#category_ident
            .value()
            .copied()
            .map(::core::convert::Into::into)
            .unwrap_or_default()
    })
}

/// The optional world-position forwarding methods.
fn position(ident: &Ident, opts: &Opts) -> TokenStream {
    match &opts.world_position {
        Some(method_name) if method_name.ends_with("()") => {
            let method_name = method_name.trim_end_matches("()");
            let method = Ident::new(method_name, ident.span());
            quote! {
                fn has_world_position(&self) -> bool {
                    true
                }

                fn world_position(&self) -> ::core::option::Option<::glam::Vec3> {
                    self.#method()
                }
            }
        }
        Some(field_name) => {
            let pos = Ident::new(field_name, ident.span());
            quote! {
                fn has_world_position(&self) -> bool {
                    true
                }

                fn world_position(&self) -> ::core::option::Option<::glam::Vec3> {
                    self.#pos.value.as_ref().map(|anchor| {
                        ::glam::Vec3::new(anchor.x, anchor.y, anchor.height)
                    })
                }
            }
        }
        None => quote! {},
    }
}

/// Shared-reference `ReplicatedFieldInfo` rows for a field list.
fn infos(fields: &[FieldInfo]) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|field| {
            let field_ident = &field.ident;
            let source_name = &field.source_name;
            quote! {
                ::gridmate::hub::ReplicatedFieldInfo {
                    name: #source_name,
                    handler: &self.#field_ident,
                    is_filter_group: false,
                }
            }
        })
        .collect()
}

/// Mutable `ReplicatedFieldInfoMut` rows for a field list, reached through
/// `owner` (`self`, `new_state`, `merged_state`).
fn infos_mut(fields: &[FieldInfo], owner: &TokenStream) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|field| {
            let field_ident = &field.ident;
            let source_name = &field.source_name;
            quote! {
                ::gridmate::hub::ReplicatedFieldInfoMut {
                    name: #source_name,
                    handler: &mut #owner.#field_ident,
                    is_filter_group: false,
                }
            }
        })
        .collect()
}

/// The per-group default-bit blocks: computed against the marshal baseline,
/// computed for the metadata pass, and applied after an unmarshal.
struct Bits {
    calculate: Vec<TokenStream>,
    metadata: Vec<TokenStream>,
    apply: Vec<TokenStream>,
}

fn bits(base: &Ident, groups: &BTreeMap<usize, Vec<FieldInfo>>) -> Bits {
    let calculate = groups
        .iter()
        .map(|(group_idx, fields)| {
            let field_infos = infos(fields);
            quote! {
                {
                    let fields = [#(#field_infos),*];
                    hub.calculate_default_bits(#group_idx, &fields, mc.baseline_seq);
                }
            }
        })
        .collect();

    let metadata = groups
        .iter()
        .map(|(group_idx, fields)| {
            let field_infos = infos(fields);
            quote! {
                {
                    let fields = [#(#field_infos),*];
                    hub.calculate_default_bits(#group_idx, &fields, ::gridmate::hub::SequenceNumber::Invalid);
                }
            }
        })
        .collect();

    let owner = quote!(self);
    let apply = groups
        .iter()
        .map(|(group_idx, fields)| {
            let field_infos = infos_mut(fields, &owner);
            quote! {
                {
                    let mut fields = [#(#field_infos),*];
                    let hub = &self.#base;
                    hub.apply_default_bits(#group_idx, &mut fields);
                }
            }
        })
        .collect();

    Bits {
        calculate,
        metadata,
        apply,
    }
}

/// `Fragment` methods that delegate straight to the embedded base.
fn contents(base: &Ident) -> TokenStream {
    quote! {
        fn base(&self) -> &::gridmate::hub::FragmentBase {
            self.#base.base()
        }

        fn base_mut(&mut self) -> &mut ::gridmate::hub::FragmentBase {
            self.#base.base_mut()
        }

        fn marshal_contents(
            &self,
            wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
        ) -> bool {
            self.marshal_contents_with(&::gridmate::hub::MarshalContext::default(), wb)
        }

        fn marshal_contents_with(
            &self,
            mc: &::gridmate::hub::MarshalContext<'_>,
            wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
        ) -> bool {
            self.__replicated_state_marshal_fields(mc, wb)
        }

        fn unmarshal_contents(
            &mut self,
            rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
        ) -> ::core::result::Result<bool, ::gridmate::serialize::error::MarshalerError> {
            self.__replicated_state_unmarshal_fields(rb)?;
            Ok(true)
        }
    }
}

/// The registered-attribute and field-metadata `Fragment` methods.
fn attrs(
    base: &Ident,
    n_groups: usize,
    groups: &BTreeMap<usize, Vec<FieldInfo>>,
    attributes: &[FieldInfo],
) -> TokenStream {
    let bits = bits(base, groups);
    let calculate = &bits.calculate;
    let metadata_bits = &bits.metadata;
    let apply = &bits.apply;

    let owner = quote!(self);
    let marshal_infos = infos(attributes);
    let metadata_infos = infos(attributes);
    let unmarshal_infos = infos_mut(attributes, &owner);
    let unmarshal_metadata_infos = infos_mut(attributes, &owner);

    quote! {
        fn marshal_attributes(
            &self,
            mc: &::gridmate::hub::MarshalContext<'_>,
            wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
        ) -> bool {
            let mut hub = self.#base.clone();
            hub.ensure_filter_groups(#n_groups);
            #(#calculate)*
            let attributes = [#(#marshal_infos),*];
            hub.marshal_registered_attributes(mc.baseline_seq, &attributes, wb)
        }

        fn unmarshal_attributes(
            &mut self,
            rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
        ) -> ::core::result::Result<bool, ::gridmate::serialize::error::MarshalerError> {
            self.#base.ensure_filter_groups(#n_groups);
            let read_any = {
                let mut attributes = [#(#unmarshal_infos),*];
                self.#base.unmarshal_registered_attributes(
                    &mut attributes,
                    rb,
                )?
            };
            #(#apply)*
            Ok(read_any)
        }

        fn marshal_field_metadata(
            &self,
            _mc: &::gridmate::hub::MarshalContext<'_>,
            wb: &mut ::gridmate::serialize::buffer::WriteBuffer,
        ) -> bool {
            let mut hub = self.#base.clone();
            hub.ensure_filter_groups(#n_groups);
            #(#metadata_bits)*
            self.__replicated_state_marshal_field_metadata(wb);
            let attributes = [#(#metadata_infos),*];
            hub.marshal_registered_attribute_metadata(&attributes, wb);
            true
        }

        fn unmarshal_field_metadata(
            &mut self,
            rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
        ) -> ::core::result::Result<bool, ::gridmate::serialize::error::MarshalerError> {
            let read_fields = self.__replicated_state_unmarshal_field_metadata(rb)?;
            self.#base.ensure_filter_groups(#n_groups);
            let mut attributes = [#(#unmarshal_metadata_infos),*];
            let read_attrs = self.#base.unmarshal_registered_attribute_metadata(
                &mut attributes,
                rb,
            )?;
            Ok(read_fields || read_attrs)
        }
    }
}

/// The `merge_and_update_sequence` method.
fn merge(base: &Ident, n_groups: usize, attributes: &[FieldInfo]) -> TokenStream {
    let old_infos = infos(attributes);
    let new_infos = infos_mut(attributes, &quote!(new_state));
    let merged_infos = infos_mut(attributes, &quote!(merged_state));

    quote! {
        fn merge_and_update_sequence(
            &self,
            new_fragment: &mut dyn ::gridmate::hub::Fragment,
            seq: ::gridmate::hub::SequenceNumber,
            inherit_previous_network_data_status: bool,
        ) -> ::core::option::Option<::std::boxed::Box<dyn ::gridmate::hub::Fragment>> {
            debug_assert!(seq.is_valid(), "Merge-to sequence should never be invalid");
            let new_correlation_id = new_fragment.correlation_id();
            let new_state = <dyn ::std::any::Any>::downcast_mut::<Self>(new_fragment)?;
            let mut merged_state = Self::default();
            let mut outcome = ::gridmate::hub::ReplicatedMergeOutcome::default();

            merged_state.#base.ensure_filter_groups(#n_groups);
            self.__replicated_state_merge_fields(
                new_state,
                &mut merged_state,
                seq,
                inherit_previous_network_data_status,
                &mut outcome,
            );
            merged_state.#base.merge_filter_group_attributes(
                &self.#base,
                &mut new_state.#base,
                seq,
                inherit_previous_network_data_status,
                &mut outcome,
            );
            {
                let old_attributes = [#(#old_infos),*];
                let mut new_attributes = [#(#new_infos),*];
                let mut merged_attributes = [#(#merged_infos),*];
                ::gridmate::hub::ReplicatedState::merge_registered_attributes(
                    &old_attributes,
                    &mut new_attributes,
                    &mut merged_attributes,
                    seq,
                    inherit_previous_network_data_status,
                    &mut outcome,
                );
            }
            merged_state
                .#base
                .finish_merge(seq, new_correlation_id, outcome);
            ::core::option::Option::Some(::std::boxed::Box::new(merged_state))
        }
    }
}

/// The network-data bookkeeping, category, and filter-group `Fragment` methods.
fn tail(
    base: &Ident,
    n_groups: usize,
    attributes: &[FieldInfo],
    category: &TokenStream,
    metadata: Option<&TokenStream>,
    position: &TokenStream,
) -> TokenStream {
    let reset_infos = infos_mut(attributes, &quote!(self));

    quote! {
        fn reset_has_new_network_data(&mut self) {
            self.__replicated_state_reset_has_new_network_data();
            let mut attributes = [#(#reset_infos),*];
            ::gridmate::hub::ReplicatedState::reset_registered_attribute_network_data(
                &mut attributes,
            );
            self.#base.reset_filter_group_attribute_network_data();
            self.#base.reset_has_new_network_data();
        }

        fn set_has_new_network_data_on_initial_state(&mut self) {
            self.#base.set_has_new_network_data_on_initial_state();
        }

        fn is_fully_merged_state(&self) -> bool {
            self.#base.is_fully_merged_state()
        }

        fn has_new_network_data(&self) -> bool {
            self.#base.has_new_network_data()
        }

        fn detected_new_data_in_last_merge(&self) -> bool {
            self.#base.detected_new_data_in_last_merge()
        }

        fn update_sequence(&self) -> ::gridmate::hub::SequenceNumber {
            self.#base.sequence()
        }

        fn is_fragment_dirty(&self, baseline: ::gridmate::hub::SequenceNumber) -> bool {
            baseline < self.#base.last_modified()
        }

        fn category(&self) -> ::gridmate::hub::FragmentCategory {
            #category
        }

        #metadata

        #position

        fn num_filter_groups(&self) -> usize {
            #n_groups
        }

        fn should_send_to_client_group(
            &self,
            target: u64,
            group_idx: usize,
        ) -> bool {
            group_idx < #n_groups
                && self.#base.should_send_to_client(target, group_idx)
        }

        fn create_new_instance(&self) -> ::core::option::Option<::std::boxed::Box<dyn ::gridmate::hub::Fragment>> {
            ::core::option::Option::Some(::std::boxed::Box::new(Self::default()))
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::derive;

    #[test]
    fn generated_borrow_scopes_do_not_use_drop_to_end_field_borrows() {
        let input = parse_quote! {
            struct TestState {
                base: gridmate::hub::ReplicatedState,
                #[replicated_state(group = 0)]
                value: gridmate::ReplicatedField<u32>,
                #[replicated_state(attribute)]
                category: gridmate::ReplicatedField<u8>,
            }
        };

        let output = derive(&input).expect("derive should succeed").to_string();

        assert!(!output.contains("drop (attributes)"));
        assert!(!output.contains("drop ((new_attributes, merged_attributes))"));
    }
}
