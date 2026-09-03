//! `#[derive(Message)]` — emits `impl gridmate::message::Message for T`.
//!
//! **Single trait, single derive.** Composes with `#[derive(ClassDesc)]`
//! (which supplies `GridMate` registration), `#[derive(AzTypeInfo)]`
//! (which supplies AZ UUID/name), and `#[derive(Marshaler)]` (the wire codec).
//!
//! ```ignore
//! #[derive(AzTypeInfo, Marshaler, ClassDesc, Message)]
//! #[az_type_info("6A379FB8-0BDD-43A1-AB3E-9843D7BE8CD3")]
//! #[class_desc(type_index = 349)]
//! pub struct PingMsg { pub timestamp: u64 }
//! ```
//!
//! Optional `#[message(type_index = N)]` overrides the `TYPE_INDEX` const
//! provided by the class descriptor for hand-audited exceptions.
//!
//! Direction flags (`#[message(client_to_server)]` and
//! `#[message(server_to_client)]`) emit the matching `Sendable` /
//! `Receivable` impls. These map source `MF_TraitClientMsg` /
//! `MF_TraitMsg` declarations directly instead of routing through a
//! facet-specific registry.
//!
//! `#[message(actor_scoped)]` records that the message body starts with
//! the native actor/facet routing header. This is protocol metadata
//! owned by the message registration, not by the structural accessor trait
//! that typed Rust code may use to read or write that field.
//!
//! # What this derive does *not* emit
//!
//! - the `Class` impl carrying `TYPE_INDEX` — use `#[derive(ClassDesc)]`.
//! - actor routing or Bevy receive systems; those are explicit runtime/plugin
//!   registrations over concrete message types.
//! - registration. A message type reaches a host's wire decoder as
//!   `ClassRegistration::of_message::<T>()` — which reads `INFO` and
//!   `TYPE_INDEX` straight off this impl, so a registration site names the type
//!   and repeats nothing — and, in debug builds, as
//!   `DebugIntrospect::of::<T>()` for the pretty-printer capture tooling uses.
//!   Both are published by the crate that declares the type, not by the
//!   linker. Release builds simply do not register the introspection entries;
//!   the whole family is `cfg(debug_assertions)` (zero size, zero format
//!   strings).
//!
//! A type deriving both `ClassDesc` and `Message` still registers **once**:
//! `of_message` is the constructor for a type that has both, and the `Message`
//! bound is what makes that a compile-time check rather than a convention.

// darling's `FromDeriveInput` expansion lands here as a sibling impl carrying
// this file's spans, so its generated parse loop and `#[darling(default)]`
// fallbacks can only be exempted at module scope.
#![allow(clippy::needless_continue, clippy::option_if_let_else)]

use darling::FromDeriveInput;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident};

use crate::error::darling_to_syn;

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(message), supports(struct_any, enum_any))]
struct MessageOpts {
    ident: Ident,
    generics: syn::Generics,

    /// Optional `TYPE_INDEX` override — `#[message(type_index = N)]`.
    ///
    /// Default behaviour: the `Message` trait's `TYPE_INDEX` const uses the
    /// value emitted by `#[derive(ClassDesc)]`. Override only for explicit
    /// hand-audited exceptions.
    #[darling(default)]
    type_index: Option<u32>,

    /// Source `MF_TraitClientMsg`: client sends, server receives.
    #[darling(default)]
    client_to_server: bool,

    /// Source `MF_TraitMsg`: server sends, client receives.
    #[darling(default)]
    server_to_client: bool,

    /// Message body starts with native `ActorRequestId`.
    #[darling(default)]
    actor_scoped: bool,
}

pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let opts = MessageOpts::from_derive_input(input).map_err(darling_to_syn)?;

    let ident = &opts.ident;
    let (impl_generics, ty_generics, where_clause) = opts.generics.split_for_impl();

    let type_index_override = opts.type_index.map_or_else(
        || {
            quote! {
                const TYPE_INDEX: u32 = {
                    const RESOLVED: u32 =
                        <#ident #ty_generics as ::gridmate::az::Class>::TYPE_INDEX;
                    if RESOLVED == 0 {
                        panic!(
                            "#[derive(Message)] requires a nonzero #[class_desc(type_index = N)] \
                             or an explicit #[message(type_index = N)]"
                        );
                    }
                    RESOLVED
                };
            }
        },
        |n| {
            quote! {
                const TYPE_INDEX: u32 = {
                    const SUPPLIED: u32 = #n;
                    const RESOLVED: u32 =
                        <#ident #ty_generics as ::gridmate::az::Class>::TYPE_INDEX;
                    if RESOLVED != 0 && RESOLVED != SUPPLIED {
                        panic!(
                            "#[derive(Message)]: #[message(type_index = N)] override disagrees \
                             with #[derive(ClassDesc)] — drop the override or fix the class descriptor"
                        );
                    }
                    SUPPLIED
                };
            }
        },
    );

    let client_to_server_impls = opts.client_to_server.then(|| {
        quote! {
            impl #impl_generics ::gridmate::message::Sendable<::gridmate::message::ClientToServer>
            for #ident #ty_generics #where_clause
            {}

            impl #impl_generics ::gridmate::message::Receivable<::gridmate::message::ClientToServer>
            for #ident #ty_generics #where_clause
            {}
        }
    });

    let server_to_client_impls = opts.server_to_client.then(|| {
        quote! {
            impl #impl_generics ::gridmate::message::Sendable<::gridmate::message::ServerToClient>
            for #ident #ty_generics #where_clause
            {}

            impl #impl_generics ::gridmate::message::Receivable<::gridmate::message::ServerToClient>
            for #ident #ty_generics #where_clause
            {}
        }
    });
    let actor_scoped = opts.actor_scoped;

    Ok(quote! {
        impl #impl_generics ::gridmate::message::Message
        for #ident #ty_generics #where_clause
        {
            #type_index_override

            const INFO: ::gridmate::az::MessageInfo = ::gridmate::az::MessageInfo {
                actor_scoped: #actor_scoped,
            };
        }

        #client_to_server_impls

        #server_to_client_impls
    })
}
