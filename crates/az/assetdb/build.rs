//! Build-time invalidation for the checked-in asset database migration chain.
//!
//! Ordinary compilation must be read-only with respect to the source tree.
//! Migration generation is an explicit maintenance operation: running it from
//! a build script created timestamped directories on every developer and CI
//! machine whenever an intermediate schema revision was compiled.

fn main() {
    println!("cargo:rerun-if-changed=src/schema.rs");
    println!("cargo:rerun-if-changed=drizzle");
}
