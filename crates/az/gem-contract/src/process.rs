use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::{ComposeError, ComposeReport, Composer, GemTargetRole, HostDecl, Registries};

/// An owned read capability for one finalized process's registries.
///
/// This lease deliberately exposes no mutable composer or reference-counting
/// mechanism. A service may clone it into owned worker tasks, but must join
/// those tasks before dropping the [`ProcessComposition`] that owns its
/// contribution lifecycle.
pub struct RegistryLease {
    registries: Arc<Registries>,
    token: Arc<LeaseToken>,
}

impl Clone for RegistryLease {
    fn clone(&self) -> Self {
        Self {
            registries: Arc::clone(&self.registries),
            token: Arc::clone(&self.token),
        }
    }
}

/// Refusal returned when a composition is already quiescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RegistryLeaseError {
    #[error("composition has begun shutdown and cannot issue registry leases")]
    ShuttingDown,
    #[error("composition lease accounting lock is poisoned")]
    Poisoned,
}

/// Refusal returned when a composition cannot yet run contribution cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProcessCompositionCleanupError {
    #[error("composition still has {active} active registry lease scope(s)")]
    ActiveLeases { active: usize },
    #[error("composition lease accounting lock is poisoned")]
    Poisoned,
}

/// Process-owned issuer for immutable registry leases.
///
/// Clones of an already-issued [`RegistryLease`] share one token; they do not
/// create a new lease scope after shutdown has begun. This lets worker tasks
/// share a read capability while preserving a single explicit quiescence
/// boundary before contribution cleanup.
#[derive(Clone)]
struct LeaseScope {
    state: Arc<Mutex<LeaseScopeState>>,
}

struct LeaseScopeState {
    accepting: bool,
    active: usize,
}

struct LeaseToken {
    scope: LeaseScope,
}

impl Drop for LeaseToken {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scope.state.lock() {
            debug_assert!(state.active > 0, "lease token must be counted before drop");
            state.active -= 1;
        }
    }
}

impl LeaseScope {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(LeaseScopeState {
                accepting: true,
                active: 0,
            })),
        }
    }

    fn issue(&self, registries: Arc<Registries>) -> Result<RegistryLease, RegistryLeaseError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RegistryLeaseError::Poisoned)?;
        if !state.accepting {
            return Err(RegistryLeaseError::ShuttingDown);
        }
        state.active += 1;
        drop(state);
        Ok(RegistryLease {
            registries,
            token: Arc::new(LeaseToken {
                scope: self.clone(),
            }),
        })
    }

    fn begin_shutdown(&self) -> Result<usize, ProcessCompositionCleanupError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessCompositionCleanupError::Poisoned)?;
        state.accepting = false;
        Ok(state.active)
    }
}

impl RegistryLease {
    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.registries.as_ref()
    }
}

impl AsRef<Registries> for RegistryLease {
    fn as_ref(&self) -> &Registries {
        self.registries()
    }
}

/// One finalized composition owned for a process lifetime.
///
/// Construction consumes the mutable [`Composer`] and validates it exactly
/// once. From then on callers can only read the resulting registries/report or
/// drive the horizontal contribution lifecycle. Shutdown first closes lease
/// issuance, then its owner joins consumers, and only quiescent cleanup
/// finishes and cleans contributions.
pub struct ProcessComposition {
    composer: Composer,
    report: ComposeReport,
    leases: LeaseScope,
    shutting_down: Arc<AtomicBool>,
    lifecycle: ProcessCompositionLifecycle,
}

/// Narrow cross-thread shutdown capability for a composition owner.
///
/// It can only close lease issuance. It cannot finish, clean, detach, or
/// expose registries; the owning [`ProcessComposition`] performs those steps
/// after it has joined every consumer.
#[derive(Clone)]
pub struct CompositionShutdownGate {
    leases: LeaseScope,
    shutting_down: Arc<AtomicBool>,
}

impl CompositionShutdownGate {
    /// Close registry-lease issuance from any thread holding this gate.
    ///
    /// Closing admission is idempotent and monotonic: already-issued leases
    /// stay valid so in-flight consumers can finish, but no new lease scope
    /// is issued afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessCompositionCleanupError::Poisoned`] when the lease
    /// accounting lock is poisoned, which leaves admission state unknown.
    pub fn begin_shutdown(&self) -> Result<(), ProcessCompositionCleanupError> {
        self.leases.begin_shutdown()?;
        self.shutting_down.store(true, Ordering::Release);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessCompositionLifecycle {
    Running,
    Quiescing,
    Finished,
    Cleaned,
}

impl ProcessComposition {
    /// Validate and seal one mutable composition for process ownership.
    ///
    /// # Errors
    ///
    /// Returns the composer's exact validation error. The composer is consumed
    /// even on failure, so an invalid composition cannot be repaired after an
    /// attempted process admission.
    pub fn new(composer: Composer) -> Result<Self, ComposeError> {
        let report = composer.finalize()?;
        Ok(Self {
            composer,
            report,
            leases: LeaseScope::new(),
            shutting_down: Arc::new(AtomicBool::new(false)),
            lifecycle: ProcessCompositionLifecycle::Running,
        })
    }

    #[must_use]
    pub const fn host(&self) -> HostDecl {
        self.composer.host()
    }

    #[must_use]
    pub const fn role(&self) -> GemTargetRole {
        self.report.role
    }

    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.composer.registries()
    }

    /// An owned read lease over this finalized process's registries.
    ///
    /// A process may hand this lease to worker threads whose work must outlive
    /// the caller's stack frame. The process owner remains responsible for
    /// joining that work before it drops this composition and finishes its
    /// contributions; the lease never exposes the mutable composer.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryLeaseError::ShuttingDown`] once shutdown has closed
    /// lease admission, or [`RegistryLeaseError::Poisoned`] when the lease
    /// accounting lock is poisoned.
    pub fn registry_lease(&self) -> Result<RegistryLease, RegistryLeaseError> {
        self.leases.issue(self.composer.registry_lease())
    }

    /// A cross-thread capability to close new registry-lease admission before
    /// cancelling work that already holds an issued lease.
    #[must_use]
    pub fn shutdown_gate(&self) -> CompositionShutdownGate {
        CompositionShutdownGate {
            leases: self.leases.clone(),
            shutting_down: Arc::clone(&self.shutting_down),
        }
    }

    #[must_use]
    pub const fn report(&self) -> &ComposeReport {
        &self.report
    }

    /// True when every contribution reports ready and shutdown has not begun.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.lifecycle == ProcessCompositionLifecycle::Running
            && !self.shutting_down.load(Ordering::Acquire)
            && self.composer.all_ready()
    }

    /// Hand the composed app to a world-hosting process.
    #[cfg(feature = "app")]
    #[must_use]
    pub const fn take_app(&mut self) -> Option<crate::App> {
        self.composer.take_app()
    }

    /// Finish every contribution once after every issued lease has left.
    fn finish(&mut self) {
        if self.lifecycle == ProcessCompositionLifecycle::Quiescing {
            self.composer.finish();
            self.lifecycle = ProcessCompositionLifecycle::Finished;
        }
    }

    /// Close registry-lease issuance before the owner joins its consumers.
    ///
    /// Service owners call this before joining their RPC/worker consumers. It
    /// makes admission monotonic: an in-flight consumer can finish through its
    /// existing lease, but no new independent registry scope can be issued.
    /// It deliberately does not finish contributions; that belongs after the
    /// owner has joined every consumer of those leases.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessCompositionCleanupError::Poisoned`] when the lease
    /// accounting lock is poisoned. The lifecycle stays `Running` in that
    /// case, because admission could not be proven closed.
    pub fn begin_shutdown(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.shutdown_gate().begin_shutdown()?;
        if self.lifecycle == ProcessCompositionLifecycle::Running {
            self.lifecycle = ProcessCompositionLifecycle::Quiescing;
        }
        Ok(())
    }

    /// Finish, then clean up, every contribution once.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessCompositionCleanupError::ActiveLeases`] when any
    /// issued registry lease is still outstanding, naming how many; the owner
    /// must join those consumers and call this again. Returns
    /// [`ProcessCompositionCleanupError::Poisoned`] when the lease accounting
    /// lock is poisoned. Contributions are neither finished nor cleaned on
    /// either refusal.
    pub fn cleanup(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.begin_shutdown()?;
        let active = self.leases.begin_shutdown()?;
        if active != 0 {
            return Err(ProcessCompositionCleanupError::ActiveLeases { active });
        }
        self.finish();
        if self.lifecycle == ProcessCompositionLifecycle::Finished {
            self.composer.cleanup();
            self.lifecycle = ProcessCompositionLifecycle::Cleaned;
        }
        Ok(())
    }
}

impl Drop for ProcessComposition {
    fn drop(&mut self) {
        // Drop must not block or panic while a process is unwinding. Explicit
        // owners call `cleanup` after joining every registry consumer; if they
        // do not, both finish and contribution cleanup are refused.
        let _ = self.begin_shutdown();
        if self.leases.begin_shutdown().ok() == Some(0) {
            self.finish();
            if self.lifecycle == ProcessCompositionLifecycle::Finished {
                self.composer.cleanup();
                self.lifecycle = ProcessCompositionLifecycle::Cleaned;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::{
        ClockDefinition, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        ProductActivation,
    };

    use super::*;

    crate::declare_caps!(LifecycleCaps:);

    struct LifecycleContribution {
        ready: Arc<AtomicBool>,
        finished: Arc<AtomicUsize>,
        cleaned: Arc<AtomicUsize>,
    }

    impl Contribution for LifecycleContribution {
        type Caps = LifecycleCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.lifecycle-test"),
                contribution: ContributionId::new("runtime-host"),
                roles: &[GemTargetRole::RuntimeHost],
            }
        }

        fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}

        fn ready(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }

        fn finish(&self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn cleanup(&self) {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn process_composition_finalizes_once_and_owns_lifecycle() {
        let ready = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(GemTargetRole::RuntimeHost);
        composer
            .add(
                LifecycleContribution {
                    ready: Arc::clone(&ready),
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();

        let process = ProcessComposition::new(composer).unwrap();
        assert_eq!(process.role(), GemTargetRole::RuntimeHost);
        assert_eq!(process.report().composed.len(), 1);
        assert!(!process.is_ready());
        ready.store(true, Ordering::SeqCst);
        assert!(process.is_ready());

        let shutdown = process.shutdown_gate();
        shutdown.begin_shutdown().unwrap();
        shutdown.begin_shutdown().unwrap();
        assert!(!process.is_ready());
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);

        drop(process);
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_a_running_process_finishes_before_cleanup() {
        let ready = Arc::new(AtomicBool::new(true));
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(GemTargetRole::RuntimeHost);
        composer
            .add(
                LifecycleContribution {
                    ready,
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();

        drop(ProcessComposition::new(composer).unwrap());

        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn registry_lease_shares_only_the_finalized_registry_set() {
        let process = ProcessComposition::new(Composer::new(GemTargetRole::RuntimeHost)).unwrap();

        let first = process.registry_lease().unwrap();
        let second = process.registry_lease().unwrap();

        assert!(std::ptr::eq(first.registries(), second.registries()));
        assert!(first.registries().get::<ClockDefinition>().is_none());
    }

    #[test]
    fn begin_shutdown_rejects_new_leases_then_cleanup_quiesces_after_join() {
        let mut process =
            ProcessComposition::new(Composer::new(GemTargetRole::RuntimeHost)).unwrap();
        let lease = process.registry_lease().unwrap();
        let clone = lease.clone();

        let shutdown = process.shutdown_gate();
        shutdown.begin_shutdown().unwrap();
        assert!(matches!(
            process.cleanup(),
            Err(ProcessCompositionCleanupError::ActiveLeases { active: 1 })
        ));
        assert!(matches!(
            process.registry_lease(),
            Err(RegistryLeaseError::ShuttingDown)
        ));

        drop(lease);
        assert!(matches!(
            process.cleanup(),
            Err(ProcessCompositionCleanupError::ActiveLeases { active: 1 })
        ));
        drop(clone);
        process.cleanup().unwrap();
    }

    #[test]
    fn finish_waits_for_the_last_registry_lease() {
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(GemTargetRole::RuntimeHost);
        composer
            .add(
                LifecycleContribution {
                    ready: Arc::new(AtomicBool::new(true)),
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();
        let mut process = ProcessComposition::new(composer).unwrap();
        let lease = process.registry_lease().unwrap();

        let shutdown = process.shutdown_gate();
        shutdown.begin_shutdown().unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert!(matches!(
            process.cleanup(),
            Err(ProcessCompositionCleanupError::ActiveLeases { active: 1 })
        ));
        assert_eq!(finished.load(Ordering::SeqCst), 0);

        drop(lease);
        process.cleanup().unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }
}
