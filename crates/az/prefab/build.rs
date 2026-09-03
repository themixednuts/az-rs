//! Publish a derived fingerprint of this crate's sources.
//!
//! Asset build rules that frame their product bytes with this crate's code
//! compose the fingerprint into their analysis fingerprint, so changing a
//! codec or an analysis pass invalidates products this crate produced
//! earlier. See `az-build-fingerprint` for why a hand-maintained counter
//! cannot do this job.

fn main() {
    az_build_fingerprint::emit();
}
