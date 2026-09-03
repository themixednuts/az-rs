//! Open-project progress taxonomy + aggregation.
//!
//! `az-work` owns the generic [`az_work::ProgressEvent`]/[`az_work::Fraction`]
//! carrier; this module owns the DAEMON concern: the [`OpenProjectPhase`]
//! taxonomy, the frozen aggregate weights, and the [`PhaseAggregator`] that
//! folds per-phase fractions into a single monotonic basis-point bar for the
//! wire.
//!
//! The open flow builds one root [`az_work::Progress`] and one `child()` per
//! phase. The daemon's RPC [`az_work::ProgressSink`] adapter maps each emitted
//! event's [`az_work::ProgressId`] back to its phase via the registry built
//! here, folds it into the aggregator, and forwards a wire
//! [`az_proto_daemon::ProjectOpenProgressEvent`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use az_proto_daemon::{
    OpenProjectPhase as WirePhase, ProjectBuildCommand,
    ProjectBuildProgressEvent as BuildWireEvent, ProjectOpenProgressEvent as OpenWireEvent,
};
use az_work::{
    Fraction, Progress, ProgressEvent, ProgressId, ProgressKind, ProgressSink, Reporter,
};

/// A single open-project phase, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpenProjectPhase {
    /// Resolve the project + plan its services.
    ResolvePlan,
    /// Build the service binaries (the heavyweight phase).
    Build,
    /// Launch + wait-for-readiness on the planned services.
    StartServices,
    /// Editor attaches to the running session (editor-side phase).
    Attach,
    /// Editor loads schema/asset catalogs (editor-side phase).
    LoadCatalogs,
}

impl OpenProjectPhase {
    /// All phases in execution order.
    pub const ALL: [Self; 5] = [
        Self::ResolvePlan,
        Self::Build,
        Self::StartServices,
        Self::Attach,
        Self::LoadCatalogs,
    ];

    /// Frozen aggregate weight in basis points. The five weights sum to 10000.
    #[must_use]
    pub const fn weight_bp(self) -> u32 {
        match self {
            Self::Build => 8_200,
            Self::StartServices => 800,
            Self::Attach => 400,
            // Resolve and catalog load are both short bookend phases.
            Self::ResolvePlan | Self::LoadCatalogs => 300,
        }
    }

    /// Map onto the transport enum.
    #[must_use]
    pub const fn to_wire(self) -> WirePhase {
        match self {
            Self::ResolvePlan => WirePhase::ResolvePlan,
            Self::Build => WirePhase::Build,
            Self::StartServices => WirePhase::StartServices,
            Self::Attach => WirePhase::Attach,
            Self::LoadCatalogs => WirePhase::LoadCatalogs,
        }
    }

    /// Map from the transport enum.
    #[must_use]
    pub const fn from_wire(phase: WirePhase) -> Self {
        match phase {
            WirePhase::ResolvePlan => Self::ResolvePlan,
            WirePhase::Build => Self::Build,
            WirePhase::StartServices => Self::StartServices,
            WirePhase::Attach => Self::Attach,
            WirePhase::LoadCatalogs => Self::LoadCatalogs,
        }
    }

    /// Stable label for human messages/logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ResolvePlan => "resolve",
            Self::Build => "build",
            Self::StartServices => "start services",
            Self::Attach => "attach",
            Self::LoadCatalogs => "load catalogs",
        }
    }
}

/// Per-phase progress state folded into a single aggregate bar.
///
/// Monotonicity is enforced two ways: each phase's completed-units clamp never
/// decreases, and the aggregate basis points are clamped to never regress.
#[derive(Debug)]
pub struct PhaseAggregator {
    /// Current fraction per phase (defaults to `Unknown{0}` == not started).
    phases: HashMap<OpenProjectPhase, Fraction>,
    /// Highest aggregate basis points emitted so far (monotone clamp).
    aggregate_floor: u32,
}

impl Default for PhaseAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl PhaseAggregator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            phases: HashMap::new(),
            aggregate_floor: 0,
        }
    }

    /// Record a phase's latest fraction, clamping any `Exact` regression in its
    /// completed-unit count. Returns the new aggregate basis points (monotone).
    pub fn update(&mut self, phase: OpenProjectPhase, fraction: Fraction) -> u32 {
        let clamped = match (self.phases.get(&phase).copied(), fraction) {
            // Never let an Exact phase's done-count slide backwards.
            (Some(Fraction::Exact { done: prev, .. }), Fraction::Exact { done, total }) => {
                Fraction::Exact {
                    done: done.max(prev),
                    total,
                }
            }
            _ => fraction,
        };
        self.phases.insert(phase, clamped);
        self.recompute()
    }

    /// Mark a phase fully complete (credits its entire weight immediately).
    pub fn complete(&mut self, phase: OpenProjectPhase) -> u32 {
        self.phases.insert(phase, Fraction::Complete);
        self.recompute()
    }

    fn recompute(&mut self) -> u32 {
        let mut total: u64 = 0;
        for phase in OpenProjectPhase::ALL {
            let fraction = self
                .phases
                .get(&phase)
                .copied()
                .unwrap_or(Fraction::Unknown { done: 0 });
            // Each phase contributes its own completion scaled by its weight.
            // Unknown contributes 0 so the aggregate stays honest while a
            // first-ever build streams live messages.
            let phase_bp = u64::from(fraction.to_basis_points());
            total += phase_bp * u64::from(phase.weight_bp()) / u64::from(Fraction::BASIS_POINTS);
        }
        let aggregate = u32::try_from(total).unwrap_or(Fraction::BASIS_POINTS);
        self.aggregate_floor = self.aggregate_floor.max(aggregate);
        self.aggregate_floor
    }
}

/// Maps live `az-work` progress node ids onto their open-project phase so the
/// RPC sink adapter can stamp the correct phase on each forwarded event.
#[derive(Debug, Default)]
pub struct PhaseRegistry {
    ids: HashMap<ProgressId, OpenProjectPhase>,
}

impl PhaseRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Associate a progress node id with a phase.
    pub fn register(&mut self, id: ProgressId, phase: OpenProjectPhase) {
        self.ids.insert(id, phase);
    }

    /// Look up the phase for a progress node id, if registered.
    #[must_use]
    pub fn phase_for(&self, id: ProgressId) -> Option<OpenProjectPhase> {
        self.ids.get(&id).copied()
    }
}

/// The daemon's open-project progress handle: one root [`Progress`] plus a
/// child node per phase, with each child's id registered so the RPC sink can
/// stamp the right phase on the wire.
///
/// Construct via [`OpenProgress::new`] with a [`Reporter`] backed by the RPC
/// sink.
pub struct OpenProgress {
    #[allow(dead_code)]
    root: Progress,
    phases: HashMap<OpenProjectPhase, Progress>,
}

impl OpenProgress {
    /// Build the phase tree under `reporter` and register each phase node id in
    /// `registry` so emitted events can be mapped back to their phase.
    #[must_use]
    pub fn new(reporter: &Reporter, registry: &Arc<Mutex<PhaseRegistry>>) -> Self {
        let root = reporter.root("open project");
        let mut phases = HashMap::new();
        // Create the child nodes WITHOUT holding the registry lock: `child()`
        // emits a `Started` event synchronously into the sink, which itself
        // locks the registry. Registering afterwards avoids a re-entrant
        // deadlock (the `Started` event is unregistered and so ignored).
        for phase in OpenProjectPhase::ALL {
            let node = root.child(phase.label());
            phases.insert(phase, node);
        }
        {
            let mut registry = registry.lock().expect("phase registry poisoned");
            for (phase, node) in &phases {
                registry.register(node.id(), *phase);
            }
        }
        Self { root, phases }
    }

    /// The progress node for `phase`.
    #[must_use]
    pub fn phase(&self, phase: OpenProjectPhase) -> &Progress {
        &self.phases[&phase]
    }
}

/// One build-command phase in a project build execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildCommandPhase {
    /// Zero-based index into the planned command list.
    pub command_index: u32,
    /// Total command count in the planned build.
    pub command_count: u32,
    /// Stable command target label for UI/status messages.
    pub target_name: String,
}

/// Map a daemon build plan's commands onto command progress phases.
#[must_use]
pub fn project_build_command_phases(
    commands: &[ProjectBuildCommand],
) -> Vec<ProjectBuildCommandPhase> {
    let command_count = u32::try_from(commands.len()).unwrap_or(u32::MAX);
    commands
        .iter()
        .enumerate()
        .map(|(index, command)| ProjectBuildCommandPhase {
            command_index: u32::try_from(index).unwrap_or(u32::MAX),
            command_count,
            target_name: command.target_name.clone(),
        })
        .collect()
}

/// Maps live `az-work` progress node ids onto project-build command phases.
#[derive(Debug, Default)]
pub struct ProjectBuildPhaseRegistry {
    ids: HashMap<ProgressId, ProjectBuildCommandPhase>,
}

impl ProjectBuildPhaseRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Associate a progress node id with a build-command phase.
    pub fn register(&mut self, id: ProgressId, phase: ProjectBuildCommandPhase) {
        self.ids.insert(id, phase);
    }

    /// Look up the build-command phase for a progress node id, if registered.
    #[must_use]
    pub fn phase_for(&self, id: ProgressId) -> Option<ProjectBuildCommandPhase> {
        self.ids.get(&id).cloned()
    }
}

/// The daemon's project-build progress handle: one root node plus one child per
/// planned command. Each child is registered so the RPC sink can stamp command
/// index/count metadata on the wire.
pub struct ProjectBuildProgress {
    #[allow(dead_code)]
    root: Progress,
    commands: Vec<Progress>,
}

impl ProjectBuildProgress {
    /// Build command progress nodes under `reporter` and register them.
    #[must_use]
    pub fn new(
        reporter: &Reporter,
        registry: &Arc<Mutex<ProjectBuildPhaseRegistry>>,
        commands: &[ProjectBuildCommand],
    ) -> Self {
        let root = reporter.root("project build");
        let phases = project_build_command_phases(commands);
        let mut command_nodes = Vec::with_capacity(phases.len());
        for phase in &phases {
            command_nodes.push(root.child(format!(
                "build {} ({}/{})",
                phase.target_name,
                phase.command_index.saturating_add(1),
                phase.command_count
            )));
        }
        {
            let mut registry = registry
                .lock()
                .expect("project build phase registry poisoned");
            for (phase, node) in phases.into_iter().zip(&command_nodes) {
                registry.register(node.id(), phase);
            }
        }
        Self {
            root,
            commands: command_nodes,
        }
    }

    /// The progress node for the command at `index`.
    #[must_use]
    pub fn command(&self, index: usize) -> Option<&Progress> {
        self.commands.get(index)
    }
}

/// Per-command progress folded into a single project-build aggregate bar.
#[derive(Debug, Default)]
struct ProjectBuildAggregator {
    phases: Vec<Fraction>,
    aggregate_floor: u32,
}

impl ProjectBuildAggregator {
    fn update(&mut self, phase: &ProjectBuildCommandPhase, fraction: Fraction) -> u32 {
        let Some(index) = usize::try_from(phase.command_index).ok() else {
            return self.aggregate_floor;
        };
        let Some(command_count) = usize::try_from(phase.command_count).ok() else {
            return self.aggregate_floor;
        };
        if command_count == 0 || index >= command_count {
            return self.aggregate_floor;
        }
        if self.phases.len() < command_count {
            self.phases
                .resize(command_count, Fraction::Unknown { done: 0 });
        }
        let clamped = match (self.phases.get(index).copied(), fraction) {
            (Some(Fraction::Exact { done: prev, .. }), Fraction::Exact { done, total }) => {
                Fraction::Exact {
                    done: done.max(prev),
                    total,
                }
            }
            _ => fraction,
        };
        self.phases[index] = clamped;
        self.recompute(command_count)
    }

    fn complete(&mut self, phase: &ProjectBuildCommandPhase) -> u32 {
        self.update(phase, Fraction::Complete)
    }

    fn recompute(&mut self, command_count: usize) -> u32 {
        let total: u64 = self
            .phases
            .iter()
            .take(command_count)
            .map(|fraction| u64::from(fraction.to_basis_points()))
            .sum::<u64>()
            / u64::try_from(command_count).unwrap_or(1);
        let aggregate = u32::try_from(total).unwrap_or(Fraction::BASIS_POINTS);
        self.aggregate_floor = self.aggregate_floor.max(aggregate);
        self.aggregate_floor
    }
}

/// Per-node accumulated `(done, total)` so the sink can reconstruct a
/// [`Fraction`] from the raw `SetTotal`/`Advance` event stream.
#[derive(Debug, Default, Clone, Copy)]
struct NodeProgress {
    done: u64,
    total: Option<u64>,
    finished: bool,
}

impl NodeProgress {
    fn fraction(self) -> Fraction {
        if self.finished {
            Fraction::Complete
        } else {
            self.total
                .map_or(Fraction::Unknown { done: self.done }, |total| {
                    Fraction::exact(self.done, total)
                })
        }
    }

    fn raw(self) -> (u64, u64) {
        if self.finished {
            return self.total.map_or((self.done, 0), |total| (total, total));
        }
        self.total
            .map_or((self.done, 0), |total| (self.done.min(total), total))
    }
}

/// An [`az_work::ProgressSink`] that folds the open-project phase tree into a
/// single monotone aggregate bar and forwards a wire
/// [`az_proto_daemon::ProjectOpenProgressEvent`] for each meaningful event.
///
/// The adapter is the single conversion point between az-work's domain-free
/// [`ProgressEvent`] and the daemon's typed wire form: it reconstructs the
/// per-phase [`Fraction`] from the raw `SetTotal`/`Advance`/`Finished` stream,
/// stamps the correct [`OpenProjectPhase`] via the [`PhaseRegistry`], clamps the
/// aggregate monotone in the [`PhaseAggregator`], and assigns a process-unique
/// monotone `seq`. The `forward` closure receives the finished wire event
/// (fire-and-forget over RPC in production; a recording buffer in tests).
pub struct CapnpProgressSink<F> {
    registry: Arc<Mutex<PhaseRegistry>>,
    aggregator: Mutex<PhaseAggregator>,
    nodes: Mutex<HashMap<ProgressId, NodeProgress>>,
    seq: AtomicU64,
    forward: F,
}

impl<F> CapnpProgressSink<F>
where
    F: Fn(OpenWireEvent) + Send + Sync + 'static,
{
    /// Build a sink sharing the phase `registry` populated by [`OpenProgress`].
    #[must_use]
    pub fn new(registry: Arc<Mutex<PhaseRegistry>>, forward: F) -> Self {
        Self {
            registry,
            aggregator: Mutex::new(PhaseAggregator::new()),
            nodes: Mutex::new(HashMap::new()),
            // Seqs start at 1 so the editor's `last_seq == 0` initial guard
            // never drops the first event.
            seq: AtomicU64::new(1),
            forward,
        }
    }
}

impl<F> ProgressSink for CapnpProgressSink<F>
where
    F: Fn(OpenWireEvent) + Send + Sync + 'static,
{
    fn event(&self, event: ProgressEvent) {
        // Only phase nodes (registered by OpenProgress) carry wire meaning; the
        // root node and any `Started` bookkeeping are ignored.
        let Some(phase) = self
            .registry
            .lock()
            .expect("phase registry poisoned")
            .phase_for(event.id)
        else {
            return;
        };

        let mut message: Option<String> = None;
        let (fraction, node_raw) = {
            let mut nodes = self.nodes.lock().expect("progress nodes poisoned");
            let node = nodes.entry(event.id).or_default();
            match event.kind {
                ProgressKind::Started => return,
                ProgressKind::SetTotal(total) => node.total = Some(total),
                ProgressKind::Advance(delta) => node.done = node.done.saturating_add(delta),
                ProgressKind::Message(msg) => message = Some(msg),
                ProgressKind::Finished => node.finished = true,
            }
            let node = *node;
            drop(nodes);
            (node.fraction(), node.raw())
        };

        let done_bp = {
            let mut aggregator = self.aggregator.lock().expect("aggregator poisoned");
            if matches!(fraction, Fraction::Complete) {
                aggregator.complete(phase)
            } else {
                aggregator.update(phase, fraction)
            }
        };

        let (phase_done, phase_total) = node_raw;
        let wire = OpenWireEvent {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            phase: phase.to_wire(),
            done_bp,
            phase_done,
            phase_total,
            message: message.unwrap_or_default(),
        };
        (self.forward)(wire);
    }
}

/// An [`az_work::ProgressSink`] that folds a project build's per-command phase
/// tree into a monotone aggregate bar and forwards
/// [`az_proto_daemon::ProjectBuildProgressEvent`] updates.
pub struct CapnpProjectBuildProgressSink<F> {
    registry: Arc<Mutex<ProjectBuildPhaseRegistry>>,
    aggregator: Mutex<ProjectBuildAggregator>,
    nodes: Mutex<HashMap<ProgressId, NodeProgress>>,
    seq: AtomicU64,
    forward: F,
}

impl<F> CapnpProjectBuildProgressSink<F>
where
    F: Fn(BuildWireEvent) + Send + Sync + 'static,
{
    /// Build a sink sharing the phase `registry` populated by
    /// [`ProjectBuildProgress`].
    #[must_use]
    pub fn new(registry: Arc<Mutex<ProjectBuildPhaseRegistry>>, forward: F) -> Self {
        Self {
            registry,
            aggregator: Mutex::new(ProjectBuildAggregator::default()),
            nodes: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
            forward,
        }
    }
}

impl<F> ProgressSink for CapnpProjectBuildProgressSink<F>
where
    F: Fn(BuildWireEvent) + Send + Sync + 'static,
{
    fn event(&self, event: ProgressEvent) {
        let Some(phase) = self
            .registry
            .lock()
            .expect("project build phase registry poisoned")
            .phase_for(event.id)
        else {
            return;
        };

        let mut message: Option<String> = None;
        let (fraction, node_raw) = {
            let mut nodes = self.nodes.lock().expect("progress nodes poisoned");
            let node = nodes.entry(event.id).or_default();
            match event.kind {
                ProgressKind::Started => return,
                ProgressKind::SetTotal(total) => node.total = Some(total),
                ProgressKind::Advance(delta) => node.done = node.done.saturating_add(delta),
                ProgressKind::Message(msg) => message = Some(msg),
                ProgressKind::Finished => node.finished = true,
            }
            let node = *node;
            drop(nodes);
            (node.fraction(), node.raw())
        };

        let done_bp = {
            let mut aggregator = self.aggregator.lock().expect("aggregator poisoned");
            if matches!(fraction, Fraction::Complete) {
                aggregator.complete(&phase)
            } else {
                aggregator.update(&phase, fraction)
            }
        };

        let (command_done, command_total) = node_raw;
        let wire = BuildWireEvent {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            command_index: phase.command_index,
            command_count: phase.command_count,
            target_name: phase.target_name,
            done_bp,
            command_done,
            command_total,
            message: message.unwrap_or_default(),
        };
        (self.forward)(wire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_full_scale() {
        let sum: u32 = OpenProjectPhase::ALL.iter().map(|p| p.weight_bp()).sum();
        assert_eq!(sum, Fraction::BASIS_POINTS);
    }

    #[test]
    fn aggregate_is_monotone_and_credits_completed_phases() {
        let mut agg = PhaseAggregator::new();

        // ResolvePlan completes -> full 300 bp credited at once.
        assert_eq!(agg.complete(OpenProjectPhase::ResolvePlan), 300);

        // Build climbs from half (8200 * 0.5 = 4100; + 300 = 4400).
        let half = agg.update(OpenProjectPhase::Build, Fraction::exact(1, 2));
        assert_eq!(half, 4_400);

        // A regressed Build fraction must not lower the aggregate.
        let regress = agg.update(OpenProjectPhase::Build, Fraction::exact(1, 4));
        assert_eq!(regress, 4_400, "aggregate must never regress");

        // Build completes -> full 8200 + 300 = 8500.
        assert_eq!(agg.complete(OpenProjectPhase::Build), 8_500);

        // Remaining phases finish to exactly full scale.
        agg.complete(OpenProjectPhase::StartServices);
        agg.complete(OpenProjectPhase::Attach);
        assert_eq!(
            agg.complete(OpenProjectPhase::LoadCatalogs),
            Fraction::BASIS_POINTS
        );
    }

    #[test]
    fn unknown_build_keeps_aggregate_at_resolve_weight() {
        let mut agg = PhaseAggregator::new();
        agg.complete(OpenProjectPhase::ResolvePlan);
        // First-ever build: Unknown contributes 0 weight.
        let bp = agg.update(OpenProjectPhase::Build, Fraction::Unknown { done: 5 });
        assert_eq!(bp, 300);
    }

    #[test]
    fn build_message_enters_build_phase_before_units_finish() {
        let registry = Arc::new(Mutex::new(PhaseRegistry::new()));
        let recorded: Arc<Mutex<Vec<OpenWireEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&recorded);
        let sink = Arc::new(CapnpProgressSink::new(
            Arc::clone(&registry),
            move |event| {
                captured.lock().unwrap().push(event);
            },
        ));
        let reporter = Reporter::new(sink);
        let progress = OpenProgress::new(&reporter, &registry);

        progress.phase(OpenProjectPhase::ResolvePlan).finish();
        progress
            .phase(OpenProjectPhase::Build)
            .message("building project services");

        let events = recorded.lock().unwrap().clone();
        let build = events
            .iter()
            .find(|event| event.phase == WirePhase::Build)
            .expect("build phase event");
        assert_eq!(build.done_bp, 300);
        assert_eq!(build.phase_done, 0);
        assert_eq!(build.phase_total, 0);
        assert_eq!(build.message, "building project services");
    }

    #[test]
    fn registry_maps_ids_to_phases() {
        use az_work::Reporter;
        let reporter = Reporter::noop();
        let root = reporter.root("open");
        let build = root.child("build");
        let mut registry = PhaseRegistry::new();
        registry.register(build.id(), OpenProjectPhase::Build);
        assert_eq!(
            registry.phase_for(build.id()),
            Some(OpenProjectPhase::Build)
        );
        assert_eq!(registry.phase_for(root.id()), None);
    }

    #[test]
    fn capnp_sink_forwards_ordered_monotone_events() {
        let registry = Arc::new(Mutex::new(PhaseRegistry::new()));
        let recorded: Arc<Mutex<Vec<OpenWireEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&recorded);
        let sink = Arc::new(CapnpProgressSink::new(
            Arc::clone(&registry),
            move |event| {
                captured.lock().unwrap().push(event);
            },
        ));
        let reporter = Reporter::new(sink);
        let progress = OpenProgress::new(&reporter, &registry);

        // Resolve completes instantly.
        progress.phase(OpenProjectPhase::ResolvePlan).finish();
        // Build streams a known total then half progress, then completes.
        let build = progress.phase(OpenProjectPhase::Build);
        build.set_total(4);
        build.advance(1);
        build.advance(1);
        build.finish();
        // Start services completes.
        progress.phase(OpenProjectPhase::StartServices).finish();

        let events = recorded.lock().unwrap().clone();
        assert!(!events.is_empty());

        // Sequence numbers are strictly increasing.
        for pair in events.windows(2) {
            assert!(pair[1].seq > pair[0].seq, "seq must be strictly increasing");
        }
        // Aggregate basis points never regress.
        for pair in events.windows(2) {
            assert!(
                pair[1].done_bp >= pair[0].done_bp,
                "aggregate basis points must be monotone"
            );
        }

        // First event is ResolvePlan completing -> 300 bp.
        assert_eq!(events[0].phase, WirePhase::ResolvePlan);
        assert_eq!(events[0].done_bp, 300);

        // The Build `set_total(4)` then two advances yield an exact 2/4 fraction
        // before the finish event credits the full Build weight.
        let build_finish = events
            .iter()
            .rev()
            .find(|event| event.phase == WirePhase::Build)
            .expect("build finish event");
        assert_eq!(build_finish.phase_done, 4);
        assert_eq!(build_finish.phase_total, 4);

        let last = events.last().unwrap();
        assert_eq!(last.phase, WirePhase::StartServices);
        // Resolve (300) + Build (8200) + StartServices (800) credited = 9300.
        assert_eq!(last.done_bp, 9_300);
    }

    #[test]
    fn project_build_commands_map_to_command_phases() {
        let project_root = std::env::temp_dir().join("azoth-daemon-progress-project");
        let gem_root = project_root.join("gems/physics");
        let commands = vec![
            ProjectBuildCommand {
                owner_id: "local.example".to_string(),
                owner_root: project_root.to_string_lossy().into_owned(),
                target_name: "game".to_string(),
                program: "cargo".to_string(),
                cwd: project_root.to_string_lossy().into_owned(),
                args: vec!["build".to_string(), "-p".to_string(), "game".to_string()],
                cargo_target_dir: None,
            },
            ProjectBuildCommand {
                owner_id: "physics".to_string(),
                owner_root: gem_root.to_string_lossy().into_owned(),
                target_name: "physics".to_string(),
                program: "cargo".to_string(),
                cwd: gem_root.to_string_lossy().into_owned(),
                args: vec!["build".to_string(), "-p".to_string(), "physics".to_string()],
                cargo_target_dir: None,
            },
        ];

        let phases = project_build_command_phases(&commands);

        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].command_index, 0);
        assert_eq!(phases[0].command_count, 2);
        assert_eq!(phases[0].target_name, "game");
        assert_eq!(phases[1].command_index, 1);
        assert_eq!(phases[1].command_count, 2);
        assert_eq!(phases[1].target_name, "physics");
    }

    #[test]
    fn project_build_aggregate_tracks_per_command_completion() {
        let first = ProjectBuildCommandPhase {
            command_index: 0,
            command_count: 2,
            target_name: "game".to_string(),
        };
        let second = ProjectBuildCommandPhase {
            command_index: 1,
            command_count: 2,
            target_name: "tool".to_string(),
        };
        let mut agg = ProjectBuildAggregator::default();

        assert_eq!(agg.update(&first, Fraction::exact(1, 2)), 2_500);
        assert_eq!(agg.complete(&first), 5_000);
        assert_eq!(agg.update(&second, Fraction::exact(1, 4)), 6_250);
        assert_eq!(agg.complete(&second), 10_000);
    }
}
