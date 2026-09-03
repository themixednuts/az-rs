//! Raw FFI bindings to the Epic Online Services SDK.
//!
//! The binding source is produced by `build.rs` (bindgen against the bundled
//! SDK headers, or a hand-rolled fallback when those headers are absent) and
//! written to `OUT_DIR`. This crate is only the wrapper that includes it.

// Machine-generated build-script output: `eos_bindings.rs` is written into
// OUT_DIR on every build and is not checked in, so there is no source line a
// per-site fix could land on. Wrapping the include in a private module keeps
// the suppression scoped to the generated code exactly the way
// `az_proto_core::generated_schema!` does for capnpc output; the C ABI naming
// and `improper_ctypes` allows are inherent to the SDK's own symbol names.
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    improper_ctypes,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/eos_bindings.rs"));
}

pub use bindings::*;
