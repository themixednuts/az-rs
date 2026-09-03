//! A typo'd id literal is a compile error: the dotted-id grammar is validated
//! in const context, so identity mistakes never reach a running process
//! (ADR 0032).

use az_gem_contract::GemId;

const GEM: GemId = GemId::new("Azoth.Vegetation");

fn main() {
    let _ = GEM;
}
