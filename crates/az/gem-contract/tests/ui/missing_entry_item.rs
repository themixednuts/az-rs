//! The generated named call is the inventory: glue that references a renamed
//! or deleted entry item fails name resolution instead of silently
//! unregistering the gem (ADR 0032).

use az_gem_contract::{Composer, ProductActivation};

/// Stands in for the contribution crate whose entry item was renamed.
mod gem_vegetation {}

fn generated_glue(composer: &mut Composer) {
    let _ = composer.add(
        gem_vegetation::runtime_contribution(),
        ProductActivation::default(),
    );
}

fn main() {
    let _ = generated_glue;
}
