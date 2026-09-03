use az_core::ReflectedValueEnvelope;
use serde::{Deserialize, Serialize};

use crate::MATERIAL_GRAPH_EXTENSION;

/// Editable material graph source.
///
/// The graph document is authored data. Asset builders lower it into shader and
/// material products; runtime code does not interpret this graph directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphSource {
    pub name: String,

    pub description: String,

    pub graph_type: String,

    pub required_catalog_hash: Option<Vec<u8>>,

    pub nodes: Vec<MaterialGraphNodeSource>,

    pub connections: Vec<MaterialGraphConnectionSource>,

    pub comments: Vec<MaterialGraphCommentSource>,
}

impl Default for MaterialGraphSource {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            graph_type: "azoth.material.graph".to_string(),
            required_catalog_hash: None,
            nodes: Vec::new(),
            connections: Vec::new(),
            comments: Vec::new(),
        }
    }
}

/// One node instance in a material graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphNodeSource {
    pub id: String,

    pub node_type: String,

    pub node_type_version: u32,

    pub input_values: Vec<MaterialGraphInputValueSource>,

    pub layout: MaterialGraphNodeLayoutSource,
}

/// One node input override in a material graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialGraphInputValueSource {
    pub port_id: u32,

    pub value: ReflectedValueEnvelope,
}

/// Canvas position for a material graph node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphNodeLayoutSource {
    pub x: f32,

    pub y: f32,
}

impl Default for MaterialGraphNodeLayoutSource {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

/// One edge between two material graph node ports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphConnectionSource {
    pub id: String,

    pub from: MaterialGraphPortRefSource,

    pub to: MaterialGraphPortRefSource,

    pub route: MaterialGraphConnectionRouteSource,
}

/// Reference to a node port in a material graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialGraphPortRefSource {
    pub node_id: String,

    pub port_id: u32,
}

/// Editable route data for a material graph connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphConnectionRouteSource {
    pub style: MaterialGraphRouteStyleSource,

    pub anchors: Vec<MaterialGraphRouteAnchorSource>,
}

impl Default for MaterialGraphConnectionRouteSource {
    fn default() -> Self {
        Self {
            style: MaterialGraphRouteStyleSource::Orthogonal,
            anchors: Vec::new(),
        }
    }
}

/// Connection routing style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialGraphRouteStyleSource {
    Orthogonal,
    Polyline,
    Spline,
}

/// One connection route anchor on the graph canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphRouteAnchorSource {
    pub id: String,

    pub position: MaterialGraphPointSource,

    pub kind: MaterialGraphRouteAnchorKindSource,
}

/// Route anchor kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialGraphRouteAnchorKindSource {
    UserWaypoint,
    SolverWaypoint,
    Junction,
}

/// Canvas-space point in a material graph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphPointSource {
    pub x: f32,

    pub y: f32,
}

/// Comment annotation in a material graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraphCommentSource {
    pub id: String,

    pub text: String,

    pub position: MaterialGraphPointSource,

    pub size: MaterialGraphPointSource,
}

#[must_use]
pub const fn material_graph_extension() -> &'static str {
    MATERIAL_GRAPH_EXTENSION
}
