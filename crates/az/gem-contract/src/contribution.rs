use crate::capability::CapsDecl;
use crate::context::GemContext;
use crate::descriptor::ContributionDescriptor;

/// One contribution crate's entry contract (ADR 0032, conditioned by 0041).
///
/// The descriptor is pure data; all behavioral contribution happens in
/// `register` through the typed context. Lifecycle phases are host-driven;
/// the composer owns its contributions so they are invocable and reload can
/// re-drive them.
pub trait Contribution: Send + 'static {
    type Caps: CapsDecl;

    fn descriptor(&self) -> ContributionDescriptor;

    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>);

    fn ready(&self) -> bool {
        true
    }

    fn finish(&self) {}

    fn cleanup(&self) {}
}

/// Object-safe lifecycle view the composer stores. Identity travels as the
/// composed `InstanceId`, so the descriptor is not re-erased here.
///
/// Registration itself stays typed: it runs in `Composer::add`, where the
/// concrete type is known.
pub trait ErasedContribution: Send {
    fn ready(&self) -> bool;
    fn finish(&self);
    fn cleanup(&self);
}

impl<C: Contribution> ErasedContribution for C {
    fn ready(&self) -> bool {
        Contribution::ready(self)
    }

    fn finish(&self) {
        Contribution::finish(self);
    }

    fn cleanup(&self) {
        Contribution::cleanup(self);
    }
}
