//! `#[derive(ChunkMarshaler)]` — emits the standard `Marshaler` delegate for
//! a type that already implements [`IFragment`](::gridmate::hub::Fragment).
//!
//! Hand-written replicated states use source-shaped `Fragment::MarshalContents`
//! methods when their bitmask layout does not match `#[derive(ReplicatedState)]`.
//! One concrete impl per type avoids Rust coherence overlap with the generic
//! `Marshaler` impls for wrapper types.
//!
//! Pair with a hand-written `Fragment` implementation:
//!
//! ```ignore
//! #[derive(Default, ChunkMarshaler)]
//! pub struct FooState { /* … */ }
//!
//! impl gridmate::hub::Fragment for FooState { /* hand-rolled marshal logic */ }
//! ```
//!
//! For types whose fragment implementation is itself derive-generated, use
//! `#[derive(ReplicatedState)]` instead.

use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

// The `syn::Result` is the shared derive-helper contract `crate::run` takes;
// this derive just happens to have nothing to reject.
#[allow(clippy::unnecessary_wraps)]
pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::gridmate::serialize::marshaler::Marshaler
        for #ident #ty_generics #where_clause
        {
            fn marshal(&self, wb: &mut ::gridmate::serialize::buffer::WriteBuffer) {
                ::gridmate::hub::Fragment::marshal_contents(self, wb);
            }

            fn unmarshal(
                rb: &mut ::gridmate::serialize::buffer::ReadBuffer,
            ) -> ::core::result::Result<Self, ::gridmate::serialize::error::MarshalerError> {
                let mut value = <Self as ::core::default::Default>::default();
                ::gridmate::hub::Fragment::unmarshal_contents(&mut value, rb)?;
                ::core::result::Result::Ok(value)
            }
        }
    })
}
