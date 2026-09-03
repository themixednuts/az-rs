//! Engine-owned visual graph geometry jobs.
//!
//! This crate is the geometry seam for the universal editor: it consumes the
//! durable `az-node-graph` model and returns semantic graph commands. It does
//! not own graph meaning, persistence, IPC, GPUI widgets, or runtime execution.

use std::collections::{BTreeMap, BTreeSet};

use az_node_graph::{
    GraphCommand, GraphCommentId, GraphConnection, GraphConnectionId, GraphConnectionRoute,
    GraphNode, GraphNodeId, GraphNodeLayout, GraphPoint, GraphPortRef, GraphRouteAnchor,
    GraphRouteAnchorId, GraphRouteAnchorKind, GraphRouteSegmentConstraint, GraphRouteStyle,
    NodePortAttachment, NodePortDescriptor, NodePortSide, NodeTypeCatalog, VisualGraphDocument,
};
use thiserror::Error;
use uuid::Uuid;

pub trait GraphLayoutSolver {
    /// Solves `request`'s layout operation and returns the resulting graph
    /// commands.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphLayoutError`] chosen by the implementation when the
    /// request cannot be solved — typically a node, port, or connection the
    /// document references but the geometry snapshot or node catalog does not
    /// describe.
    fn solve(&self, request: GraphLayoutRequest<'_>)
    -> Result<GraphLayoutResult, GraphLayoutError>;
}

#[derive(Debug, Clone, Copy)]
pub struct GraphLayoutRequest<'a> {
    pub document: &'a VisualGraphDocument,
    pub catalog: &'a NodeTypeCatalog,
    pub geometry: &'a GraphGeometrySnapshot,
    pub operation: GraphLayoutOperation,
}

impl<'a> GraphLayoutRequest<'a> {
    #[must_use]
    pub const fn new(
        document: &'a VisualGraphDocument,
        catalog: &'a NodeTypeCatalog,
        geometry: &'a GraphGeometrySnapshot,
        operation: GraphLayoutOperation,
    ) -> Self {
        Self {
            document,
            catalog,
            geometry,
            operation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLayoutOperation {
    AutoLayout(GraphAutoLayoutOptions),
    RouteConnections(GraphRouteOptions),
    RemoveOverlaps(GraphOverlapOptions),
    RefreshSpatialIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLayoutDirection {
    LeftToRight,
    TopToBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphAutoLayoutOptions {
    pub direction: GraphLayoutDirection,
    pub scope: GraphLayoutScope,
}

impl Default for GraphAutoLayoutOptions {
    fn default() -> Self {
        Self {
            direction: GraphLayoutDirection::LeftToRight,
            scope: GraphLayoutScope::WholeDocument,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphRouteOptions {
    pub scope: GraphLayoutScope,
}

impl Default for GraphRouteOptions {
    fn default() -> Self {
        Self {
            scope: GraphLayoutScope::WholeDocument,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphOverlapOptions {
    pub scope: GraphLayoutScope,
}

impl Default for GraphOverlapOptions {
    fn default() -> Self {
        Self {
            scope: GraphLayoutScope::WholeDocument,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLayoutScope {
    WholeDocument,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphLayoutResult {
    pub commands: Vec<GraphCommand>,
    pub spatial_index: GraphSpatialIndex,
    pub diagnostics: Vec<GraphLayoutDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLayoutDiagnostic {
    pub severity: GraphLayoutDiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLayoutDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphGeometrySnapshot {
    pub node_bounds: BTreeMap<GraphNodeId, GraphRect>,
}

impl GraphGeometrySnapshot {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            node_bounds: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_node_bounds(mut self, node_id: GraphNodeId, bounds: GraphRect) -> Self {
        self.node_bounds.insert(node_id, bounds);
        self
    }
}

impl Default for GraphGeometrySnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphLayoutTuning {
    pub default_node_size: GraphSize,
    pub layer_spacing: f32,
    pub row_spacing: f32,
    pub route_padding: f32,
    pub spatial_cell_size: f32,
}

impl Default for GraphLayoutTuning {
    fn default() -> Self {
        Self {
            default_node_size: GraphSize {
                width: 220.0,
                height: 96.0,
            },
            layer_spacing: 120.0,
            row_spacing: 48.0,
            route_padding: 24.0,
            spatial_cell_size: 256.0,
        }
    }
}

#[must_use]
pub fn graph_node_bounds(
    node: &GraphNode,
    geometry: &GraphGeometrySnapshot,
    tuning: GraphLayoutTuning,
) -> GraphRect {
    let size = geometry
        .node_bounds
        .get(&node.id)
        .map_or(tuning.default_node_size, |bounds| GraphSize {
            width: bounds.width,
            height: bounds.height,
        });
    GraphRect::from_origin_size(GraphPoint::new(node.layout.x, node.layout.y), size)
}

#[must_use]
pub fn graph_port_anchor(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
    geometry: &GraphGeometrySnapshot,
    tuning: GraphLayoutTuning,
    port_ref: &GraphPortRef,
) -> Option<GraphPoint> {
    let working = WorkingGeometry::from_snapshot(document, geometry, tuning);
    port_anchor(document, catalog, &working, port_ref)
}

#[must_use]
pub fn graph_connection_route_points(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
    geometry: &GraphGeometrySnapshot,
    tuning: GraphLayoutTuning,
    connection: &GraphConnection,
) -> Option<Vec<GraphPoint>> {
    let working = WorkingGeometry::from_snapshot(document, geometry, tuning);
    route_points(connection, document, catalog, &working)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl GraphRect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn from_origin_size(origin: GraphPoint, size: GraphSize) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub const fn center(self) -> GraphPoint {
        GraphPoint::new(
            self.width.mul_add(0.5, self.x),
            self.height.mul_add(0.5, self.y),
        )
    }

    #[must_use]
    pub fn expanded(self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            amount.mul_add(2.0, self.width),
            amount.mul_add(2.0, self.height),
        )
    }

    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    #[must_use]
    pub fn contains_point(self, point: GraphPoint) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    #[must_use]
    pub const fn with_origin(self, origin: GraphPoint) -> Self {
        Self::new(origin.x, origin.y, self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSpatialIndex {
    cell_size: f32,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
    entries: Vec<GraphSpatialEntry>,
}

impl GraphSpatialIndex {
    #[must_use]
    pub fn build(
        document: &VisualGraphDocument,
        catalog: &NodeTypeCatalog,
        geometry: &GraphGeometrySnapshot,
        tuning: GraphLayoutTuning,
    ) -> Self {
        let working = WorkingGeometry::from_snapshot(document, geometry, tuning);
        Self::build_from_working(document, catalog, &working, tuning)
    }

    #[must_use]
    pub fn entries(&self) -> &[GraphSpatialEntry] {
        &self.entries
    }

    #[must_use]
    pub fn query_rect(&self, query: GraphRect) -> Vec<&GraphSpatialEntry> {
        let mut seen = BTreeSet::new();
        let mut matches = Vec::new();
        for cell in cells_for_rect(query, self.cell_size) {
            if let Some(indices) = self.cells.get(&cell) {
                for index in indices {
                    if seen.insert(*index) {
                        let entry = &self.entries[*index];
                        if entry.bounds.intersects(query) {
                            matches.push(entry);
                        }
                    }
                }
            }
        }
        matches
    }

    fn build_from_working(
        document: &VisualGraphDocument,
        catalog: &NodeTypeCatalog,
        working: &WorkingGeometry,
        tuning: GraphLayoutTuning,
    ) -> Self {
        let mut index = Self {
            cell_size: tuning.spatial_cell_size.max(1.0),
            cells: BTreeMap::new(),
            entries: Vec::new(),
        };

        for node in &document.nodes {
            if let Some(bounds) = working.node_bounds.get(&node.id).copied() {
                index.insert(GraphSpatialEntry {
                    kind: GraphSpatialEntryKind::Node { node_id: node.id },
                    bounds,
                });
                if let Some(node_type) =
                    catalog.node_type_version(&node.node_type, node.node_type_version)
                {
                    for port in &node_type.ports {
                        let anchor = port_anchor_for_descriptor(
                            node,
                            node_type.ports.as_slice(),
                            port,
                            bounds,
                        );
                        index.insert(GraphSpatialEntry {
                            kind: GraphSpatialEntryKind::Port {
                                port: GraphPortRef::new(node.id, port.id),
                            },
                            bounds: GraphRect::new(anchor.x - 5.0, anchor.y - 5.0, 10.0, 10.0),
                        });
                    }
                }
            }
        }
        for comment in &document.comments {
            index.insert(GraphSpatialEntry {
                kind: GraphSpatialEntryKind::Comment {
                    comment_id: comment.id,
                },
                bounds: GraphRect::new(
                    comment.bounds.x,
                    comment.bounds.y,
                    comment.bounds.width,
                    comment.bounds.height,
                ),
            });
        }
        for connection in &document.connections {
            if let Some(points) = route_points(connection, document, catalog, working) {
                for (segment_index, window) in points.windows(2).enumerate() {
                    index.insert(GraphSpatialEntry {
                        kind: GraphSpatialEntryKind::ConnectionSegment {
                            connection_id: connection.id,
                            // A single connection's route never approaches
                            // u32::MAX segments; saturate rather than wrap if a
                            // malformed document somehow produces one.
                            segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                        },
                        bounds: segment_bounds(window[0], window[1]).expanded(2.0),
                    });
                }
            }
            for anchor in &connection.route.anchors {
                index.insert(GraphSpatialEntry {
                    kind: GraphSpatialEntryKind::RouteAnchor {
                        connection_id: connection.id,
                        anchor_id: anchor.id,
                    },
                    bounds: GraphRect::new(
                        anchor.position.x - 4.0,
                        anchor.position.y - 4.0,
                        8.0,
                        8.0,
                    ),
                });
            }
        }
        index
    }

    fn insert(&mut self, entry: GraphSpatialEntry) {
        let index = self.entries.len();
        for cell in cells_for_rect(entry.bounds, self.cell_size) {
            self.cells.entry(cell).or_default().push(index);
        }
        self.entries.push(entry);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSpatialEntry {
    pub kind: GraphSpatialEntryKind,
    pub bounds: GraphRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSpatialEntryKind {
    Node {
        node_id: GraphNodeId,
    },
    Port {
        port: GraphPortRef,
    },
    Comment {
        comment_id: GraphCommentId,
    },
    ConnectionSegment {
        connection_id: GraphConnectionId,
        segment_index: u32,
    },
    RouteAnchor {
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
    },
}

#[derive(Debug, Error)]
pub enum GraphLayoutError {
    #[error("graph document is invalid before layout: {0}")]
    InvalidDocument(#[from] az_node_graph::VisualGraphValidationError),
    #[error("layout tuning is invalid: {0}")]
    InvalidTuning(String),
    #[error("graph command produced by layout solver was invalid: {0}")]
    InvalidCommand(#[from] az_node_graph::GraphCommandApplyError),
}

#[derive(Debug, Clone)]
pub struct DefaultGraphLayoutSolver {
    tuning: GraphLayoutTuning,
}

impl DefaultGraphLayoutSolver {
    #[must_use]
    pub const fn new(tuning: GraphLayoutTuning) -> Self {
        Self { tuning }
    }

    #[must_use]
    pub const fn tuning(&self) -> GraphLayoutTuning {
        self.tuning
    }
}

impl Default for DefaultGraphLayoutSolver {
    fn default() -> Self {
        Self::new(GraphLayoutTuning::default())
    }
}

impl GraphLayoutSolver for DefaultGraphLayoutSolver {
    fn solve(
        &self,
        request: GraphLayoutRequest<'_>,
    ) -> Result<GraphLayoutResult, GraphLayoutError> {
        validate_tuning(self.tuning)?;
        request.document.validate_against(request.catalog)?;

        let mut working =
            WorkingGeometry::from_snapshot(request.document, request.geometry, self.tuning);
        let mut commands = Vec::new();
        let diagnostics = Vec::new();

        match request.operation {
            GraphLayoutOperation::AutoLayout(options) => {
                self.solve_auto_layout(request.document, options, &mut working, &mut commands);
                self.solve_routes(
                    request.document,
                    request.catalog,
                    GraphRouteOptions {
                        scope: options.scope,
                    },
                    &working,
                    &mut commands,
                );
            }
            GraphLayoutOperation::RouteConnections(options) => {
                self.solve_routes(
                    request.document,
                    request.catalog,
                    options,
                    &working,
                    &mut commands,
                );
            }
            GraphLayoutOperation::RemoveOverlaps(options) => {
                self.solve_overlap_removal(request.document, options, &mut working, &mut commands);
            }
            GraphLayoutOperation::RefreshSpatialIndex => {}
        }

        let mut projected = request.document.clone();
        if !commands.is_empty() {
            projected.apply_commands(commands.clone(), request.catalog)?;
        }
        let projected_working =
            WorkingGeometry::from_snapshot(&projected, request.geometry, self.tuning);
        let spatial_index = GraphSpatialIndex::build_from_working(
            &projected,
            request.catalog,
            &projected_working,
            self.tuning,
        );

        Ok(GraphLayoutResult {
            commands,
            spatial_index,
            diagnostics,
        })
    }
}

impl DefaultGraphLayoutSolver {
    fn solve_auto_layout(
        &self,
        document: &VisualGraphDocument,
        options: GraphAutoLayoutOptions,
        working: &mut WorkingGeometry,
        commands: &mut Vec<GraphCommand>,
    ) {
        let ranks = assign_layers(document);
        let mut layers = order_layers(document, &ranks, options, working);

        let mut primary = 0.0;
        for node_ids in layers.values_mut() {
            let max_primary_span = node_ids
                .iter()
                .filter_map(|node_id| working.node_bounds.get(node_id))
                .map(|bounds| match options.direction {
                    GraphLayoutDirection::LeftToRight => bounds.width,
                    GraphLayoutDirection::TopToBottom => bounds.height,
                })
                .fold(0.0, f32::max);
            let mut secondary = 0.0;

            for node_id in node_ids {
                let Some(current) = working.node_bounds.get(node_id).copied() else {
                    continue;
                };
                let next = match options.direction {
                    GraphLayoutDirection::LeftToRight => {
                        current.with_origin(GraphPoint::new(primary, secondary))
                    }
                    GraphLayoutDirection::TopToBottom => {
                        current.with_origin(GraphPoint::new(secondary, primary))
                    }
                };
                update_node_layout(document, working, commands, *node_id, next);
                secondary += match options.direction {
                    GraphLayoutDirection::LeftToRight => current.height + self.tuning.row_spacing,
                    GraphLayoutDirection::TopToBottom => current.width + self.tuning.row_spacing,
                };
            }

            primary += max_primary_span + self.tuning.layer_spacing;
        }
    }

    fn solve_routes(
        &self,
        document: &VisualGraphDocument,
        catalog: &NodeTypeCatalog,
        options: GraphRouteOptions,
        working: &WorkingGeometry,
        commands: &mut Vec<GraphCommand>,
    ) {
        for connection in &document.connections {
            if !options.scope.contains_connection(connection.id) {
                continue;
            }
            let obstacles =
                obstacles_for_connection(document, working, self.tuning.route_padding, connection);
            let Some(route) =
                self.route_connection(connection, document, catalog, working, &obstacles)
            else {
                continue;
            };
            if route != connection.route {
                commands.push(GraphCommand::SetConnectionRoute {
                    connection_id: connection.id,
                    route,
                });
            }
        }
    }

    fn route_connection(
        &self,
        connection: &GraphConnection,
        document: &VisualGraphDocument,
        catalog: &NodeTypeCatalog,
        working: &WorkingGeometry,
        obstacles: &[GraphRect],
    ) -> Option<GraphConnectionRoute> {
        let start = port_anchor(document, catalog, working, &connection.from)?;
        let end = port_anchor(document, catalog, working, &connection.to)?;
        let preserved = connection
            .route
            .anchors
            .iter()
            .filter(|anchor| {
                anchor.kind != GraphRouteAnchorKind::SolverWaypoint
                    || anchor.outgoing_segment == GraphRouteSegmentConstraint::Fixed
            })
            .cloned()
            .collect::<Vec<_>>();

        let mut anchors = Vec::new();
        let mut previous = start;
        let mut sequence = 0_u32;

        for checkpoint in preserved {
            append_solver_route_points(
                connection.id,
                &mut sequence,
                &route_span_orthogonal(
                    previous,
                    checkpoint.position,
                    obstacles,
                    self.tuning.route_padding,
                )?,
                self.tuning.route_padding,
                &mut anchors,
            );
            previous = checkpoint.position;
            push_route_anchor(&mut anchors, checkpoint);
        }
        append_solver_route_points(
            connection.id,
            &mut sequence,
            &route_span_orthogonal(previous, end, obstacles, self.tuning.route_padding)?,
            self.tuning.route_padding,
            &mut anchors,
        );

        Some(GraphConnectionRoute {
            style: GraphRouteStyle::Orthogonal,
            anchors,
        })
    }

    fn solve_overlap_removal(
        &self,
        document: &VisualGraphDocument,
        options: GraphOverlapOptions,
        working: &mut WorkingGeometry,
        commands: &mut Vec<GraphCommand>,
    ) {
        let mut node_ids = document
            .nodes
            .iter()
            .map(|node| node.id)
            .filter(|node_id| options.scope.contains_node(*node_id))
            .collect::<Vec<_>>();
        node_ids.sort_by(|left, right| {
            let left_bounds = working.node_bounds.get(left).copied().unwrap_or_default();
            let right_bounds = working.node_bounds.get(right).copied().unwrap_or_default();
            left_bounds
                .y
                .total_cmp(&right_bounds.y)
                .then_with(|| left_bounds.x.total_cmp(&right_bounds.x))
                .then_with(|| left.cmp(right))
        });

        for _ in 0..32 {
            let mut changed = false;
            for index in 0..node_ids.len() {
                for previous_index in 0..index {
                    let current_id = node_ids[index];
                    let previous_id = node_ids[previous_index];
                    changed |= separate_overlapping_pair(
                        document,
                        working,
                        commands,
                        current_id,
                        previous_id,
                        self.tuning.row_spacing,
                    );
                }
            }
            if !changed {
                break;
            }
        }
    }
}

impl GraphLayoutScope {
    #[must_use]
    pub const fn contains_node(self, _node_id: GraphNodeId) -> bool {
        match self {
            Self::WholeDocument => true,
        }
    }

    #[must_use]
    pub const fn contains_connection(self, _connection_id: GraphConnectionId) -> bool {
        match self {
            Self::WholeDocument => true,
        }
    }
}

#[derive(Debug, Clone)]
struct WorkingGeometry {
    node_bounds: BTreeMap<GraphNodeId, GraphRect>,
}

impl WorkingGeometry {
    fn from_snapshot(
        document: &VisualGraphDocument,
        snapshot: &GraphGeometrySnapshot,
        tuning: GraphLayoutTuning,
    ) -> Self {
        let mut node_bounds = BTreeMap::new();
        for node in &document.nodes {
            let size =
                snapshot
                    .node_bounds
                    .get(&node.id)
                    .map_or(tuning.default_node_size, |bounds| GraphSize {
                        width: bounds.width,
                        height: bounds.height,
                    });
            node_bounds.insert(
                node.id,
                GraphRect::from_origin_size(GraphPoint::new(node.layout.x, node.layout.y), size),
            );
        }
        Self { node_bounds }
    }
}

impl Default for GraphRect {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }
}

fn update_node_layout(
    document: &VisualGraphDocument,
    working: &mut WorkingGeometry,
    commands: &mut Vec<GraphCommand>,
    node_id: GraphNodeId,
    next: GraphRect,
) {
    let Some(current) = working.node_bounds.insert(node_id, next) else {
        return;
    };
    if points_close(
        GraphPoint::new(current.x, current.y),
        GraphPoint::new(next.x, next.y),
    ) {
        return;
    }
    if document.nodes.iter().any(|node| node.id == node_id) {
        commands.push(GraphCommand::MoveNode {
            node_id,
            layout: GraphNodeLayout {
                x: next.x,
                y: next.y,
            },
        });
    }
}

fn assign_layers(document: &VisualGraphDocument) -> BTreeMap<GraphNodeId, usize> {
    let mut ranks = document
        .nodes
        .iter()
        .map(|node| (node.id, 0_usize))
        .collect::<BTreeMap<_, _>>();

    for _ in 0..document.nodes.len() {
        let mut changed = false;
        for connection in &document.connections {
            let from_rank = *ranks.get(&connection.from.node_id).unwrap_or(&0);
            let to_rank = ranks.entry(connection.to.node_id).or_default();
            if *to_rank <= from_rank {
                *to_rank = from_rank + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ranks
}

fn order_layers(
    document: &VisualGraphDocument,
    ranks: &BTreeMap<GraphNodeId, usize>,
    options: GraphAutoLayoutOptions,
    working: &WorkingGeometry,
) -> BTreeMap<usize, Vec<GraphNodeId>> {
    let mut layers = BTreeMap::<usize, Vec<GraphNodeId>>::new();
    for node in &document.nodes {
        if options.scope.contains_node(node.id) {
            layers
                .entry(*ranks.get(&node.id).unwrap_or(&0))
                .or_default()
                .push(node.id);
        }
    }

    for node_ids in layers.values_mut() {
        node_ids.sort_by(|left, right| {
            let left_bounds = working.node_bounds.get(left).copied().unwrap_or_default();
            let right_bounds = working.node_bounds.get(right).copied().unwrap_or_default();
            secondary_axis(left_bounds, options.direction)
                .total_cmp(&secondary_axis(right_bounds, options.direction))
                .then_with(|| left.cmp(right))
        });
    }

    let rank_keys = layers.keys().copied().collect::<Vec<_>>();
    for _ in 0..4 {
        for window in rank_keys.windows(2) {
            reorder_layer_by_barycenter(document, ranks, &mut layers, window[1], window[0]);
        }
        for window in rank_keys.windows(2).rev() {
            reorder_layer_by_barycenter(document, ranks, &mut layers, window[0], window[1]);
        }
    }
    layers
}

#[allow(
    clippy::cast_precision_loss,
    reason = "Layer ordinals only ever feed a barycenter comparison. f32 holds               every integer below 2^24 exactly and one layer of an editor graph               never comes near that; a checked narrowing would saturate and               silently reorder the layer, which is strictly worse than rounding               that cannot occur."
)]
fn reorder_layer_by_barycenter(
    document: &VisualGraphDocument,
    ranks: &BTreeMap<GraphNodeId, usize>,
    layers: &mut BTreeMap<usize, Vec<GraphNodeId>>,
    layer_rank: usize,
    neighbor_rank: usize,
) {
    let Some(neighbor_positions) = layers.get(&neighbor_rank).map(|node_ids| {
        node_ids
            .iter()
            .enumerate()
            .map(|(index, node_id)| (*node_id, index as f32))
            .collect::<BTreeMap<_, _>>()
    }) else {
        return;
    };
    let Some(layer) = layers.get_mut(&layer_rank) else {
        return;
    };

    let mut stable_positions = BTreeMap::new();
    for (index, node_id) in layer.iter().enumerate() {
        stable_positions.insert(*node_id, index);
    }

    layer.sort_by(|left, right| {
        let left_key = barycenter_for_node(document, ranks, &neighbor_positions, *left);
        let right_key = barycenter_for_node(document, ranks, &neighbor_positions, *right);
        match (left_key, right_key) {
            (Some(left_key), Some(right_key)) => left_key
                .total_cmp(&right_key)
                .then_with(|| stable_positions[left].cmp(&stable_positions[right]))
                .then_with(|| left.cmp(right)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => stable_positions[left]
                .cmp(&stable_positions[right])
                .then_with(|| left.cmp(right)),
        }
    });
}

#[allow(
    clippy::cast_precision_loss,
    reason = "`count` is the number of cross-rank neighbours of one node,               bounded by the document's connection count and far below f32's               2^24 exact-integer limit."
)]
fn barycenter_for_node(
    document: &VisualGraphDocument,
    ranks: &BTreeMap<GraphNodeId, usize>,
    neighbor_positions: &BTreeMap<GraphNodeId, f32>,
    node_id: GraphNodeId,
) -> Option<f32> {
    let node_rank = *ranks.get(&node_id)?;
    let mut total = 0.0_f32;
    let mut count = 0_u32;
    for connection in &document.connections {
        let neighbor = if connection.from.node_id == node_id {
            connection.to.node_id
        } else if connection.to.node_id == node_id {
            connection.from.node_id
        } else {
            continue;
        };
        if ranks.get(&neighbor).copied() == Some(node_rank) {
            continue;
        }
        if let Some(position) = neighbor_positions.get(&neighbor) {
            total += *position;
            count += 1;
        }
    }
    (count > 0).then_some(total / count as f32)
}

const fn secondary_axis(bounds: GraphRect, direction: GraphLayoutDirection) -> f32 {
    match direction {
        GraphLayoutDirection::LeftToRight => bounds.y,
        GraphLayoutDirection::TopToBottom => bounds.x,
    }
}

fn separate_overlapping_pair(
    document: &VisualGraphDocument,
    working: &mut WorkingGeometry,
    commands: &mut Vec<GraphCommand>,
    current_id: GraphNodeId,
    previous_id: GraphNodeId,
    padding: f32,
) -> bool {
    let Some(current) = working.node_bounds.get(&current_id).copied() else {
        return false;
    };
    let Some(previous) = working.node_bounds.get(&previous_id).copied() else {
        return false;
    };
    if !current
        .expanded(padding * 0.5)
        .intersects(previous.expanded(padding * 0.5))
    {
        return false;
    }

    let current_center = current.center();
    let previous_center = previous.center();
    let overlap_x =
        current.right().min(previous.right()) - current.x.max(previous.x) + padding.max(1.0);
    let overlap_y =
        current.bottom().min(previous.bottom()) - current.y.max(previous.y) + padding.max(1.0);
    if overlap_x <= 0.0 || overlap_y <= 0.0 {
        return false;
    }

    let delta_x = (current_center.x - previous_center.x).abs();
    let delta_y = (current_center.y - previous_center.y).abs();
    let mut next = current;
    if delta_x > delta_y {
        let direction = if current_center.x >= previous_center.x {
            1.0
        } else {
            -1.0
        };
        next.x += overlap_x * direction;
    } else {
        let direction = if current_center.y >= previous_center.y {
            1.0
        } else {
            -1.0
        };
        next.y += overlap_y * direction;
    }
    update_node_layout(document, working, commands, current_id, next);
    true
}

fn append_solver_route_points(
    connection_id: GraphConnectionId,
    sequence: &mut u32,
    route: &[GraphPoint],
    _padding: f32,
    anchors: &mut Vec<GraphRouteAnchor>,
) {
    if route.len() < 3 {
        return;
    }
    for point in route.iter().copied().skip(1).take(route.len() - 2) {
        if !anchors
            .last()
            .is_some_and(|anchor| points_close(anchor.position, point))
        {
            *sequence += 1;
            push_route_anchor(
                anchors,
                GraphRouteAnchor {
                    id: deterministic_anchor_id(connection_id, *sequence),
                    position: point,
                    kind: GraphRouteAnchorKind::SolverWaypoint,
                    outgoing_segment: GraphRouteSegmentConstraint::Flexible,
                },
            );
        }
    }
}

fn push_route_anchor(anchors: &mut Vec<GraphRouteAnchor>, anchor: GraphRouteAnchor) {
    if anchors
        .last()
        .is_some_and(|previous| points_close(previous.position, anchor.position))
    {
        return;
    }
    anchors.push(anchor);
}

fn deterministic_anchor_id(connection_id: GraphConnectionId, sequence: u32) -> GraphRouteAnchorId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(connection_id.as_uuid().as_bytes());
    hasher.update(&sequence.to_le_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    GraphRouteAnchorId::new(Uuid::from_bytes(bytes))
}

fn port_anchor(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
    working: &WorkingGeometry,
    port_ref: &GraphPortRef,
) -> Option<GraphPoint> {
    let node = document
        .nodes
        .iter()
        .find(|node| node.id == port_ref.node_id)?;
    let node_type = catalog.node_type_version(&node.node_type, node.node_type_version)?;
    let port = node_type
        .ports
        .iter()
        .find(|port| port.id == port_ref.port_id)?;
    let bounds = working.node_bounds.get(&node.id).copied()?;
    Some(port_anchor_for_descriptor(
        node,
        node_type.ports.as_slice(),
        port,
        bounds,
    ))
}

#[allow(
    clippy::cast_precision_loss,
    reason = "Both casts are over the port count on one side of a single node,               used only to place the port a fraction of the way along that               side. The value is orders of magnitude below f32's 2^24               exact-integer limit, and a saturating narrowing would move the               anchor rather than merely round it."
)]
fn port_anchor_for_descriptor(
    _node: &GraphNode,
    ports: &[NodePortDescriptor],
    port: &NodePortDescriptor,
    bounds: GraphRect,
) -> GraphPoint {
    let mut side_ports = ports
        .iter()
        .filter(|candidate| candidate.layout.side == port.layout.side)
        .collect::<Vec<_>>();
    side_ports.sort_by(|left, right| {
        left.layout
            .order
            .unwrap_or(u32::MAX)
            .cmp(&right.layout.order.unwrap_or(u32::MAX))
            .then_with(|| left.id.cmp(&right.id))
    });
    let fraction = match port.layout.attachment {
        NodePortAttachment::EvenlySpaced => {
            let index = side_ports
                .iter()
                .position(|candidate| candidate.id == port.id)
                .unwrap_or(0) as f32;
            (index + 1.0) / (side_ports.len() as f32 + 1.0)
        }
        NodePortAttachment::FixedFraction { per_mille } => f32::from(per_mille.min(1000)) / 1000.0,
    };

    match port.layout.side {
        NodePortSide::North => GraphPoint::new(bounds.width.mul_add(fraction, bounds.x), bounds.y),
        NodePortSide::East => {
            GraphPoint::new(bounds.right(), bounds.height.mul_add(fraction, bounds.y))
        }
        NodePortSide::South => {
            GraphPoint::new(bounds.width.mul_add(fraction, bounds.x), bounds.bottom())
        }
        NodePortSide::West => GraphPoint::new(bounds.x, bounds.height.mul_add(fraction, bounds.y)),
    }
}

fn obstacles_for_connection(
    document: &VisualGraphDocument,
    working: &WorkingGeometry,
    padding: f32,
    connection: &GraphConnection,
) -> Vec<GraphRect> {
    let mut obstacles = working
        .node_bounds
        .iter()
        .filter(|(node_id, _)| {
            **node_id != connection.from.node_id && **node_id != connection.to.node_id
        })
        .map(|(_, bounds)| *bounds)
        .map(|bounds| bounds.expanded(padding))
        .collect::<Vec<_>>();
    obstacles.extend(document.comments.iter().map(|comment| {
        GraphRect::new(
            comment.bounds.x,
            comment.bounds.y,
            comment.bounds.width,
            comment.bounds.height,
        )
        .expanded(padding)
    }));
    obstacles
}

fn route_span_orthogonal(
    start: GraphPoint,
    end: GraphPoint,
    obstacles: &[GraphRect],
    padding: f32,
) -> Option<Vec<GraphPoint>> {
    if points_close(start, end) {
        return Some(vec![start, end]);
    }
    let active_obstacles = obstacles
        .iter()
        .copied()
        .filter(|obstacle| {
            !point_inside_obstacle(start, *obstacle) && !point_inside_obstacle(end, *obstacle)
        })
        .collect::<Vec<_>>();

    let (points, node_at) = route_candidate_grid(start, end, &active_obstacles, padding);

    let start_index = points
        .iter()
        .position(|point| points_close(*point, start))?;
    let end_index = points.iter().position(|point| points_close(*point, end))?;
    let adjacency = route_adjacency(&points, &node_at, &active_obstacles);
    let states = route_shortest_states(&adjacency, start_index, end_index, padding.max(1.0))?;

    let mut route = Vec::new();
    for state in states {
        let point = points[state / 3];
        if !route.last().is_some_and(|last| points_close(*last, point)) {
            route.push(point);
        }
    }
    Some(compress_collinear_route(route))
}

/// Builds the visibility-grid candidate points for an orthogonal route.
///
/// Candidate coordinates are the endpoints plus each obstacle's edges and their
/// padded offsets. Points that fall inside an obstacle are dropped unless they
/// are an endpoint. Returns the surviving points and a column-major
/// `node_at[x][y]` lookup from grid cell to point index.
fn route_candidate_grid(
    start: GraphPoint,
    end: GraphPoint,
    active_obstacles: &[GraphRect],
    padding: f32,
) -> (Vec<GraphPoint>, Vec<Vec<Option<usize>>>) {
    let mut xs = vec![start.x, end.x];
    let mut ys = vec![start.y, end.y];
    for obstacle in active_obstacles {
        xs.extend([
            obstacle.x - padding,
            obstacle.x,
            obstacle.right(),
            obstacle.right() + padding,
        ]);
        ys.extend([
            obstacle.y - padding,
            obstacle.y,
            obstacle.bottom(),
            obstacle.bottom() + padding,
        ]);
    }
    sort_dedup_floats(&mut xs);
    sort_dedup_floats(&mut ys);

    let mut points = Vec::new();
    let mut node_at = vec![vec![None; ys.len()]; xs.len()];
    for (x_index, x) in xs.iter().copied().enumerate() {
        for (y_index, y) in ys.iter().copied().enumerate() {
            let point = GraphPoint::new(x, y);
            if !points_close(point, start)
                && !points_close(point, end)
                && point_inside_any_obstacle(point, active_obstacles)
            {
                continue;
            }
            let point_index = points.len();
            points.push(point);
            node_at[x_index][y_index] = Some(point_index);
        }
    }

    (points, node_at)
}

/// Links each candidate point to its nearest unobstructed neighbour along each
/// axis, tagging horizontal edges with axis `1` and vertical edges with axis `2`.
fn route_adjacency(
    points: &[GraphPoint],
    node_at: &[Vec<Option<usize>>],
    active_obstacles: &[GraphRect],
) -> Vec<Vec<(usize, f32, usize)>> {
    let mut adjacency = vec![Vec::<(usize, f32, usize)>::new(); points.len()];
    let y_len = node_at.first().map_or(0, Vec::len);

    for y_index in 0..y_len {
        let mut previous = None;
        for column in node_at {
            let Some(current) = column[y_index] else {
                continue;
            };
            if let Some(previous_index) = previous
                && segment_clear(points[previous_index], points[current], active_obstacles)
            {
                insert_route_edge(&mut adjacency, points, previous_index, current, 1);
            }
            previous = Some(current);
        }
    }

    for column in node_at {
        let mut previous = None;
        for current in column.iter().copied().flatten() {
            if let Some(previous_index) = previous
                && segment_clear(points[previous_index], points[current], active_obstacles)
            {
                insert_route_edge(&mut adjacency, points, previous_index, current, 2);
            }
            previous = Some(current);
        }
    }

    adjacency
}

/// Runs Dijkstra over `(point, incoming axis)` states, charging `bend_penalty`
/// whenever the route changes axis, and returns the state path from `start_index`
/// to `end_index`.
///
/// Returns `None` when no finite-cost path reaches the end point.
fn route_shortest_states(
    adjacency: &[Vec<(usize, f32, usize)>],
    start_index: usize,
    end_index: usize,
    bend_penalty: f32,
) -> Option<Vec<usize>> {
    let state_count = adjacency.len() * 3;
    let mut dist = vec![f32::INFINITY; state_count];
    let mut previous_state = vec![None; state_count];
    let mut visited = vec![false; state_count];
    dist[start_index * 3] = 0.0;

    while let Some(state) = min_unvisited_state(&dist, &visited) {
        if state / 3 == end_index {
            break;
        }
        visited[state] = true;
        let point_index = state / 3;
        let incoming_axis = state % 3;
        for (next_index, length, axis) in &adjacency[point_index] {
            let bend_cost = if incoming_axis != 0 && incoming_axis != *axis {
                bend_penalty
            } else {
                0.0
            };
            let next_state = next_index * 3 + *axis;
            let next_dist = dist[state] + *length + bend_cost;
            if next_dist < dist[next_state] {
                dist[next_state] = next_dist;
                previous_state[next_state] = Some(state);
            }
        }
    }

    let end_state = [end_index * 3, end_index * 3 + 1, end_index * 3 + 2]
        .into_iter()
        .min_by(|left, right| dist[*left].total_cmp(&dist[*right]))?;
    if !dist[end_state].is_finite() {
        return None;
    }

    let mut states = Vec::new();
    let mut cursor = end_state;
    states.push(cursor);
    while cursor != start_index * 3 {
        cursor = previous_state[cursor]?;
        states.push(cursor);
    }
    states.reverse();
    Some(states)
}

fn insert_route_edge(
    adjacency: &mut [Vec<(usize, f32, usize)>],
    points: &[GraphPoint],
    left: usize,
    right: usize,
    axis: usize,
) {
    let length =
        (points[left].x - points[right].x).abs() + (points[left].y - points[right].y).abs();
    adjacency[left].push((right, length, axis));
    adjacency[right].push((left, length, axis));
}

fn sort_dedup_floats(values: &mut Vec<f32>) {
    values.retain(|value| value.is_finite());
    values.sort_by(f32::total_cmp);
    values.dedup_by(|left, right| (*left - *right).abs() < 0.001);
}

fn point_inside_any_obstacle(point: GraphPoint, obstacles: &[GraphRect]) -> bool {
    obstacles
        .iter()
        .any(|obstacle| point_inside_obstacle(point, *obstacle))
}

fn point_inside_obstacle(point: GraphPoint, obstacle: GraphRect) -> bool {
    point.x > obstacle.x
        && point.x < obstacle.right()
        && point.y > obstacle.y
        && point.y < obstacle.bottom()
}

fn segment_clear(start: GraphPoint, end: GraphPoint, obstacles: &[GraphRect]) -> bool {
    obstacles
        .iter()
        .all(|obstacle| !segment_intersects_obstacle(start, end, *obstacle))
}

fn segment_intersects_obstacle(start: GraphPoint, end: GraphPoint, obstacle: GraphRect) -> bool {
    if points_close(start, end) {
        return point_inside_obstacle(start, obstacle);
    }
    if (start.y - end.y).abs() < 0.001 {
        let x_min = start.x.min(end.x);
        let x_max = start.x.max(end.x);
        return start.y > obstacle.y
            && start.y < obstacle.bottom()
            && x_min < obstacle.right()
            && x_max > obstacle.x;
    }
    if (start.x - end.x).abs() < 0.001 {
        let y_min = start.y.min(end.y);
        let y_max = start.y.max(end.y);
        return start.x > obstacle.x
            && start.x < obstacle.right()
            && y_min < obstacle.bottom()
            && y_max > obstacle.y;
    }
    true
}

fn min_unvisited_state(dist: &[f32], visited: &[bool]) -> Option<usize> {
    dist.iter()
        .enumerate()
        .filter(|(index, value)| !visited[*index] && value.is_finite())
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| index)
}

fn compress_collinear_route(route: Vec<GraphPoint>) -> Vec<GraphPoint> {
    if route.len() <= 2 {
        return route;
    }
    let mut compressed = Vec::with_capacity(route.len());
    compressed.push(route[0]);
    for window in route.windows(3) {
        if !points_collinear(window[0], window[1], window[2]) {
            compressed.push(window[1]);
        }
    }
    compressed.push(*route.last().expect("route has at least two points"));
    compressed
}

fn points_collinear(a: GraphPoint, b: GraphPoint, c: GraphPoint) -> bool {
    ((a.x - b.x).abs() < 0.001 && (b.x - c.x).abs() < 0.001)
        || ((a.y - b.y).abs() < 0.001 && (b.y - c.y).abs() < 0.001)
}

fn route_points(
    connection: &GraphConnection,
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
    working: &WorkingGeometry,
) -> Option<Vec<GraphPoint>> {
    let mut points = Vec::new();
    points.push(port_anchor(document, catalog, working, &connection.from)?);
    points.extend(
        connection
            .route
            .anchors
            .iter()
            .map(|anchor| anchor.position),
    );
    points.push(port_anchor(document, catalog, working, &connection.to)?);
    Some(points)
}

fn segment_bounds(start: GraphPoint, end: GraphPoint) -> GraphRect {
    let x = start.x.min(end.x);
    let y = start.y.min(end.y);
    GraphRect::new(
        x,
        y,
        (start.x - end.x).abs().max(1.0),
        (start.y - end.y).abs().max(1.0),
    )
}

fn points_close(left: GraphPoint, right: GraphPoint) -> bool {
    (left.x - right.x).abs() < 0.001 && (left.y - right.y).abs() < 0.001
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Rust's float-to-int `as` saturates at the integer bounds and maps               NaN to 0, which is exactly the clamping a spatial-hash cell index               wants for an off-canvas or degenerate rect. `core` offers no               checked f32 -> i32 conversion, so clippy's try_from suggestion               has nothing to point at."
)]
fn cells_for_rect(rect: GraphRect, cell_size: f32) -> Vec<(i32, i32)> {
    let cell_size = cell_size.max(1.0);
    let min_x = (rect.x / cell_size).floor() as i32;
    let max_x = (rect.right() / cell_size).floor() as i32;
    let min_y = (rect.y / cell_size).floor() as i32;
    let max_y = (rect.bottom() / cell_size).floor() as i32;
    let mut cells = Vec::new();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            cells.push((x, y));
        }
    }
    cells
}

fn validate_tuning(tuning: GraphLayoutTuning) -> Result<(), GraphLayoutError> {
    let values = [
        ("default node width", tuning.default_node_size.width),
        ("default node height", tuning.default_node_size.height),
        ("layer spacing", tuning.layer_spacing),
        ("row spacing", tuning.row_spacing),
        ("route padding", tuning.route_padding),
        ("spatial cell size", tuning.spatial_cell_size),
    ];
    for (name, value) in values {
        if !value.is_finite() || value <= 0.0 {
            return Err(GraphLayoutError::InvalidTuning(format!(
                "{name} must be finite and greater than zero"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use az_node_graph::{
        GraphComment, GraphCommentBounds, GraphConnection, GraphNode, GraphPortRef,
        GraphRouteAnchor, NodePortCapacity, NodePortDescriptor, NodePortDirection, NodePortId,
        NodePortLayout, NodePortSide, NodePortValue, NodeTypeDescriptor,
    };

    use super::*;

    const FLOAT_SCHEMA: &str = "core.f32";

    #[test]
    fn auto_layout_orders_dataflow_nodes_by_rank() {
        let catalog = test_catalog();
        let mut source = GraphNode::new(node_id(1), "azoth.test.float", 1);
        source.layout = GraphNodeLayout { x: 40.0, y: 20.0 };
        let mut target = GraphNode::new(node_id(2), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 10.0, y: 20.0 };
        let document = document_with_nodes_and_connection(&source, &target);
        let solver = DefaultGraphLayoutSolver::default();

        let result = solver
            .solve(GraphLayoutRequest::new(
                &document,
                &catalog,
                &GraphGeometrySnapshot::default(),
                GraphLayoutOperation::AutoLayout(GraphAutoLayoutOptions::default()),
            ))
            .unwrap();

        let source_move = move_for(&result.commands, source.id).unwrap();
        let target_move = move_for(&result.commands, target.id).unwrap();
        assert!(target_move.x > source_move.x);
    }

    #[test]
    fn auto_layout_uses_barycentric_layer_ordering_to_reduce_crossings() {
        let catalog = test_catalog();
        let mut source_a = GraphNode::new(node_id(1), "azoth.test.float", 1);
        source_a.layout = GraphNodeLayout { x: 0.0, y: 0.0 };
        let mut source_b = GraphNode::new(node_id(2), "azoth.test.float", 1);
        source_b.layout = GraphNodeLayout { x: 0.0, y: 140.0 };
        let mut target_c = GraphNode::new(node_id(3), "azoth.test.float", 1);
        target_c.layout = GraphNodeLayout { x: 400.0, y: 0.0 };
        let mut target_d = GraphNode::new(node_id(4), "azoth.test.float", 1);
        target_d.layout = GraphNodeLayout { x: 400.0, y: 140.0 };
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![
                source_a.clone(),
                source_b.clone(),
                target_c.clone(),
                target_d.clone(),
            ],
            connections: vec![
                GraphConnection::new(
                    connection_id(1),
                    GraphPortRef::new(source_a.id, NodePortId::new(2)),
                    GraphPortRef::new(target_d.id, NodePortId::new(1)),
                ),
                GraphConnection::new(
                    connection_id(2),
                    GraphPortRef::new(source_b.id, NodePortId::new(2)),
                    GraphPortRef::new(target_c.id, NodePortId::new(1)),
                ),
            ],
            comments: Vec::new(),
        };

        let result = DefaultGraphLayoutSolver::default()
            .solve(GraphLayoutRequest::new(
                &document,
                &catalog,
                &GraphGeometrySnapshot::default(),
                GraphLayoutOperation::AutoLayout(GraphAutoLayoutOptions::default()),
            ))
            .unwrap();

        // Barycenter ordering should uncross the two connections: the target
        // wired to the upper source ends up above the one wired to the lower.
        let wired_to_upper = move_for(&result.commands, target_d.id).unwrap();
        let wired_to_lower = move_for(&result.commands, target_c.id).unwrap();
        assert!(wired_to_upper.y < wired_to_lower.y);
    }

    #[test]
    fn routing_preserves_user_waypoints_and_detours_around_obstacles() {
        let catalog = test_catalog();
        let mut source = GraphNode::new(node_id(1), "azoth.test.float", 1);
        source.layout = GraphNodeLayout { x: 0.0, y: 0.0 };
        let mut target = GraphNode::new(node_id(2), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 700.0, y: 0.0 };
        let mut obstacle = GraphNode::new(node_id(3), "azoth.test.float", 1);
        obstacle.layout = GraphNodeLayout { x: 320.0, y: -20.0 };
        let user_anchor =
            GraphRouteAnchor::user_waypoint(route_anchor_id(7), GraphPoint::new(250.0, 160.0));
        let connection = GraphConnection::new(
            connection_id(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        )
        .with_route(GraphConnectionRoute::orthogonal().with_anchor(user_anchor.clone()));
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source, target, obstacle],
            connections: vec![connection.clone()],
            comments: Vec::new(),
        };
        let solver = DefaultGraphLayoutSolver::default();

        let result = solver
            .solve(GraphLayoutRequest::new(
                &document,
                &catalog,
                &GraphGeometrySnapshot::default(),
                GraphLayoutOperation::RouteConnections(GraphRouteOptions::default()),
            ))
            .unwrap();

        let route = route_for(&result.commands, connection.id).unwrap();
        assert!(route.anchors.iter().any(|anchor| anchor == &user_anchor));
        assert!(route.anchors.iter().any(|anchor| {
            anchor.kind == GraphRouteAnchorKind::SolverWaypoint && anchor.position.x > 350.0
        }));
    }

    #[test]
    fn fixed_port_attachment_uses_descriptor_fraction() {
        let catalog = NodeTypeCatalog::new(
            1,
            100,
            vec![
                NodeTypeDescriptor::new("azoth.test.fixed-port", 1, "Fixed Port")
                    .with_port(
                        NodePortDescriptor::new(
                            NodePortId::new(1),
                            "in",
                            NodePortDirection::Input,
                            NodePortValue::Data {
                                schema_type: FLOAT_SCHEMA.to_string(),
                            },
                        )
                        .with_layout(
                            NodePortLayout::new(NodePortSide::West).with_fixed_fraction(250),
                        ),
                    )
                    .with_port(
                        NodePortDescriptor::new(
                            NodePortId::new(2),
                            "out",
                            NodePortDirection::Output,
                            NodePortValue::Data {
                                schema_type: FLOAT_SCHEMA.to_string(),
                            },
                        )
                        .with_layout(
                            NodePortLayout::new(NodePortSide::East).with_fixed_fraction(750),
                        )
                        .with_capacity(NodePortCapacity::Multiple),
                    ),
            ],
        );
        let node = GraphNode::new(node_id(1), "azoth.test.fixed-port", 1);
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![node],
            connections: Vec::new(),
            comments: Vec::new(),
        };
        let index = GraphSpatialIndex::build(
            &document,
            &catalog,
            &GraphGeometrySnapshot::default(),
            GraphLayoutTuning::default(),
        );

        let west_port_hits = index.query_rect(GraphRect::new(-5.0, 19.0, 10.0, 10.0));
        assert!(west_port_hits.iter().any(|entry| {
            entry.kind
                == GraphSpatialEntryKind::Port {
                    port: GraphPortRef::new(node_id(1), NodePortId::new(1)),
                }
        }));
        let east_port_hits = index.query_rect(GraphRect::new(215.0, 67.0, 10.0, 10.0));
        assert!(east_port_hits.iter().any(|entry| {
            entry.kind
                == GraphSpatialEntryKind::Port {
                    port: GraphPortRef::new(node_id(1), NodePortId::new(2)),
                }
        }));
    }

    #[test]
    fn overlap_cleanup_separates_nodes_without_changing_x() {
        let catalog = test_catalog();
        let mut a = GraphNode::new(node_id(1), "azoth.test.float", 1);
        a.layout = GraphNodeLayout { x: 10.0, y: 10.0 };
        let mut b = GraphNode::new(node_id(2), "azoth.test.float", 1);
        b.layout = GraphNodeLayout { x: 20.0, y: 20.0 };
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![a, b.clone()],
            connections: Vec::new(),
            comments: Vec::new(),
        };

        let result = DefaultGraphLayoutSolver::default()
            .solve(GraphLayoutRequest::new(
                &document,
                &catalog,
                &GraphGeometrySnapshot::default(),
                GraphLayoutOperation::RemoveOverlaps(GraphOverlapOptions::default()),
            ))
            .unwrap();

        let moved = move_for(&result.commands, b.id).unwrap();
        #[allow(
            clippy::float_cmp,
            reason = "Exact comparison is the point: overlap cleanup moves only                       y, so x must come back bit-identical to the 20.0 authored                       above, not merely close."
        )]
        {
            assert_eq!(moved.x, 20.0);
        }
        assert!(moved.y > 100.0);
    }

    #[test]
    fn spatial_index_queries_nodes_segments_and_route_anchors() {
        let catalog = test_catalog();
        let source = GraphNode::new(node_id(1), "azoth.test.float", 1);
        let mut target = GraphNode::new(node_id(2), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 500.0, y: 0.0 };
        let anchor =
            GraphRouteAnchor::user_waypoint(route_anchor_id(9), GraphPoint::new(250.0, 60.0));
        let connection = GraphConnection::new(
            connection_id(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        )
        .with_route(GraphConnectionRoute::orthogonal().with_anchor(anchor.clone()));
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target],
            connections: vec![connection.clone()],
            comments: vec![GraphComment {
                id: comment_id(1),
                text: "note".to_string(),
                bounds: GraphCommentBounds {
                    x: 25.0,
                    y: 200.0,
                    width: 120.0,
                    height: 80.0,
                },
            }],
        };

        let index = GraphSpatialIndex::build(
            &document,
            &catalog,
            &GraphGeometrySnapshot::default(),
            GraphLayoutTuning::default(),
        );

        let node_hits = index.query_rect(GraphRect::new(0.0, 0.0, 10.0, 10.0));
        assert!(
            node_hits
                .iter()
                .any(|entry| entry.kind == GraphSpatialEntryKind::Node { node_id: source.id })
        );
        let port_hits = index.query_rect(GraphRect::new(-5.0, 43.0, 10.0, 10.0));
        assert!(port_hits.iter().any(|entry| {
            entry.kind
                == GraphSpatialEntryKind::Port {
                    port: GraphPortRef::new(source.id, NodePortId::new(1)),
                }
        }));
        let anchor_hits = index.query_rect(GraphRect::new(248.0, 58.0, 4.0, 4.0));
        assert!(anchor_hits.iter().any(|entry| {
            entry.kind
                == GraphSpatialEntryKind::RouteAnchor {
                    connection_id: connection.id,
                    anchor_id: anchor.id,
                }
        }));
        assert!(
            index
                .entries()
                .iter()
                .any(|entry| matches!(entry.kind, GraphSpatialEntryKind::ConnectionSegment { .. }))
        );
    }

    fn test_catalog() -> NodeTypeCatalog {
        NodeTypeCatalog::new(
            1,
            100,
            vec![
                NodeTypeDescriptor::new("azoth.test.float", 1, "Float")
                    .with_port(NodePortDescriptor::new(
                        NodePortId::new(1),
                        "in",
                        NodePortDirection::Input,
                        NodePortValue::Data {
                            schema_type: FLOAT_SCHEMA.to_string(),
                        },
                    ))
                    .with_port(
                        NodePortDescriptor::new(
                            NodePortId::new(2),
                            "out",
                            NodePortDirection::Output,
                            NodePortValue::Data {
                                schema_type: FLOAT_SCHEMA.to_string(),
                            },
                        )
                        .with_capacity(NodePortCapacity::Multiple),
                    ),
            ],
        )
    }

    fn document_with_nodes_and_connection(
        source: &GraphNode,
        target: &GraphNode,
    ) -> VisualGraphDocument {
        VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target.clone()],
            connections: vec![GraphConnection::new(
                connection_id(1),
                GraphPortRef::new(source.id, NodePortId::new(2)),
                GraphPortRef::new(target.id, NodePortId::new(1)),
            )],
            comments: Vec::new(),
        }
    }

    fn move_for(commands: &[GraphCommand], node_id: GraphNodeId) -> Option<GraphNodeLayout> {
        commands.iter().find_map(|command| match command {
            GraphCommand::MoveNode {
                node_id: moved,
                layout,
            } if *moved == node_id => Some(*layout),
            _ => None,
        })
    }

    fn route_for(
        commands: &[GraphCommand],
        connection_id: GraphConnectionId,
    ) -> Option<&GraphConnectionRoute> {
        commands.iter().find_map(|command| match command {
            GraphCommand::SetConnectionRoute {
                connection_id: routed,
                route,
            } if *routed == connection_id => Some(route),
            _ => None,
        })
    }

    fn node_id(value: u128) -> GraphNodeId {
        GraphNodeId::new(Uuid::from_u128(value))
    }

    fn connection_id(value: u128) -> GraphConnectionId {
        GraphConnectionId::new(Uuid::from_u128(value))
    }

    fn route_anchor_id(value: u128) -> GraphRouteAnchorId {
        GraphRouteAnchorId::new(Uuid::from_u128(value))
    }

    fn comment_id(value: u128) -> GraphCommentId {
        GraphCommentId::new(Uuid::from_u128(value))
    }
}
